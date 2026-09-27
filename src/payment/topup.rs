// src/payment/topup.rs
//
// Crediting a Stripe top-up. The one place that turns a PaymentIntent into
// credits, used by the browser's confirm call (HTTP and gRPC) and by the
// payment_intent.succeeded webhook alike.
//
// Everything comes from the intent as Stripe returns it, never from the caller:
//   - how many credits: the amount Stripe charged, 1 credit per currency unit
//   - whose account:    the email the intent was created for (metadata)
//   - whether at all:   status Succeeded, purpose "api0_credits"
// and each intent is credited once, enforced by a unique index on
// credit_transactions.stripe_payment_intent_id.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use stripe::{PaymentIntent, PaymentIntentStatus};

/// Metadata `purpose` on every intent created for an api0 top-up. The Stripe
/// account also takes payments for other things (desktop app licences, other
/// sites); an intent without this purpose is never turned into credits.
pub const PURPOSE_CREDITS: &str = "api0_credits";

/// Smallest and largest top-up, in the currency's minor unit (cents).
pub const MIN_AMOUNT: i64 = 100;
pub const MAX_AMOUNT: i64 = 1_000_000;

#[derive(Debug, PartialEq)]
pub enum TopUpError {
    NotSucceeded(String),
    NotATopUp,
    NoEmail,
    WrongAccount,
    Store(String),
}

impl std::fmt::Display for TopUpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotSucceeded(status) => write!(f, "Payment not completed. Status: {}", status),
            Self::NotATopUp => write!(f, "This payment is not a credit top-up"),
            Self::NoEmail => write!(f, "This payment is not linked to an account"),
            Self::WrongAccount => write!(f, "This payment belongs to another account"),
            Self::Store(e) => write!(f, "Credits could not be added: {}", e),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum TopUp {
    /// Credits were added now.
    Credited { credits: i64, new_balance: i64 },
    /// This intent had been credited before; nothing changed.
    AlreadyCredited { balance: i64 },
}

/// What a succeeded intent is worth: its email and credits. Pure, so the rules
/// can be tested without Stripe or a database.
pub fn assess(intent: &PaymentIntent) -> Result<(String, i64), TopUpError> {
    if intent.status != PaymentIntentStatus::Succeeded {
        return Err(TopUpError::NotSucceeded(format!("{:?}", intent.status)));
    }
    if intent.metadata.get("purpose").map(String::as_str) != Some(PURPOSE_CREDITS) {
        return Err(TopUpError::NotATopUp);
    }
    let email = intent
        .metadata
        .get("email")
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .ok_or(TopUpError::NoEmail)?;
    // amount_received is what was actually captured; for the automatic-capture
    // intents api0 creates it equals amount once the intent has succeeded.
    let credits = intent.amount_received / 100;
    Ok((email, credits))
}

/// Credits `intent` to its account. When `caller_email` is given (the browser
/// confirm path) the intent must have been created for that account.
pub async fn credit_intent(
    store: &EndpointStore,
    intent: &PaymentIntent,
    caller_email: Option<&str>,
) -> Result<(String, TopUp), TopUpError> {
    let (email, credits) = assess(intent)?;
    if let Some(caller) = caller_email {
        if caller.trim().to_lowercase() != email {
            app_log!(warn,
                payment_intent_id = %intent.id,
                caller = %caller,
                "Confirm call for an intent created for another account"
            );
            return Err(TopUpError::WrongAccount);
        }
    }

    let tenant = crate::endpoint_store::tenant_management::get_default_tenant(store, &email)
        .await
        .map_err(|e| TopUpError::Store(e.to_string()))?;
    let intent_id = intent.id.to_string();
    let description = format!(
        "Stripe payment – {} {:.2}",
        intent.currency.to_string().to_uppercase(),
        intent.amount_received as f64 / 100.0
    );

    let outcome = record(store, &tenant.id, &email, credits, &intent_id, &description)
        .await
        .map_err(|e| TopUpError::Store(e.to_string()))?;

    match &outcome {
        TopUp::Credited { credits, new_balance } => app_log!(info,
            email = %email, payment_intent_id = %intent_id, credits = credits, new_balance = new_balance,
            "Credits added for Stripe payment"
        ),
        TopUp::AlreadyCredited { .. } => app_log!(info,
            email = %email, payment_intent_id = %intent_id,
            "Stripe payment already credited; nothing to do"
        ),
    }
    Ok((email, outcome))
}

