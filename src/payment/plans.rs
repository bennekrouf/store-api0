// src/payment/plans.rs
//
// Monthly plans, the only thing on sale: Pro in the apps a plan lists, plus a
// monthly allowance of AI credits. The super admin sets them in the dashboard;
// a site lists the active ones on its pricing page. A subscription gets one
// licence number (payment::subscription) that carries what its plan includes.
//
// A plan priced at 0 is the free allowance, not an offer: it is never listed
// for a pricing page. An app asks for it by email (POST /api/plans/free), the
// licence is emailed, and its few credits refill every month. Its products are
// usually none: a taste of the AI features, no Pro.
//
//   GET /api/plans                    the active plans, for a pricing page
//   GET /api/admin/plans              every plan, with Stripe ids, and the credit pricing
//   PUT /api/admin/plans/{id}         { name, description, price_cents, currency,
//                                       monthly_credits, products, active, sort_order }
//                                     — creates or updates. `products` are the app ids
//                                     the plan makes Pro, "*" for every app.
//   PUT /api/admin/plans-pricing      { tokens_per_credit, model_multipliers, app_uses }
//
// All require X-Internal-Secret; the gateway restricts the admin routes to the
// super admin.
//
// A plan's Stripe price is made here, never pasted in: saving an active plan
// whose amount or currency changed creates a new monthly price and archives the
// old one. Subscribers already on the old price keep paying it; their credits
// follow the plan as it is now, from their next paid month.
//
// Credits: an AI call costs ceil(tokens × multiplier ÷ tokens_per_credit), at
// least 1. With the default 10,000 tokens per credit, one credit is one
// ordinary call (scoring a job, writing a commit message: 2–6k tokens), so
// "500 credits" reads as "about 500 AI actions".

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::service::PaymentService;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

/// Metadata `purpose` on plan subscriptions, their Checkout sessions and
/// Stripe products and prices.
pub const PURPOSE_PLAN: &str = "plan";

const DEFAULT_TOKENS_PER_CREDIT: i64 = 10_000;
const CURRENCIES: &[&str] = &["eur", "chf", "usd"];

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub id: String,
    pub name: String,
    pub description: String,
    pub price_cents: i64,
    pub currency: String,
    pub monthly_credits: i64,
    /// The apps it makes Pro, by the id each app checks for; "*" is every app.
    pub products: Vec<String>,
    pub stripe_product_id: Option<String>,
    pub stripe_price_id: Option<String>,
    pub active: bool,
    pub sort_order: i32,
}

const PLAN_COLUMNS: &str = "id, name, description, price_cents, currency, monthly_credits, \
                            stripe_product_id, stripe_price_id, active, sort_order, products";

impl Plan {
    /// Free plans are given, not bought: no Stripe, one per email.
    pub fn is_free(&self) -> bool {
        self.price_cents == 0
    }
}

fn plan_from_row(r: &tokio_postgres::Row) -> Plan {
    Plan {
        id: r.get(0),
        name: r.get(1),
        description: r.get(2),
        price_cents: r.get(3),
        currency: r.get(4),
        monthly_credits: r.get(5),
        stripe_product_id: r.get(6),
        stripe_price_id: r.get(7),
        active: r.get(8),
        sort_order: r.get(9),
        products: r.get(10),
    }
}

pub async fn list_plans(store: &EndpointStore, only_active: bool) -> Result<Vec<Plan>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT {} FROM plans {} ORDER BY sort_order, id",
        PLAN_COLUMNS,
        if only_active { "WHERE active" } else { "" }
    );
    let rows = client.query(sql.as_str(), &[]).await.map_err(|e| e.to_string())?;
    Ok(rows.iter().map(plan_from_row).collect())
}

pub async fn get_plan(store: &EndpointStore, id: &str) -> Result<Option<Plan>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!("SELECT {} FROM plans WHERE id = $1", PLAN_COLUMNS);
    let row = client.query_opt(sql.as_str(), &[&id]).await.map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(plan_from_row))
}

