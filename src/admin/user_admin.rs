// src/admin/user_admin.rs
//
// The super admin's changes to one person in the user directory
// (admin/users_directory.rs).
//
//   POST /api/admin/licences/plan   { subscription_id, plan_id, gift? }
//        Moves a licence to another plan. A Stripe subscription switches price
//        as the customer portal would (prorated: the difference is billed or
//        credited on the next invoice) and is synced back from Stripe; a free
//        or hand-granted licence changes in the database only.
//        gift: true upgrades a Stripe subscription without changing what the
//        customer pays: the dearer plan's price, nothing prorated, and a coupon
//        taking the difference off every invoice for as long as the gift lasts.
//        Any later move without gift ends it (the coupon comes off, nothing
//        prorated, since the month was paid at the gifted price). A gift is
//        announced to the customer by email (EmailKind::PlanGifted).
//   POST /api/admin/users/delete    { email, confirm_email }
//        Deletes a person: their account, memberships, keys, the workspaces
//        only they belong to, and their licences. Refused while they pay
//        through Stripe (cancel first) or are the only owner of a workspace
//        others use (hand it over first).
//
// Both need X-Internal-Secret; the gateway restricts them to the super admin.

use crate::admin::tenant_overview::cascade_steps;
use crate::app_log;
use crate::email::{send_async, EmailKind, Lang};
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::plans;
use crate::payment::service::PaymentService;
use crate::payment::subscription::sync_subscription;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

fn fail(status: actix_web::http::StatusCode, error: &str) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({ "success": false, "error": error }))
}

fn bad_request(error: &str) -> HttpResponse {
    fail(actix_web::http::StatusCode::BAD_REQUEST, error)
}

fn server_error(error: &str) -> HttpResponse {
    fail(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, error)
}

#[derive(Debug, Deserialize)]
pub struct ChangePlanBody {
    pub subscription_id: String,
    pub plan_id: String,
    /// Give the plan, keeping what the customer pays (Stripe subscriptions only).
    #[serde(default)]
    pub gift: bool,
}

/// What a gift takes off each invoice so the customer goes on paying for
/// `paid` while on `to`, in cents; an error when `to` is not an upgrade.
fn gift_amount_off(paid: &plans::Plan, to: &plans::Plan) -> Result<i64, String> {
    if !paid.currency.eq_ignore_ascii_case(&to.currency) {
        return Err(format!("{} and {} are in different currencies: a gift needs the same", paid.name, to.name));
    }
    let off = to.price_cents - paid.price_cents;
    if off <= 0 {
        return Err(format!("A gift is an upgrade: pick a plan dearer than {}, which they pay for", paid.name));
    }
    Ok(off)
}

