// src/payment/subscription.rs
//
// Monthly plan subscriptions (plans in payment::plans): signing up, keeping
// the subscriptions table in step with Stripe, the month's AI credits, and the
// plan key the desktop apps check.
//
//   POST /api/plans/checkout            { email, plan_id } → { url } of a Stripe Checkout page
//   GET  /api/plans/subscription/{email} the current subscription and its credits, or null
//   POST /api/plans/portal              { email } → { url } of Stripe's customer portal
//   GET  /api/plans/key/{email}         a signed plan key, valid to the end of the paid month
//
// All require X-Internal-Secret. The gateway authenticates the person and puts
// their email in the request; the store never takes it from a browser.
//
// Credits: each paid month sets credits_granted to the plan's monthly credits
// and credits_used to 0, so unused credits lapse. A plan switched mid-month
// (customer portal) raises the allowance at once and lowers it at the next
// month. The AI endpoint asks credits_left before a call and charges
// spend_plan_credits after it.
//
// The webhook (payment::webhook) calls sync_subscription on
// checkout.session.completed, invoice.paid, invoice.payment_failed and
// customer.subscription.created/updated/deleted. It always re-reads the
// subscription from Stripe, so events arriving out of order end in the same state.
//
// Plan key, for the apps (same signing key and shape as the licences):
//
//     <base64url(payload JSON)>.<base64url(Ed25519 signature of those bytes)>
//
// payload: {"v":2,"id":"sub_…","product":"plan","edition":"<plan id>","email",
//           "issued","expires"} — dates YYYY-MM-DD. `expires` is the end of the
// paid month plus KEY_GRACE_DAYS, so a renewal has time to arrive; an app
// fetches a new key before then.

use crate::app_log;
use crate::endpoint_store::{tenant_management, EndpointStore};
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::license::signing_key;
use crate::payment::plans::{self, PURPOSE_PLAN};
use crate::payment::service::PaymentService;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Days, NaiveDate, Utc};
use ed25519_dalek::Signer;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use stripe::Subscription;

/// The product name in every plan key.
pub const KEY_PRODUCT: &str = "plan";
const KEY_GRACE_DAYS: u64 = 3;

/// The statuses that give access and credits. A failed renewal (past_due)
/// stops both until Stripe's retry succeeds.
const LIVE_STATUSES: &str = "('active', 'trialing')";

#[derive(Debug, Clone, Serialize)]
pub struct SubscriptionView {
    pub id: String,
    pub plan_id: String,
    pub plan_name: String,
    pub status: String,
    pub live: bool,
    pub cancel_at_period_end: bool,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub credits_granted: i64,
    pub credits_used: i64,
    pub credits_left: i64,
}

/// The subscription that counts for `email`: a live one if any, else the
/// latest, so a lapsed subscriber still sees what they had.
pub async fn current_subscription(store: &EndpointStore, email: &str) -> Result<Option<SubscriptionView>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT s.id, s.plan_id, p.name, s.status, \
                (s.status IN {live} AND s.period_end > NOW()) AS live, \
                s.cancel_at_period_end, s.period_start, s.period_end, s.credits_granted, s.credits_used \
         FROM subscriptions s JOIN plans p ON p.id = s.plan_id \
         WHERE s.email = $1 \
         ORDER BY live DESC, s.period_end DESC LIMIT 1",
        live = LIVE_STATUSES
    );
    let row = client.query_opt(sql.as_str(), &[&email.to_lowercase()]).await.map_err(|e| e.to_string())?;
    Ok(row.map(|r| {
        let (granted, used): (i64, i64) = (r.get(8), r.get(9));
        SubscriptionView {
            id: r.get(0),
            plan_id: r.get(1),
            plan_name: r.get(2),
            status: r.get(3),
            live: r.get(4),
            cancel_at_period_end: r.get(5),
            period_start: r.get(6),
            period_end: r.get(7),
            credits_granted: granted,
            credits_used: used,
            credits_left: (granted - used).max(0),
        }
    }))
}

// ── Credits, for the AI endpoint ─────────────────────────────────────────────

/// Credits left this month, or `None` without a live plan.
pub async fn credits_left(store: &EndpointStore, email: &str) -> Result<Option<i64>, String> {
    Ok(current_subscription(store, email).await?.filter(|s| s.live).map(|s| s.credits_left))
}