/// The plan a Stripe price belongs to, archived prices included — as long as
/// the plan still points at it. Used when a subscriber switches plan in the
/// customer portal.
pub async fn plan_for_price(store: &EndpointStore, price_id: &str) -> Result<Option<Plan>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!("SELECT {} FROM plans WHERE stripe_price_id = $1", PLAN_COLUMNS);
    let row = client.query_opt(sql.as_str(), &[&price_id]).await.map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(plan_from_row))
}

// ── Credit pricing ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Pricing {
    pub tokens_per_credit: i64,
    pub model_multipliers: HashMap<String, f64>,
    /// What one use of each app's AI costs, for a pricing page to say what a
    /// plan's credits buy ("~4 repos a month"). Shown in this order. Only an
    /// estimate for buyers: calls are charged by tokens, as above.
    #[serde(default)]
    pub app_uses: Vec<AppUse>,
}

/// One use of an app's AI — a repository for a month, a video — in credits.
/// What a "use" is, in words, is the site's to say, by `app`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppUse {
    pub app: String,
    pub credits: i64,
}

impl Default for Pricing {
    fn default() -> Self {
        Self { tokens_per_credit: DEFAULT_TOKENS_PER_CREDIT, model_multipliers: HashMap::new(), app_uses: Vec::new() }
    }
}

impl Pricing {
    /// What one AI call costs: ceil(tokens × multiplier ÷ tokens_per_credit),
    /// at least 1 for any call that used tokens.
    pub fn credits_for(&self, model: &str, tokens: i64) -> i64 {
        if tokens <= 0 {
            return 0;
        }
        let factor = self.model_multipliers.get(model).copied().unwrap_or(1.0).max(0.0);
        let per = self.tokens_per_credit.max(1) as f64;
        ((tokens as f64 * factor / per).ceil() as i64).max(1)
    }

    fn check(&self) -> Result<(), String> {
        if self.tokens_per_credit < 1 {
            return Err("tokens_per_credit must be at least 1".into());
        }
        if let Some((m, f)) = self.model_multipliers.iter().find(|(_, f)| !(f.is_finite() && **f > 0.0)) {
            return Err(format!("The multiplier for {} must be above 0 (got {})", m, f));
        }
        let mut seen = Vec::new();
        for u in &self.app_uses {
            let id_ok = !u.app.is_empty()
                && u.app.len() <= 40
                && u.app.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            if !id_ok {
                return Err(format!("'{}' is not an app id: lowercase letters, digits and dashes", u.app));
            }
            if u.credits < 1 {
                return Err(format!("One use of {} must cost at least 1 credit", u.app));
            }
            if seen.contains(&&u.app) {
                return Err(format!("{} is listed twice", u.app));
            }
            seen.push(&u.app);
        }
        Ok(())
    }
}

pub async fn read_pricing(store: &EndpointStore) -> Result<Pricing, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let rows = client
        .query(
            "SELECT key, value FROM system_config \
             WHERE key IN ('plans.tokens_per_credit', 'plans.model_multipliers', 'plans.app_uses')",
            &[],
        )
        .await
        .map_err(|e| e.to_string())?;
    let mut pricing = Pricing::default();
    for row in rows {
        let (k, v): (&str, &str) = (row.get(0), row.get(1));
        match k {
            "plans.tokens_per_credit" => {
                if let Ok(n) = v.trim().parse::<i64>() {
                    pricing.tokens_per_credit = n.max(1);
                }
            }
            "plans.model_multipliers" => match serde_json::from_str(v) {
                Ok(m) => pricing.model_multipliers = m,
                Err(e) => app_log!(warn, error = %e, "plans.model_multipliers is not a JSON object of numbers; ignoring it"),
            },
            "plans.app_uses" => match serde_json::from_str(v) {
                Ok(u) => pricing.app_uses = u,
                Err(e) => app_log!(warn, error = %e, "plans.app_uses is not a JSON list of {{app, credits}}; ignoring it"),
            },
            _ => {}
        }
    }
    Ok(pricing)
}

// ── HTTP ─────────────────────────────────────────────────────────────────────