pub async fn change_licence_plan(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    stripe: web::Data<Arc<PaymentService>>,
    body: web::Json<ChangePlanBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return server_error("Database unavailable"),
    };
    let row = match client
        .query_opt(
            "SELECT email, plan_id, stripe_customer_id, is_free, gift_paid_plan_id, locale FROM subscriptions \
              WHERE id = $1 AND licence IS NOT NULL",
            &[&body.subscription_id],
        )
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return fail(actix_web::http::StatusCode::NOT_FOUND, "No such licence"),
        Err(e) => {
            app_log!(error, error = %e, "Licence lookup failed");
            return server_error("Query failed");
        }
    };
    let email: String = row.get(0);
    let from: String = row.get(1);
    let on_stripe = !row.get::<_, String>(2).is_empty();
    let is_free: bool = row.get(3);
    // The plan they pay for while a gift lasts.
    let gifted_from: Option<String> = row.get(4);
    let lang = Lang::from_tag(row.get::<_, Option<String>>(5).as_deref());
    // Set once a gift is given: the plan they go on paying for, for the email.
    let mut gift_paid_name: Option<String> = None;
    if from == body.plan_id {
        return bad_request("The licence is already on that plan");
    }
    if body.gift && !on_stripe {
        return bad_request("Only a paid subscription can be a gift: moving this licence costs them nothing already");
    }
    let plan = match plans::get_plan(&store, &body.plan_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return bad_request("No such plan"),
        Err(e) => {
            app_log!(error, error = %e, "Plan lookup failed");
            return server_error("Query failed");
        }
    };

    if on_stripe {
        if plan.is_free() {
            return bad_request("A paid subscription can't move to a free plan: cancel it instead");
        }
        let Some(price) = plan.stripe_price_id.as_deref() else {
            return bad_request("That plan has no Stripe price yet: put it on sale in Plans first");
        };
        let id = body.subscription_id.as_str();
        let changed = if body.gift {
            let paid_id = gifted_from.clone().unwrap_or_else(|| from.clone());
            let paid = match plans::get_plan(&store, &paid_id).await {
                Ok(Some(p)) => p,
                Ok(None) => return server_error("The plan they pay for no longer exists"),
                Err(e) => {
                    app_log!(error, error = %e, "Plan lookup failed");
                    return server_error("Query failed");
                }
            };
            let off = match gift_amount_off(&paid, &plan) {
                Ok(off) => off,
                Err(why) => return bad_request(&why),
            };
            let gifted = match stripe.gift_coupon(off, &plan.currency).await {
                Ok(coupon) => stripe.switch_subscription_price(id, price, Some(&coupon)).await,
                Err(e) => Err(e),
            };
            if gifted.is_ok() {
                gift_paid_name = Some(paid.name.clone());
                if let Err(e) = client
                    .execute("UPDATE subscriptions SET gift_paid_plan_id = $2 WHERE id = $1", &[&id, &paid.id])
                    .await
                {
                    app_log!(error, error = %e, subscription_id = %id, "Marking the gift failed");
                }
            }
            gifted
        } else if gifted_from.is_some() {
            // Ending a gift: this month was paid at the gifted price, so
            // nothing is prorated, and the next invoice is the new plan's.
            match stripe.switch_subscription_price(id, price, None).await {
                Ok(_) => match stripe.remove_subscription_discount(id).await {
                    Ok(()) => stripe.retrieve_subscription(id).await,
                    Err(e) => Err(e),
                },
                Err(e) => Err(e),
            }
        } else {
            stripe.change_subscription_price(id, price).await
        };
        let sub = match changed {
            Ok(s) => s,
            Err(e) => {
                app_log!(error, error = %e, subscription_id = %id, gift = body.gift, "Stripe plan change failed");
                return server_error(&format!("Stripe refused the change: {}", e));
            }
        };
        // The webhook will say the same; syncing now shows the change at once.
        if let Err(e) = sync_subscription(store.get_ref(), &sub).await {
            app_log!(error, error = %e, subscription_id = %body.subscription_id, "Sync after plan change failed");
            return server_error("Changed in Stripe, but the sync failed: the webhook will catch up");
        }
        if let Some(paid_plan_name) = gift_paid_name {
            send_async(
                Arc::clone(store.get_ref()),
                email.clone(),
                EmailKind::PlanGifted {
                    plan_name: plan.name.clone(),
                    paid_plan_name,
                    apps: if plans::includes(&plan.products, "*") { "*".into() } else { plan.products.join(", ") },
                    monthly_credits: plan.monthly_credits,
                    lang,
                },
            );
        }
    } else {
        // A free licence starts every month with its plan's credits, so it
        // takes the new plan's now. A granted one's credits are a total set by
        // hand for its whole period (scripts/grant-licence.sh): they stay.
        if let Err(e) = client
            .execute(
                "UPDATE subscriptions SET plan_id = $2, \
                        credits_granted = CASE WHEN is_free THEN $3 ELSE credits_granted END, \
                        updated_at = NOW() \
                  WHERE id = $1",
                &[&body.subscription_id, &plan.id, &plan.monthly_credits],
            )
            .await
        {
            app_log!(error, error = %e, "Licence plan update failed");
            return server_error("Update failed");
        }
    }

    app_log!(warn, subscription_id = %body.subscription_id, email = %email, from = %from, to = %plan.id,
        on_stripe, is_free, gift = body.gift, gift_ended = gifted_from.is_some() && !body.gift,
        "Licence plan changed by an administrator");
    HttpResponse::Ok().json(serde_json::json!({
        "success": true, "plan_id": plan.id, "on_stripe": on_stripe, "gift": body.gift,
    }))
}

