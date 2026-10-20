use stripe::{
    BillingPortalSession, CheckoutSession, CheckoutSessionId, CheckoutSessionMode, Client,
    CreateBillingPortalSession, CreateCheckoutSession, CreateCheckoutSessionAutomaticTax,
    CreateCheckoutSessionLineItems, CreateCheckoutSessionPaymentIntentData,
    CreateCheckoutSessionSubscriptionData, CreatePaymentIntent, CreatePrice, CreatePriceRecurring,
    CreatePriceRecurringInterval, CreateProduct, Currency, CustomerId, IdOrCreate, PaymentIntent,
    PaymentIntentId, PaymentIntentStatus, Price, PriceId, PriceTaxBehavior, Product, ProductId,
    Subscription, SubscriptionId, UpdatePrice, UpdateProduct,
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

    pub async fn retrieve_price(&self, price_id: &str) -> Result<Price> {
        let id: PriceId = price_id.parse()?;
        Ok(Price::retrieve(&self.client, &id, &[]).await?)
    }

    // ── Monthly plans ───────────────────────────────────────────────────────

    /// A new monthly price for a plan, on its Stripe product (created first
    /// when `product_id` is `None`). Prices are VAT-inclusive: the amount on the
    /// plans page is what a buyer pays. Returns (product id, price id).
    pub async fn create_plan_price(
        &self,
        product_id: Option<&str>,
        name: &str,
        amount: i64,
        currency: &str,
        metadata: HashMap<String, String>,
    ) -> Result<(String, String)> {
        let currency: Currency = currency.parse().map_err(|_| anyhow::anyhow!("Unknown currency {}", currency))?;
        let product_id = match product_id {
            Some(id) => id.to_string(),
            None => {
                let mut params = CreateProduct::new(name);
                params.metadata = Some(metadata.clone());
                Product::create(&self.client, params).await?.id.to_string()
            }
        };
        let mut params = CreatePrice::new(currency);
        params.product = Some(IdOrCreate::Id(&product_id));
        params.unit_amount = Some(amount);
        params.recurring = Some(CreatePriceRecurring {
            interval: CreatePriceRecurringInterval::Month,
            ..Default::default()
        });
        params.tax_behavior = Some(PriceTaxBehavior::Inclusive);
        params.metadata = Some(metadata);
        let price = Price::create(&self.client, params).await?;
        Ok((product_id, price.id.to_string()))
    }

    /// Takes a price off sale. Subscriptions on it carry on.
    pub async fn archive_price(&self, price_id: &str) -> Result<()> {
        let id: PriceId = price_id.parse()?;
        let params = UpdatePrice { active: Some(false), ..Default::default() };
        Price::update(&self.client, &id, params).await?;
        Ok(())
    }

    pub async fn rename_product(&self, product_id: &str, name: &str) -> Result<()> {
        let id: ProductId = product_id.parse()?;
        let mut params = UpdateProduct::new();
        params.name = Some(name);
        Product::update(&self.client, &id, params).await?;
        Ok(())
    }

    /// A hosted Checkout page that starts a monthly subscription. `metadata`
    /// goes on the session and on the subscription, which is what the webhooks
    /// read back. An earlier customer is reused, so one person keeps one
    /// customer (and one portal) across subscriptions.
    pub async fn create_subscription_checkout(
        &self,
        price_id: &str,
        customer: Option<&str>,
        email: &str,
        tenant_id: &str,
        success_url: &str,
        cancel_url: &str,
        metadata: HashMap<String, String>,
    ) -> Result<CheckoutSession> {
        let mut params = CreateCheckoutSession::new();
        params.mode = Some(CheckoutSessionMode::Subscription);
        params.success_url = Some(success_url);
        params.cancel_url = Some(cancel_url);
        params.client_reference_id = Some(tenant_id);
        let customer_id = customer.map(|c| c.parse::<CustomerId>()).transpose()?;
        match customer_id {
            Some(id) => params.customer = Some(id),
            None => params.customer_email = Some(email),
        }
        params.line_items = Some(vec![CreateCheckoutSessionLineItems {
            price: Some(price_id.to_string()),
            quantity: Some(1),
            ..Default::default()
        }]);
        params.metadata = Some(metadata.clone());
        params.subscription_data = Some(CreateCheckoutSessionSubscriptionData {
            metadata: Some(metadata),
            ..Default::default()
        });
        params.allow_promotion_codes = Some(true);
        if std::env::var("STRIPE_AUTOMATIC_TAX").as_deref() == Ok("true") {
            params.automatic_tax = Some(CreateCheckoutSessionAutomaticTax {
                enabled: true,
                ..Default::default()
            });
        }
        Ok(CheckoutSession::create(&self.client, params).await?)
    }

    pub async fn retrieve_subscription(&self, subscription_id: &str) -> Result<Subscription> {
        let id: SubscriptionId = subscription_id.parse()?;
        Ok(Subscription::retrieve(&self.client, &id, &[]).await?)
    }

    /// Stripe's customer portal: change card, see invoices, cancel.
    pub async fn create_portal_session(&self, customer_id: &str, return_url: &str) -> Result<String> {
        let mut params = CreateBillingPortalSession::new(customer_id.parse()?);
        params.return_url = Some(return_url);
        Ok(BillingPortalSession::create(&self.client, params).await?.url)
    }
}