fn public_plan(p: &Plan) -> serde_json::Value {
    serde_json::json!({
        "id": p.id,
        "name": p.name,
        "description": p.description,
        "price_cents": p.price_cents,
        "currency": p.currency,
        "monthly_credits": p.monthly_credits,
        "products": p.products,
        "sort_order": p.sort_order,
    })
}

/// GET /api/plans — what a pricing page shows: the paid plans with a Stripe
/// price, so every plan shown can be bought. The free allowance is not one.
pub async fn public_plans_handler(req: HttpRequest, store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let (plans, pricing) = match (list_plans(&store, true).await, read_pricing(&store).await) {
        (Ok(p), Ok(c)) => (p, c),
        (Err(e), _) | (_, Err(e)) => {
            app_log!(error, error = %e, "Could not read the plans");
            return HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "message": "Plans unavailable" }));
        }
    };
    let plans: Vec<_> = plans.iter().filter(|p| !p.is_free() && p.stripe_price_id.is_some()).map(public_plan).collect();
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "plans": plans,
        "tokens_per_credit": pricing.tokens_per_credit,
        "app_uses": pricing.app_uses,
    }))
}

/// GET /api/admin/plans
pub async fn admin_plans_handler(req: HttpRequest, store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match (list_plans(&store, false).await, read_pricing(&store).await) {
        (Ok(plans), Ok(pricing)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "plans": plans,
            "pricing": pricing,
            "currencies": CURRENCIES,
        })),
        (Err(e), _) | (_, Err(e)) => {
            app_log!(error, error = %e, "Could not read the plans");
            HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "message": e }))
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PlanInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub price_cents: i64,
    pub currency: String,
    pub monthly_credits: i64,
    #[serde(default)]
    pub products: Vec<String>,
    pub active: bool,
    #[serde(default)]
    pub sort_order: i32,
}

impl PlanInput {
    fn check(&mut self, id: &str) -> Result<(), String> {
        let slug = !id.is_empty()
            && id.len() <= 40
            && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !slug {
            return Err("The plan id must be lowercase letters, digits and dashes".into());
        }
        self.name = self.name.trim().to_string();
        self.description = self.description.trim().to_string();
        self.currency = self.currency.trim().to_lowercase();
        if self.name.is_empty() {
            return Err("A plan needs a name".into());
        }
        if !CURRENCIES.contains(&self.currency.as_str()) {
            return Err(format!("Currency must be one of {}", CURRENCIES.join(", ")));
        }
        // 0 is a free plan. Otherwise Stripe's own floor is 0.50 in these currencies.
        if self.price_cents != 0 && self.price_cents < 50 {
            return Err("The price must be 0 (free) or at least 0.50".into());
        }
        if self.monthly_credits < 0 {
            return Err("Monthly credits cannot be negative".into());
        }
        self.products = normalise_products(&self.products)?;
        Ok(())
    }
}

/// App ids, lowercased, deduplicated and in order; "*" stands alone.
fn normalise_products(products: &[String]) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for p in products {
        let p = p.trim().to_lowercase();
        if p.is_empty() {
            continue;
        }
        let ok = p == "*"
            || (p.len() <= 40 && p.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        if !ok {
            return Err(format!("'{}' is not an app id: lowercase letters, digits and dashes", p));
        }
        if !out.contains(&p) {
            out.push(p);
        }
    }
    if out.iter().any(|p| p == "*") {
        return Ok(vec!["*".into()]);
    }
    Ok(out)
}

/// Whether a plan with `products` makes `app` Pro.
pub fn includes(products: &[String], app: &str) -> bool {
    products.iter().any(|p| p == "*" || p == app)
}

/// PUT /api/admin/plans/{id}
pub async fn save_plan_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    payment_service: web::Data<Arc<PaymentService>>,
    id: web::Path<String>,
    body: web::Json<PlanInput>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let id = id.into_inner();
    let mut input = body.into_inner();
    if let Err(why) = input.check(&id) {
        return HttpResponse::BadRequest().json(serde_json::json!({ "success": false, "message": why }));
    }
    match save_plan(&store, &payment_service, &id, input).await {
        Ok(plan) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "plan": plan })),
        Err(e) => {
            app_log!(error, error = %e, plan = %id, "Could not save a plan");
            HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "message": e }))
        }
    }
}