/// The ledger row and the balance change, in one transaction. The ledger row
/// goes first: if the unique index says this intent is already there, nothing
/// else happens.
async fn record(
    store: &EndpointStore,
    tenant_id: &str,
    email: &str,
    credits: i64,
    intent_id: &str,
    description: &str,
) -> Result<TopUp, StoreError> {
    let mut client = store.get_admin_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let inserted = tx
        .query_opt(
            "INSERT INTO credit_transactions \
             (tenant_id, email, amount, balance_after, action_type, description, stripe_payment_intent_id) \
             VALUES ($1, $2, $3, 0, 'stripe_topup', $4, $5) \
             ON CONFLICT (stripe_payment_intent_id) WHERE stripe_payment_intent_id IS NOT NULL DO NOTHING \
             RETURNING id",
            &[&tenant_id, &email, &credits, &description, &intent_id],
        )
        .await
        .to_store_error()?;

    let Some(row) = inserted else {
        let balance: i64 = tx
            .query_one("SELECT credit_balance FROM tenants WHERE id = $1", &[&tenant_id])
            .await
            .to_store_error()?
            .get(0);
        return Ok(TopUp::AlreadyCredited { balance });
    };
    let tx_id: i64 = row.get(0);

    let new_balance: i64 = tx
        .query_one(
            "UPDATE tenants SET credit_balance = credit_balance + $1 WHERE id = $2 RETURNING credit_balance",
            &[&credits, &tenant_id],
        )
        .await
        .to_store_error()?
        .get(0);
    tx.execute(
        "UPDATE credit_transactions SET balance_after = $1 WHERE id = $2",
        &[&new_balance, &tx_id],
    )
    .await
    .to_store_error()?;

    tx.commit().await.to_store_error()?;
    Ok(TopUp::Credited { credits, new_balance })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn intent(status: PaymentIntentStatus, amount: i64, meta: &[(&str, &str)]) -> PaymentIntent {
        PaymentIntent {
            status,
            amount,
            amount_received: amount,
            metadata: meta.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>(),
            ..Default::default()
        }
    }

    const TOPUP: [(&str, &str); 2] = [("purpose", PURPOSE_CREDITS), ("email", "Anna@Band.ch")];

    #[test]
    fn credits_are_what_stripe_charged_for_the_intents_own_account() {
        let i = intent(PaymentIntentStatus::Succeeded, 2500, &TOPUP);
        assert_eq!(assess(&i), Ok(("anna@band.ch".to_string(), 25)));
    }

    #[test]
    fn nothing_is_credited_before_the_payment_succeeds() {
        let i = intent(PaymentIntentStatus::Processing, 2500, &TOPUP);
        assert!(matches!(assess(&i), Err(TopUpError::NotSucceeded(_))));
    }

    #[test]
    fn other_payments_on_the_account_are_not_credits() {
        // A licence purchase, and a payment from another site with only an email.
        let licence = intent(PaymentIntentStatus::Succeeded, 3900, &[("purpose", "license")]);
        let other = intent(PaymentIntentStatus::Succeeded, 3900, &[("email", "anna@band.ch")]);
        assert_eq!(assess(&licence), Err(TopUpError::NotATopUp));
        assert_eq!(assess(&other), Err(TopUpError::NotATopUp));
    }

    #[test]
    fn a_top_up_without_an_email_is_refused() {
        let i = intent(PaymentIntentStatus::Succeeded, 2500, &[("purpose", PURPOSE_CREDITS)]);
        assert_eq!(assess(&i), Err(TopUpError::NoEmail));
    }
}

/// Against a real database: `TEST_DATABASE_URL=postgres://… cargo test -- --ignored topup_db`
#[cfg(test)]
mod topup_db {
    use super::*;

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL"]
    async fn an_intent_is_credited_once_however_often_it_is_reported() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = EndpointStore::new(&url).await.expect("store");
        // The schema is applied on every start: a second start must not fail.
        EndpointStore::new(&url).await.expect("schema re-applied");

        let email = format!("topup_{}@example.com", uuid::Uuid::new_v4().simple());
        let tenant = crate::endpoint_store::tenant_management::get_default_tenant(&store, &email)
            .await
            .expect("tenant");
        let before = store.get_credit_balance(&tenant.id).await.unwrap();
        let intent_id = format!("pi_test_{}", uuid::Uuid::new_v4().simple());

        let first = record(&store, &tenant.id, &email, 25, &intent_id, "test").await.unwrap();
        assert_eq!(first, TopUp::Credited { credits: 25, new_balance: before + 25 });
        let again = record(&store, &tenant.id, &email, 25, &intent_id, "test").await.unwrap();
        assert_eq!(again, TopUp::AlreadyCredited { balance: before + 25 });

        // Reported concurrently (webhook and browser at the same moment).
        let other = format!("pi_test_{}", uuid::Uuid::new_v4().simple());
        let (a, b) = tokio::join!(
            record(&store, &tenant.id, &email, 10, &other, "test"),
            record(&store, &tenant.id, &email, 10, &other, "test"),
        );
        let credited = [a.unwrap(), b.unwrap()]
            .iter()
            .filter(|o| matches!(o, TopUp::Credited { .. }))
            .count();
        assert_eq!(credited, 1);
        assert_eq!(store.get_credit_balance(&tenant.id).await.unwrap(), before + 35);
    }
}