#[derive(Debug, Deserialize)]
pub struct DeleteUserBody {
    pub email: String,
    /// The email retyped: a person is not deleted by a stray click.
    pub confirm_email: String,
}

/// What goes with a person, in order, each taking their lower-cased email as
/// `$1`. Their keys' usage logs first, since the logs hold a foreign key to
/// the keys. Kept: llm_usage, credit_transactions and the other usage logs
/// (financial history), and email_opt_outs (an unsubscribe must outlive them).
const PERSON_STEPS: &[(&str, &str)] = &[
    ("api_usage_logs", "DELETE FROM api_usage_logs WHERE key_id IN (SELECT id FROM api_keys WHERE lower(email) = $1)"),
    ("api_keys", "DELETE FROM api_keys WHERE lower(email) = $1"),
    ("user_endpoints", "DELETE FROM user_endpoints WHERE lower(email) = $1"),
    ("user_groups", "DELETE FROM user_groups WHERE lower(email) = $1"),
    ("tenant_users", "DELETE FROM tenant_users WHERE lower(email) = $1"),
    ("tenant_invites", "DELETE FROM tenant_invites WHERE lower(email) = $1"),
    ("domains", "DELETE FROM domains WHERE lower(email) = $1"),
    ("reference_data", "DELETE FROM reference_data WHERE lower(email) = $1"),
    ("user_roles", "DELETE FROM user_roles WHERE lower(email) = $1"),
    ("app_user_prefs", "DELETE FROM app_user_prefs WHERE lower(email) = $1"),
    ("user_downstream_credentials", "DELETE FROM user_downstream_credentials WHERE lower(user_email) = $1"),
    ("downstream_oauth_requests", "DELETE FROM downstream_oauth_requests WHERE lower(user_email) = $1"),
    ("channel_link_codes", "DELETE FROM channel_link_codes WHERE lower(user_email) = $1"),
    ("channel_identities", "DELETE FROM channel_identities WHERE lower(user_email) = $1"),
    ("email_opt_ins", "DELETE FROM email_opt_ins WHERE lower(email) = $1"),
    ("subscriptions", "DELETE FROM subscriptions WHERE email = $1"),
    ("user_preferences", "DELETE FROM user_preferences WHERE lower(email) = $1"),
];

