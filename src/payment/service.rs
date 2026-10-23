use stripe::{
    BillingPortalConfiguration, BillingPortalSession, CheckoutSession, CheckoutSessionId, CheckoutSessionMode, Client,
    CreateBillingPortalSession, CreateCheckoutSession, CreateCheckoutSessionAutomaticTax,
    CreateCheckoutSessionLineItems,
    CreateCheckoutSessionSubscriptionData, CreatePaymentIntent, CreatePrice, CreatePriceRecurring,
    CreatePriceRecurringInterval, CreateProduct, Currency, CustomerId, IdOrCreate, PaymentIntent,
    PaymentIntentId, PaymentIntentStatus, Price, PriceId, PriceTaxBehavior, Product, ProductId,
    Subscription, SubscriptionId, UpdatePrice, UpdateProduct, UpdateSubscription,
};
use anyhow::Result;
use std::collections::HashMap;
use crate::app_log;
use crate::payment::topup::PURPOSE_CREDITS;

/// Stripe's "General - Electronically Supplied Services", as the app licences
/// sold before the plans had.
const PLAN_TAX_CODE: &str = "txcd_10000000";

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

    pub async fn retrieve_checkout_session(&self, session_id: &str) -> Result<CheckoutSession> {
        let id: CheckoutSessionId = session_id.parse()?;
        Ok(CheckoutSession::retrieve(&self.client, &id, &[]).await?)
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

    /// What Checkout, the customer portal and invoices show for a plan's
    /// product: its description, its logo (`None` keeps the current one), and
    /// the tax code Stripe Tax works VAT out from.
    pub async fn update_plan_product(&self, product_id: &str, description: &str, image_url: Option<&str>) -> Result<()> {
        let id: ProductId = product_id.parse()?;
        let mut params = UpdateProduct::new();
        if !description.is_empty() {
            params.description = Some(description.to_string());
        }
        params.images = image_url.map(|u| vec![u.to_string()]);
        params.tax_code = Some(PLAN_TAX_CODE.to_string());
        Product::update(&self.client, &id, params).await?;
        Ok(())
    }

    /// Creates the customer portal's configuration, or updates `existing`, so
    /// a subscriber can switch between the plans on sale (`plans`: each
    /// product with its price), change card, see invoices, and cancel at the
    /// end of the paid month. A switch is prorated: the difference is billed
    /// or credited at once. Returns its id, which portal sessions are opened with.
    pub async fn save_portal_configuration(&self, existing: Option<&str>, plans: &[(String, String)]) -> Result<String> {
        let form = portal_form(plans);
        let path = match existing {
            Some(id) => format!("/billing_portal/configurations/{}", id),
            None => "/billing_portal/configurations".to_string(),
        };
        let saved: BillingPortalConfiguration = self.client.post_form(&path, form).await?;
        Ok(saved.id.to_string())
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
        success_url: &str,
        cancel_url: &str,
        metadata: HashMap<String, String>,
    ) -> Result<CheckoutSession> {
        let mut params = CreateCheckoutSession::new();
        params.mode = Some(CheckoutSessionMode::Subscription);
        params.success_url = Some(success_url);
        params.cancel_url = Some(cancel_url);
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

    /// Cancels at the end of the paid month (`true`), or takes that back
    /// (`false`). The subscription runs, and is billed, until then either way.
    pub async fn set_cancel_at_period_end(&self, subscription_id: &str, cancel: bool) -> Result<Subscription> {
        let id: SubscriptionId = subscription_id.parse()?;
        let params = UpdateSubscription { cancel_at_period_end: Some(cancel), ..Default::default() };
        Ok(Subscription::update(&self.client, &id, params).await?)
    }

    /// Stripe's customer portal: change card, see invoices, cancel.
    /// `configuration`: the one save_portal_configuration made, else Stripe's default.
    pub async fn create_portal_session(&self, customer_id: &str, return_url: &str, configuration: Option<&str>) -> Result<String> {
        let mut params = CreateBillingPortalSession::new(customer_id.parse()?);
        params.return_url = Some(return_url);
        params.configuration = configuration;
        Ok(BillingPortalSession::create(&self.client, params).await?.url)
    }
}

/// The customer portal's settings, as Stripe's form body takes them.
fn portal_form(plans: &[(String, String)]) -> serde_json::Value {
    let products: Vec<_> = plans
        .iter()
        .map(|(product, price)| serde_json::json!({ "product": product, "prices": [price] }))
        .collect();
    serde_json::json!({
        "business_profile": { "headline": "Manage your mayorana plan" },
        "features": {
            "invoice_history": { "enabled": true },
            "payment_method_update": { "enabled": true },
            "subscription_cancel": { "enabled": true, "mode": "at_period_end" },
            "subscription_update": {
                // Stripe refuses an enabled switch with no products to switch to.
                "enabled": !products.is_empty(),
                "default_allowed_updates": if products.is_empty() { vec![] } else { vec!["price", "promotion_code"] },
                "proration_behavior": "create_prorations",
                "products": products,
            },
        },
    })
}

#[cfg(test)]
mod portal_tests {
    use super::portal_form;

    /// The form as Stripe reads it, brackets unescaped.
    fn encoded(plans: &[(String, String)]) -> String {
        serde_qs::to_string(&portal_form(plans)).unwrap().replace("%5B", "[").replace("%5D", "]")
    }

    #[test]
    fn the_portal_switches_between_the_plans_on_sale() {
        let plans = vec![("prod_a".to_string(), "price_a".to_string()), ("prod_b".to_string(), "price_b".to_string())];
        let form = encoded(&plans);
        for part in [
            "features[subscription_update][enabled]=true",
            "features[subscription_update][products][0][product]=prod_a",
            "features[subscription_update][products][1][prices][0]=price_b",
            "features[subscription_update][default_allowed_updates][0]=price",
            "features[subscription_cancel][mode]=at_period_end",
        ] {
            assert!(form.contains(part), "{part} not in {form}");
        }
    }

    #[test]
    fn with_nothing_on_sale_switching_is_off() {
        assert!(encoded(&[]).contains("features[subscription_update][enabled]=false"));
    }
}