#[derive(Debug, PartialEq)]
pub struct Spent {
    pub credits: i64,
    pub credits_left: i64,
}

#[derive(Debug)]
pub enum SpendError {
    NoPlan,
    Store(String),
}

impl std::fmt::Display for SpendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoPlan => write!(f, "No active plan"),
            Self::Store(e) => write!(f, "Credits could not be charged: {}", e),
        }
    }
}

/// Charges one AI call to `email`'s live plan, priced by plans::Pricing. A call
/// that was allowed to start is always charged, even past the allowance; usage
/// is capped at what was granted, so the balance never shows below zero.
pub async fn spend_plan_credits(store: &EndpointStore, email: &str, model: &str, tokens: i64) -> Result<Spent, SpendError> {
    let pricing = plans::read_pricing(store).await.map_err(SpendError::Store)?;
    let cost = pricing.credits_for(model, tokens);
    let client = store.get_admin_conn().await.map_err(|e| SpendError::Store(e.to_string()))?;
    let sql = format!(
        "UPDATE subscriptions SET credits_used = LEAST(credits_granted, credits_used + $2), updated_at = NOW() \
         WHERE id = (SELECT id FROM subscriptions \
                     WHERE email = $1 AND status IN {live} AND period_end > NOW() \
                     ORDER BY period_end DESC LIMIT 1) \
         RETURNING credits_granted - credits_used",
        live = LIVE_STATUSES
    );
    let row = client
        .query_opt(sql.as_str(), &[&email.to_lowercase(), &cost])
        .await
        .map_err(|e| SpendError::Store(e.to_string()))?
        .ok_or(SpendError::NoPlan)?;
    Ok(Spent { credits: cost, credits_left: row.get(0) })
}

// ── Keeping in step with Stripe ──────────────────────────────────────────────

fn timestamp(t: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(t, 0).unwrap_or_else(Utc::now)
}

/// Writes `sub` as Stripe has it now. Subscriptions that are not plan
/// subscriptions (other sites on the same Stripe account) are skipped.
pub async fn sync_subscription(store: &EndpointStore, sub: &Subscription) -> Result<(), String> {
    let meta = &sub.metadata;
    if meta.get("purpose").map(String::as_str) != Some(PURPOSE_PLAN) {
        return Ok(());
    }
    let (Some(tenant_id), Some(email)) = (meta.get("tenant_id"), meta.get("email")) else {
        return Err(format!("Plan subscription {} has no tenant_id/email metadata", sub.id));
    };
    let email = email.trim().to_lowercase();

    // The plan its price belongs to (a switch in the portal changes the
    // price), else the one it was bought as.
    let price_id = sub.items.data.first().and_then(|i| i.price.as_ref()).map(|p| p.id.to_string());
    let mut plan = match price_id.as_deref() {
        Some(p) => plans::plan_for_price(store, p).await?,
        None => None,
    };
    if plan.is_none() {
        if let Some(id) = meta.get("plan_id") {
            plan = plans::get_plan(store, id).await?;
        }
    }
    let plan = plan.ok_or_else(|| format!("Subscription {} matches no plan", sub.id))?;

    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    // A later period start is a new paid month: fresh allowance, nothing used.
    // Within a month, a bigger plan raises the allowance now; a smaller one
    // waits for the next month.
    client
        .execute(
            "INSERT INTO subscriptions (id, tenant_id, email, plan_id, stripe_customer_id, status, \
                                        cancel_at_period_end, period_start, period_end, credits_granted, credits_used) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 0) \
             ON CONFLICT (id) DO UPDATE SET \
                plan_id = EXCLUDED.plan_id, status = EXCLUDED.status, \
                stripe_customer_id = EXCLUDED.stripe_customer_id, \
                cancel_at_period_end = EXCLUDED.cancel_at_period_end, \
                credits_granted = CASE WHEN EXCLUDED.period_start > subscriptions.period_start \
                                       THEN EXCLUDED.credits_granted \
                                       ELSE GREATEST(subscriptions.credits_granted, EXCLUDED.credits_granted) END, \
                credits_used = CASE WHEN EXCLUDED.period_start > subscriptions.period_start \
                                    THEN 0 ELSE subscriptions.credits_used END, \
                period_start = GREATEST(subscriptions.period_start, EXCLUDED.period_start), \
                period_end = GREATEST(subscriptions.period_end, EXCLUDED.period_end), \
                updated_at = NOW()",
            &[
                &sub.id.to_string(),
                tenant_id,
                &email,
                &plan.id,
                &sub.customer.id().to_string(),
                &sub.status.as_str(),
                &sub.cancel_at_period_end,
                &timestamp(sub.current_period_start),
                &timestamp(sub.current_period_end),
                &plan.monthly_credits,
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
    app_log!(info, subscription_id = %sub.id, email = %email, plan = %plan.id, status = %sub.status.as_str(), "Plan subscription synced");
    Ok(())
}

/// Fetches the subscription from Stripe and syncs it.
pub async fn sync_by_id(store: &EndpointStore, stripe: &PaymentService, subscription_id: &str) -> Result<(), String> {
    let sub = stripe.retrieve_subscription(subscription_id).await.map_err(|e| e.to_string())?;
    sync_subscription(store, &sub).await
}

// ── Plan key ─────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct PlanKeyPayload {
    pub v: u8,
    pub id: String,
    pub product: String,
    pub edition: String,
    pub email: String,
    pub issued: NaiveDate,
    pub expires: NaiveDate,
}

pub fn encode_plan_key(payload: &PlanKeyPayload, key: &ed25519_dalek::SigningKey) -> String {
    let json = serde_json::to_vec(payload).expect("plan key payload serialises");
    let signature = key.sign(&json);
    format!("{}.{}", URL_SAFE_NO_PAD.encode(&json), URL_SAFE_NO_PAD.encode(signature.to_bytes()))
}

// ── HTTP ─────────────────────────────────────────────────────────────────────

fn site_url() -> Option<String> {
    std::env::var("PLANS_SITE_URL").ok().map(|s| s.trim_end_matches('/').to_string()).filter(|s| !s.is_empty())
}

fn fail(status: actix_web::http::StatusCode, message: &str) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({ "success": false, "message": message }))
}