pub async fn delete_user(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<DeleteUserBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let email = body.email.trim().to_lowercase();
    if email.is_empty() || body.confirm_email.trim().to_lowercase() != email {
        return bad_request("Confirmation does not match the email");
    }
    let mut client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return server_error("Database unavailable"),
    };
    let tx = match client.transaction().await {
        Ok(t) => t,
        Err(_) => return server_error("Database unavailable"),
    };

    // Stripe would go on billing a subscription whose row is gone.
    match tx
        .query_opt(
            "SELECT 1 FROM subscriptions WHERE email = $1 AND stripe_customer_id <> '' \
                AND status NOT IN ('canceled', 'incomplete_expired', 'upgraded')",
            &[&email],
        )
        .await
    {
        Ok(Some(_)) => return bad_request("They have a Stripe subscription: cancel it first, then delete them"),
        Ok(None) => {}
        Err(e) => {
            app_log!(error, error = %e, "Delete user: subscription check failed");
            return server_error("Query failed");
        }
    }
    match tx
        .query_opt("SELECT 1 FROM user_roles WHERE lower(email) = $1 AND role = 'super_admin'", &[&email])
        .await
    {
        Ok(Some(_)) => return bad_request("A super admin can't be deleted"),
        Ok(None) => {}
        Err(e) => {
            app_log!(error, error = %e, "Delete user: role check failed");
            return server_error("Query failed");
        }
    }

    // Workspaces they own: the ones only they belong to go with them; one that
    // others use and nobody else owns would be left without an owner.
    let owned = match tx
        .query(
            "SELECT t.id, t.name,
                    (SELECT count(*) FROM tenant_users o WHERE o.tenant_id = t.id AND lower(o.email) <> $1),
                    (SELECT count(*) FROM tenant_users o WHERE o.tenant_id = t.id AND lower(o.email) <> $1 AND o.role = 'owner')
               FROM tenants t JOIN tenant_users tu ON tu.tenant_id = t.id
              WHERE lower(tu.email) = $1 AND tu.role = 'owner'",
            &[&email],
        )
        .await
    {
        Ok(rows) => rows,
        Err(e) => {
            app_log!(error, error = %e, "Delete user: workspace check failed");
            return server_error("Query failed");
        }
    };
    let stranded: Vec<String> = owned
        .iter()
        .filter(|r| r.get::<_, i64>(2) > 0 && r.get::<_, i64>(3) == 0)
        .map(|r| r.get(1))
        .collect();
    if !stranded.is_empty() {
        return bad_request(&format!(
            "They are the only owner of {}, which others use: make someone else owner first",
            stranded.join(", ")
        ));
    }
    let solo: Vec<(String, String)> = owned
        .iter()
        .filter(|r| r.get::<_, i64>(2) == 0)
        .map(|r| (r.get(0), r.get(1)))
        .collect();

    let mut removed = serde_json::Map::new();
    for (label, sql) in PERSON_STEPS {
        match tx.execute(*sql, &[&email]).await {
            Ok(n) => {
                removed.insert((*label).to_string(), serde_json::json!(n));
            }
            Err(e) => {
                app_log!(error, error = %e, email = %email, step = %label, "Delete user: step failed, rolling back");
                return server_error(&format!("Failed while clearing {}: {}", label, e));
            }
        }
    }
    for (tenant_id, name) in &solo {
        for (label, sql) in cascade_steps() {
            if let Err(e) = tx.execute(sql.as_str(), &[tenant_id]).await {
                app_log!(error, error = %e, tenant_id = %tenant_id, step = %label, "Delete user: workspace step failed, rolling back");
                return server_error(&format!("Failed while deleting workspace {}: {}", name, e));
            }
        }
    }
    if removed.get("user_preferences").and_then(|v| v.as_u64()) == Some(0)
        && removed.get("subscriptions").and_then(|v| v.as_u64()) == Some(0)
    {
        return fail(actix_web::http::StatusCode::NOT_FOUND, "No such user");
    }
    if let Err(e) = tx.commit().await {
        app_log!(error, error = %e, email = %email, "Delete user: commit failed");
        return server_error("Commit failed");
    }

    let workspaces: Vec<&String> = solo.iter().map(|(_, n)| n).collect();
    app_log!(warn, email = %email, removed = ?removed, workspaces = ?workspaces, "User deleted by an administrator");
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "deleted": email,
        "workspaces_deleted": workspaces,
        "removed": removed,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(id: &str, price_cents: i64, currency: &str) -> plans::Plan {
        plans::Plan {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            price_cents,
            currency: currency.into(),
            monthly_credits: 0,
            products: vec![],
            stripe_product_id: None,
            stripe_price_id: None,
            active: true,
            sort_order: 0,
        }
    }

    #[test]
    fn a_gift_takes_the_difference_off() {
        assert_eq!(gift_amount_off(&plan("basic", 900, "eur"), &plan("pro", 1900, "EUR")), Ok(1000));
    }

    #[test]
    fn a_gift_is_only_an_upgrade_in_one_currency() {
        assert!(gift_amount_off(&plan("pro", 1900, "eur"), &plan("basic", 900, "eur")).is_err());
        assert!(gift_amount_off(&plan("pro", 1900, "eur"), &plan("pro2", 1900, "eur")).is_err());
        assert!(gift_amount_off(&plan("basic", 900, "eur"), &plan("pro", 1900, "chf")).is_err());
    }
}