async fn save_plan(store: &EndpointStore, stripe: &PaymentService, id: &str, input: PlanInput) -> Result<Plan, String> {
    let existing = get_plan(store, id).await?;
    let mut product_id = existing.as_ref().and_then(|p| p.stripe_product_id.clone());
    let mut price_id = existing.as_ref().and_then(|p| p.stripe_price_id.clone());

    let price_changed = existing
        .as_ref()
        .map(|p| p.price_cents != input.price_cents || p.currency != input.currency)
        .unwrap_or(true);
    // Only an active plan needs a price; an inactive one gets it when it goes on sale.
    // A free plan has no price; one turned free keeps its old one archived.
    if input.price_cents == 0 {
        if let Some(old) = price_id.take() {
            if let Err(e) = stripe.archive_price(&old).await {
                app_log!(warn, error = %e, price_id = %old, "Could not archive the Stripe price of a plan made free");
            }
        }
    } else if input.active && (price_changed || price_id.is_none()) {
        let metadata = HashMap::from([
            ("purpose".to_string(), PURPOSE_PLAN.to_string()),
            ("plan_id".to_string(), id.to_string()),
        ]);
        let (product, price) = stripe
            .create_plan_price(product_id.as_deref(), &input.name, input.price_cents, &input.currency, metadata)
            .await
            .map_err(|e| format!("Stripe refused the new price: {}", e))?;
        if let Some(old) = price_id.as_deref() {
            if let Err(e) = stripe.archive_price(old).await {
                app_log!(warn, error = %e, price_id = %old, "Could not archive a plan's previous Stripe price");
            }
        }
        app_log!(info, plan = %id, price_id = %price, amount = input.price_cents, currency = %input.currency, "Created a Stripe price for a plan");
        product_id = Some(product);
        price_id = Some(price);
    }
    if let (Some(product), Some(old)) = (product_id.as_deref(), existing.as_ref()) {
        if old.name != input.name {
            if let Err(e) = stripe.rename_product(product, &input.name).await {
                app_log!(warn, error = %e, product_id = %product, "Could not rename a plan's Stripe product");
            }
        }
    }

    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!(
        "INSERT INTO plans (id, name, description, price_cents, currency, monthly_credits, \
                            stripe_product_id, stripe_price_id, active, sort_order, products, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NOW()) \
         ON CONFLICT (id) DO UPDATE SET \
            name = EXCLUDED.name, description = EXCLUDED.description, \
            price_cents = EXCLUDED.price_cents, currency = EXCLUDED.currency, \
            monthly_credits = EXCLUDED.monthly_credits, \
            stripe_product_id = EXCLUDED.stripe_product_id, stripe_price_id = EXCLUDED.stripe_price_id, \
            active = EXCLUDED.active, sort_order = EXCLUDED.sort_order, \
            products = EXCLUDED.products, updated_at = NOW() \
         RETURNING {}",
        PLAN_COLUMNS
    );
    let row = client
        .query_one(
            sql.as_str(),
            &[
                &id,
                &input.name,
                &input.description,
                &input.price_cents,
                &input.currency,
                &input.monthly_credits,
                &product_id,
                &price_id,
                &input.active,
                &input.sort_order,
                &input.products,
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(plan_from_row(&row))
}

/// PUT /api/admin/plans-pricing
pub async fn save_pricing_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<Pricing>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let pricing = body.into_inner();
    if let Err(why) = pricing.check() {
        return HttpResponse::BadRequest().json(serde_json::json!({ "success": false, "message": why }));
    }
    let multipliers = serde_json::to_string(&pricing.model_multipliers).unwrap_or_else(|_| "{}".into());
    let app_uses = serde_json::to_string(&pricing.app_uses).unwrap_or_else(|_| "[]".into());
    let result = async {
        let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
        client
            .execute(
                "INSERT INTO system_config (key, value, updated_at) VALUES \
                    ('plans.tokens_per_credit', $1, NOW()), ('plans.model_multipliers', $2, NOW()), \
                    ('plans.app_uses', $3, NOW()) \
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
                &[&pricing.tokens_per_credit.to_string(), &multipliers, &app_uses],
            )
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    match result {
        Ok(_) => {
            app_log!(info, tokens_per_credit = pricing.tokens_per_credit, multipliers = %multipliers, app_uses = %app_uses, "Plan credit pricing changed");
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "pricing": pricing }))
        }
        Err(e) => {
            app_log!(error, error = %e, "Could not save the plan credit pricing");
            HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "message": e }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_costs_its_tokens_rounded_up_and_at_least_one_credit() {
        let pricing = Pricing {
            tokens_per_credit: 10_000,
            model_multipliers: HashMap::from([("deepseek-reasoner".to_string(), 3.0)]),
            app_uses: Vec::new(),
        };
        assert_eq!(pricing.credits_for("deepseek-chat", 0), 0);
        assert_eq!(pricing.credits_for("deepseek-chat", 1), 1);
        assert_eq!(pricing.credits_for("deepseek-chat", 3_500), 1);
        assert_eq!(pricing.credits_for("deepseek-chat", 10_000), 1);
        assert_eq!(pricing.credits_for("deepseek-chat", 10_001), 2);
        assert_eq!(pricing.credits_for("deepseek-reasoner", 3_500), 2);
        assert_eq!(pricing.credits_for("deepseek-reasoner", 10_000), 3);
    }

    #[test]
    fn pricing_and_plans_are_checked() {
        assert!(Pricing { tokens_per_credit: 0, ..Default::default() }.check().is_err());
        let bad = Pricing { tokens_per_credit: 10, model_multipliers: HashMap::from([("m".into(), 0.0)]), app_uses: Vec::new() };
        assert!(bad.check().is_err());

        let mut plan = PlanInput {
            name: " Starter ".into(),
            description: String::new(),
            price_cents: 1900,
            currency: "EUR".into(),
            monthly_credits: 500,
            products: vec![" GitAgent ".into(), "jobscout".into(), "gitagent".into()],
            active: true,
            sort_order: 1,
        };
        assert!(plan.check("starter").is_ok());
        assert_eq!(plan.products, vec!["gitagent", "jobscout"]);
        assert_eq!((plan.name.as_str(), plan.currency.as_str()), ("Starter", "eur"));
        assert!(plan.check("Starter Plan").is_err());
        plan.price_cents = 10;
        assert!(plan.check("starter").is_err());
    }

    #[test]
    fn a_plan_names_the_apps_it_unlocks_or_all_of_them() {
        assert_eq!(normalise_products(&["jobscout".into(), "*".into()]).unwrap(), vec!["*"]);
        assert!(normalise_products(&["Job Scout".into()]).is_err());
        assert!(includes(&["gitagent".into()], "gitagent"));
        assert!(!includes(&["gitagent".into()], "jobscout"));
        assert!(includes(&["*".into()], "jobscout"));
        assert!(!includes(&[], "gitagent"));
    }

    #[test]
    fn what_an_app_use_costs_is_checked() {
        let with = |uses: Vec<(&str, i64)>| Pricing {
            app_uses: uses.into_iter().map(|(a, c)| AppUse { app: a.into(), credits: c }).collect(),
            ..Default::default()
        };
        assert!(with(vec![("gitagent", 50), ("jobscout", 100)]).check().is_ok());
        assert!(with(vec![("gitagent", 0)]).check().is_err());
        assert!(with(vec![("Git Agent", 5)]).check().is_err());
        assert!(with(vec![("gitagent", 5), ("gitagent", 6)]).check().is_err());
        // A pricing saved before app uses existed still reads.
        let old: Pricing = serde_json::from_str(r#"{"tokens_per_credit":10000,"model_multipliers":{}}"#).unwrap();
        assert!(old.app_uses.is_empty());
    }
}