#[derive(Deserialize)]
pub struct CheckoutRequest {
    pub email: String,
    pub plan_id: String,
}

/// POST /api/plans/checkout
pub async fn checkout_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    payment_service: web::Data<Arc<PaymentService>>,
    body: web::Json<CheckoutRequest>,
) -> impl Responder {
    use actix_web::http::StatusCode;
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let email = body.email.trim().to_lowercase();
    let Some(site) = site_url() else {
        app_log!(error, "PLANS_SITE_URL is not set — cannot start a plan checkout");
        return fail(StatusCode::SERVICE_UNAVAILABLE, "Plans are not on sale yet");
    };
    let plan = match plans::get_plan(&store, &body.plan_id).await {
        Ok(Some(p)) if p.active && p.stripe_price_id.is_some() => p,
        Ok(_) => return fail(StatusCode::NOT_FOUND, "Unknown plan"),
        Err(e) => {
            app_log!(error, error = %e, "Could not read a plan for checkout");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Checkout could not be started");
        }
    };
    match current_subscription(&store, &email).await {
        Ok(Some(s)) if s.live => {
            return fail(StatusCode::CONFLICT, "You already have a plan — change it from Manage subscription");
        }
        Ok(_) => {}
        Err(e) => {
            app_log!(error, error = %e, "Could not read subscriptions for checkout");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Checkout could not be started");
        }
    }
    let tenant = match tenant_management::get_default_tenant(&store, &email).await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, error = %e, email = %email, "Could not resolve a tenant for plan checkout");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Checkout could not be started");
        }
    };
    let customer = earlier_customer(&store, &email).await;
    let metadata = HashMap::from([
        ("purpose".to_string(), PURPOSE_PLAN.to_string()),
        ("plan_id".to_string(), plan.id.clone()),
        ("tenant_id".to_string(), tenant.id.clone()),
        ("email".to_string(), email.clone()),
    ]);
    let success_url = format!("{}?checkout=success&session_id={{CHECKOUT_SESSION_ID}}", site);
    let price_id = plan.stripe_price_id.as_deref().unwrap_or_default();
    match payment_service
        .create_subscription_checkout(price_id, customer.as_deref(), &email, &tenant.id, &success_url, &site, metadata)
        .await
    {
        Ok(session) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "url": session.url })),
        Err(e) => {
            app_log!(error, error = %e, plan = %plan.id, "Failed to create a plan checkout");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "Checkout could not be started")
        }
    }
}

