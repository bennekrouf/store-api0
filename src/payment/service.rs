use stripe::{
    CheckoutSession, CheckoutSessionId, CheckoutSessionMode, Client, CreateCheckoutSession,
    CreateCheckoutSessionAutomaticTax, CreateCheckoutSessionLineItems,
    CreateCheckoutSessionPaymentIntentData, CreatePaymentIntent, Currency, PaymentIntent,
    PaymentIntentId, PaymentIntentStatus,
};
use anyhow::Result;
use std::collections::HashMap;
use crate::app_log;
use crate::payment::topup::PURPOSE_CREDITS;

pub struct PaymentService {
    client: Client,
}

impl PaymentService {
    pub fn new(secret_key: String) -> Self {
        Self {
            client: Client::new(secret_key),
        }
    }

    pub async fn create_payment_intent(
        &self,
        amount: i64,
        currency: &str,
        email: &str,
    ) -> Result<PaymentIntent> {
        // Stripe expects amount in smallest currency unit (e.g., cents)
        let currency_enum = match currency.to_lowercase().as_str() {
            "usd" => Currency::USD,
            "eur" => Currency::EUR,
            "chf" => Currency::CHF,
            _ => Currency::USD, // Default
        };

        let mut create_intent = CreatePaymentIntent::new(amount, currency_enum);
        create_intent.receipt_email = Some(email);
        // Read back when the intent is credited: the account comes from here,
        // not from whoever reports the payment. See payment::topup.
        create_intent.metadata = Some(HashMap::from([
            ("purpose".to_string(), PURPOSE_CREDITS.to_string()),
            ("email".to_string(), email.to_string()),
        ]));

        app_log!(info, "Creating payment intent for {} ({} {})", email, amount, currency);

        let intent = PaymentIntent::create(&self.client, create_intent).await?;
        Ok(intent)
    }

    pub async fn confirm_payment(
        &self,
        payment_intent_id: &str,
    ) -> Result<PaymentIntent> {
        app_log!(info, "Verifying payment intent status: {}", payment_intent_id);

        let id: PaymentIntentId = payment_intent_id.parse()?;
        let intent = PaymentIntent::retrieve(&self.client, &id, &[]).await?;

        if intent.status != PaymentIntentStatus::Succeeded {
             app_log!(warn, "Payment intent {} status is {:?}", payment_intent_id, intent.status);
        }

        Ok(intent)
    }

    /// A hosted Checkout page for one licence of a desktop app. `metadata` is
    /// copied onto both the session and its PaymentIntent, so the webhook that
    /// completes the session and a later refund can both be traced back to it.
    pub async fn create_license_checkout(
        &self,
        price_id: &str,
        success_url: &str,
        cancel_url: &str,
        metadata: HashMap<String, String>,
    ) -> Result<CheckoutSession> {
        let mut params = CreateCheckoutSession::new();
        params.mode = Some(CheckoutSessionMode::Payment);
        params.success_url = Some(success_url);
        params.cancel_url = Some(cancel_url);
        params.line_items = Some(vec![CreateCheckoutSessionLineItems {
            price: Some(price_id.to_string()),
            quantity: Some(1),
            ..Default::default()
        }]);
        params.metadata = Some(metadata.clone());
        params.payment_intent_data = Some(CreateCheckoutSessionPaymentIntentData {
            metadata: Some(metadata),
            ..Default::default()
        });
        params.allow_promotion_codes = Some(true);
        // Stripe Tax works out VAT from the buyer's address; it needs the
        // account's tax registrations set up in the Stripe dashboard.
        if std::env::var("STRIPE_AUTOMATIC_TAX").as_deref() == Ok("true") {
            params.automatic_tax = Some(CreateCheckoutSessionAutomaticTax {
                enabled: true,
                ..Default::default()
            });
        }

        Ok(CheckoutSession::create(&self.client, params).await?)
    }

    pub async fn retrieve_checkout_session(&self, session_id: &str) -> Result<CheckoutSession> {
        let id: CheckoutSessionId = session_id.parse()?;
        Ok(CheckoutSession::retrieve(&self.client, &id, &[]).await?)
    }
}
