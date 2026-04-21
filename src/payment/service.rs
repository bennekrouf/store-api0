use stripe::{Client, CreatePaymentIntent, PaymentIntent, Currency, PaymentIntentStatus, PaymentIntentId};
use anyhow::Result;
use crate::app_log;

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
            _ => Currency::USD, // Default
        };

        let mut create_intent = CreatePaymentIntent::new(amount, currency_enum);
        create_intent.receipt_email = Some(email);

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
}