async fn earlier_customer(store: &EndpointStore, email: &str) -> Option<String> {
    let client = store.get_admin_conn().await.ok()?;
    client
        .query_opt(
            "SELECT stripe_customer_id FROM subscriptions WHERE email = $1 ORDER BY created_at DESC LIMIT 1",
            &[&email],
        )
        .await
        .ok()
        .flatten()
        .map(|r| r.get(0))
}

/// GET /api/plans/subscription/{email}
pub async fn subscription_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match current_subscription(&store, &email).await {
        Ok(sub) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "subscription": sub })),
        Err(e) => {
            app_log!(error, error = %e, "Could not read a subscription");
            fail(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Subscription unavailable")
        }
    }
}

#[derive(Deserialize)]
pub struct PortalRequest {
    pub email: String,
}

/// POST /api/plans/portal
pub async fn portal_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    payment_service: web::Data<Arc<PaymentService>>,
    body: web::Json<PortalRequest>,
) -> impl Responder {
    use actix_web::http::StatusCode;
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let email = body.email.trim().to_lowercase();
    let Some(customer) = earlier_customer(&store, &email).await else {
        return fail(StatusCode::NOT_FOUND, "No subscription to manage");
    };
    let return_url = site_url().unwrap_or_default();
    match payment_service.create_portal_session(&customer, &return_url).await {
        Ok(url) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "url": url })),
        Err(e) => {
            app_log!(error, error = %e, email = %email, "Failed to open the customer portal");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "The subscription page could not be opened")
        }
    }
}

/// GET /api/plans/key/{email}
pub async fn key_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    use actix_web::http::StatusCode;
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let email = email.trim().to_lowercase();
    let sub = match current_subscription(&store, &email).await {
        Ok(Some(s)) if s.live => s,
        Ok(_) => return fail(StatusCode::PAYMENT_REQUIRED, "No active plan"),
        Err(e) => {
            app_log!(error, error = %e, "Could not read a subscription for a plan key");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Key unavailable");
        }
    };
    let signer = match signing_key() {
        Ok(k) => k,
        Err(e) => {
            app_log!(error, error = %e, "Plan key requested but signing is not configured");
            return fail(StatusCode::SERVICE_UNAVAILABLE, "Key unavailable");
        }
    };
    let payload = PlanKeyPayload {
        v: 2,
        id: sub.id.clone(),
        product: KEY_PRODUCT.to_string(),
        edition: sub.plan_id.clone(),
        email,
        issued: Utc::now().date_naive(),
        expires: sub.period_end.date_naive() + Days::new(KEY_GRACE_DAYS),
    };
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "key": encode_plan_key(&payload, &signer),
        "plan": sub.plan_id,
        "expires": payload.expires.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, SigningKey, Verifier};

    #[test]
    fn plan_key_is_a_signed_v2_payload() {
        let signer = SigningKey::from_bytes(&[7u8; 32]);
        let payload = PlanKeyPayload {
            v: 2,
            id: "sub_test".into(),
            product: KEY_PRODUCT.into(),
            edition: "starter".into(),
            email: "anna@band.ch".into(),
            issued: NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(),
            expires: NaiveDate::from_ymd_opt(2026, 11, 22).unwrap(),
        };
        let key = encode_plan_key(&payload, &signer);
        let (body, sig) = key.split_once('.').unwrap();
        let json = URL_SAFE_NO_PAD.decode(body).unwrap();
        let sig = Signature::from_slice(&URL_SAFE_NO_PAD.decode(sig).unwrap()).unwrap();
        signer.verifying_key().verify(&json, &sig).unwrap();
        let text = String::from_utf8(json).unwrap();
        assert!(text.contains(r#""v":2"#) && text.contains(r#""product":"plan""#) && text.contains(r#""expires":"2026-11-22""#));
    }
}
