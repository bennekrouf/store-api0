// src/payment/subscription.rs
//
// Subscriptions to the monthly plans (payment::plans), the only thing on sale.
// Someone picks a plan and gives their email; Stripe takes the payment; the
// subscription gets one licence number, emailed to them, which
//   - unlocks Pro in the apps the plan lists (apps ask GET /v1/licence), and
//   - pays for AI calls from the plan's monthly credits (/v1/chat/completions).
// No api0 account is needed: the email and the licence are the identity.
//
//   POST /api/plans/checkout                { email, plan_id } → { url } of a Stripe Checkout page
//   GET  /api/plans/session/{id}            the licence a finished Checkout session bought
//   GET  /api/plans/subscription/{email}    the current subscription and its credits, or null
//   POST /api/plans/portal                  { email } → { url } of Stripe's customer portal
//   POST /api/internal/licence/status       { licence } → what it unlocks, and its credits
//   POST /api/internal/licence/portal       { licence } → { url } of Stripe's customer portal
//
// All require X-Internal-Secret; the gateway is the only caller.
//
// Credits: each paid month sets credits_granted to the plan's monthly credits
// and credits_used to 0, so unused credits lapse. A plan switched mid-month
// (customer portal) raises the allowance at once and lowers it at the next
// month. The AI endpoint reads the licence's credits before a call and
// charges spend_credits after it.
//
// The webhook (payment::webhook) calls sync_subscription on
// checkout.session.completed, invoice.paid, invoice.payment_failed and
// customer.subscription.created/updated/deleted. It always re-reads the
// subscription from Stripe, so events arriving out of order end in the same state.

use crate::app_log;
use crate::email::{send_async, EmailKind};
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::plans::{self, PURPOSE_PLAN};
use crate::payment::service::PaymentService;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use chrono::{DateTime, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use stripe::Subscription;

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

// ── The licence ──────────────────────────────────────────────────────────────

/// Twenty characters from an alphabet with no look-alikes (100 bits), in four
/// groups: typed from an email, read aloud, pasted into an app.
const LICENCE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const LICENCE_GROUPS: usize = 4;
const LICENCE_GROUP_LEN: usize = 5;

fn new_licence() -> String {
    let mut rng = rand::rng();
    let chars: String = (0..LICENCE_GROUPS * LICENCE_GROUP_LEN)
        .map(|_| LICENCE_ALPHABET[rng.random_range(0..LICENCE_ALPHABET.len())] as char)
        .collect();
    group(&chars)
}

fn group(chars: &str) -> String {
    chars
        .as_bytes()
        .chunks(LICENCE_GROUP_LEN)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

/// A licence as typed — any case, spaces or dashes — in its stored form, or
/// `None` if it cannot be one.
pub fn normalise_licence(typed: &str) -> Option<String> {
    let chars: String = typed
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let ok = chars.len() == LICENCE_GROUPS * LICENCE_GROUP_LEN
        && chars.bytes().all(|b| LICENCE_ALPHABET.contains(&b));
    ok.then(|| group(&chars))
}

/// What a licence unlocks right now.
#[derive(Debug, Clone, Serialize)]
pub struct LicenceStatus {
    pub licence: String,
    pub subscription_id: String,
    pub email: String,
    pub plan_id: String,
    pub plan_name: String,
    /// The apps it makes Pro; "*" is every app.
    pub products: Vec<String>,
    pub status: String,
    /// Paid up: Pro and AI credits. False once it lapses or is cancelled.
    pub active: bool,
    pub cancel_at_period_end: bool,
    pub period_end: DateTime<Utc>,
    pub credits_granted: i64,
    pub credits_used: i64,
    pub credits_left: i64,
}

async fn licence_where(store: &EndpointStore, condition: &str, value: &str) -> Result<Option<LicenceStatus>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT s.licence, s.id, s.email, s.plan_id, p.name, p.products, s.status, \
                (s.status IN {live} AND s.period_end > NOW()), \
                s.cancel_at_period_end, s.period_end, s.credits_granted, s.credits_used \
         FROM subscriptions s JOIN plans p ON p.id = s.plan_id \
         WHERE {condition} AND s.licence IS NOT NULL",
        live = LIVE_STATUSES
    );
    let row = client.query_opt(sql.as_str(), &[&value]).await.map_err(|e| e.to_string())?;
    Ok(row.map(|r| {
        let (granted, used): (i64, i64) = (r.get(10), r.get(11));
        LicenceStatus {
            licence: r.get(0),
            subscription_id: r.get(1),
            email: r.get(2),
            plan_id: r.get(3),
            plan_name: r.get(4),
            products: r.get(5),
            status: r.get(6),
            active: r.get(7),
            cancel_at_period_end: r.get(8),
            period_end: r.get(9),
            credits_granted: granted,
            credits_used: used,
            credits_left: (granted - used).max(0),
        }
    }))
}

/// The licence as typed, or `None` when no subscription has it.
pub async fn licence_status(store: &EndpointStore, typed: &str) -> Result<Option<LicenceStatus>, String> {
    match normalise_licence(typed) {
        Some(licence) => licence_where(store, "s.licence = $1", &licence).await,
        None => Ok(None),
    }
}

async fn licence_of_subscription(store: &EndpointStore, subscription_id: &str) -> Result<Option<LicenceStatus>, String> {
    licence_where(store, "s.id = $1", subscription_id).await
}

// ── Credits, for the AI endpoint ─────────────────────────────────────────────

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

/// Charges one AI call to a live subscription, priced by plans::Pricing. A
/// call that was allowed to start is always charged, even past the allowance;
/// usage is capped at what was granted, so the balance never shows below zero.
pub async fn spend_credits(store: &EndpointStore, subscription_id: &str, model: &str, tokens: i64) -> Result<Spent, SpendError> {
    let pricing = plans::read_pricing(store).await.map_err(SpendError::Store)?;
    let cost = pricing.credits_for(model, tokens);
    let client = store.get_admin_conn().await.map_err(|e| SpendError::Store(e.to_string()))?;
    let sql = format!(
        "UPDATE subscriptions SET credits_used = LEAST(credits_granted, credits_used + $2), updated_at = NOW() \
         WHERE id = $1 AND status IN {live} AND period_end > NOW() \
         RETURNING credits_granted - credits_used",
        live = LIVE_STATUSES
    );
    let row = client
        .query_opt(sql.as_str(), &[&subscription_id, &cost])
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
pub async fn sync_subscription(store: &Arc<EndpointStore>, sub: &Subscription) -> Result<(), String> {
    let meta = &sub.metadata;
    if meta.get("purpose").map(String::as_str) != Some(PURPOSE_PLAN) {
        return Ok(());
    }
    let Some(email) = meta.get("email") else {
        return Err(format!("Plan subscription {} has no email metadata", sub.id));
    };
    let email = email.trim().to_lowercase();
    // Set by sign-ups from before subscriptions needed no account.
    let tenant_id: Option<String> = meta.get("tenant_id").filter(|t| !t.is_empty()).cloned();

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
                                        cancel_at_period_end, period_start, period_end, credits_granted, credits_used, licence) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 0, $11) \
             ON CONFLICT (id) DO UPDATE SET \
                licence = COALESCE(subscriptions.licence, EXCLUDED.licence), \
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
                &tenant_id,
                &email,
                &plan.id,
                &sub.customer.id().to_string(),
                &sub.status.as_str(),
                &sub.cancel_at_period_end,
                &timestamp(sub.current_period_start),
                &timestamp(sub.current_period_end),
                &plan.monthly_credits,
                &new_licence(),
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
    app_log!(info, subscription_id = %sub.id, email = %email, plan = %plan.id, status = %sub.status.as_str(), "Plan subscription synced");

    // The licence goes out once, when the subscription first turns live: a
    // Checkout still being paid creates the subscription before it is.
    let sql = format!(
        "UPDATE subscriptions SET licence_emailed_at = NOW() \
         WHERE id = $1 AND licence_emailed_at IS NULL AND status IN {live} \
         RETURNING licence",
        live = LIVE_STATUSES
    );
    let fresh = client.query_opt(sql.as_str(), &[&sub.id.to_string()]).await.map_err(|e| e.to_string())?;
    if let Some(row) = fresh {
        let licence: String = row.get(0);
        app_log!(info, subscription_id = %sub.id, email = %email, "Licence issued");
        send_async(Arc::clone(store), email, EmailKind::LicenceIssued {
            plan_name: plan.name.clone(),
            licence,
            apps: if plans::includes(&plan.products, "*") { String::new() } else { plan.products.join(", ") },
            monthly_credits: plan.monthly_credits,
        });
    }
    Ok(())
}

/// Fetches the subscription from Stripe and syncs it.
pub async fn sync_by_id(store: &Arc<EndpointStore>, stripe: &PaymentService, subscription_id: &str) -> Result<(), String> {
    let sub = stripe.retrieve_subscription(subscription_id).await.map_err(|e| e.to_string())?;
    sync_subscription(store, &sub).await
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
    if !plausible_email(&email) {
        return fail(StatusCode::BAD_REQUEST, "Enter the email the licence should go to");
    }
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
    let customer = earlier_customer(&store, &email).await;
    let metadata = HashMap::from([
        ("purpose".to_string(), PURPOSE_PLAN.to_string()),
        ("plan_id".to_string(), plan.id.clone()),
        ("email".to_string(), email.clone()),
    ]);
    let success_url = format!("{}?checkout=success&session_id={{CHECKOUT_SESSION_ID}}", site);
    let price_id = plan.stripe_price_id.as_deref().unwrap_or_default();
    match payment_service
        .create_subscription_checkout(price_id, customer.as_deref(), &email, &success_url, &site, metadata)
        .await
    {
        Ok(session) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "url": session.url })),
        Err(e) => {
            app_log!(error, error = %e, plan = %plan.id, "Failed to create a plan checkout");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "Checkout could not be started")
        }
    }
}

/// Enough to catch a typo before Stripe does; Stripe checks it properly.
fn plausible_email(email: &str) -> bool {
    match email.split_once('@') {
        Some((user, domain)) => !user.is_empty() && domain.contains('.') && !email.contains(char::is_whitespace),
        None => false,
    }
}

/// GET /api/plans/session/{id} — for the page Checkout returns to. The id is
/// Stripe's unguessable cs_…, which only reaches the buyer's browser.
pub async fn session_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    payment_service: web::Data<Arc<PaymentService>>,
    session_id: web::Path<String>,
) -> impl Responder {
    use actix_web::http::StatusCode;
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let session = match payment_service.retrieve_checkout_session(&session_id).await {
        Ok(s) => s,
        Err(e) => {
            app_log!(warn, error = %e, "Could not read a Checkout session");
            return fail(StatusCode::NOT_FOUND, "Unknown checkout");
        }
    };
    let is_plan = session.metadata.as_ref().and_then(|m| m.get("purpose")).map(String::as_str) == Some(PURPOSE_PLAN);
    let Some(sub_id) = session.subscription.as_ref().map(|s| s.id().to_string()).filter(|_| is_plan) else {
        return fail(StatusCode::NOT_FOUND, "Unknown checkout");
    };
    // The page can come back before the webhook: sync now rather than wait.
    if let Err(e) = sync_by_id(&store, &payment_service, &sub_id).await {
        app_log!(error, error = %e, subscription_id = %sub_id, "Could not sync a subscription for its thank-you page");
    }
    match licence_of_subscription(&store, &sub_id).await {
        Ok(Some(l)) if l.active => HttpResponse::Ok().json(serde_json::json!({ "success": true, "licence": l })),
        // Paid by a method that settles later: the email follows once it does.
        Ok(_) => HttpResponse::Accepted().json(serde_json::json!({
            "success": true, "licence": null, "message": "Payment is being confirmed — your licence will be emailed to you.",
        })),
        Err(e) => {
            app_log!(error, error = %e, "Could not read a licence for a Checkout session");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "Licence unavailable")
        }
    }
}

#[derive(Deserialize)]
pub struct LicenceRequest {
    pub licence: String,
}

/// POST /api/internal/licence/status
pub async fn licence_status_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<LicenceRequest>,
) -> impl Responder {
    use actix_web::http::StatusCode;
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match licence_status(&store, &body.licence).await {
        Ok(Some(l)) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "licence": l })),
        Ok(None) => fail(StatusCode::NOT_FOUND, "Unknown licence"),
        Err(e) => {
            app_log!(error, error = %e, "Could not read a licence");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "Licence unavailable")
        }
    }
}

/// POST /api/internal/licence/portal — managing the subscription from an app,
/// where the licence is all there is.
pub async fn licence_portal_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    payment_service: web::Data<Arc<PaymentService>>,
    body: web::Json<LicenceRequest>,
) -> impl Responder {
    use actix_web::http::StatusCode;
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let licence = match licence_status(&store, &body.licence).await {
        Ok(Some(l)) => l,
        Ok(None) => return fail(StatusCode::NOT_FOUND, "Unknown licence"),
        Err(e) => {
            app_log!(error, error = %e, "Could not read a licence for the portal");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Licence unavailable");
        }
    };
    let Some(customer) = earlier_customer(&store, &licence.email).await else {
        return fail(StatusCode::NOT_FOUND, "No subscription to manage");
    };
    let return_url = site_url().unwrap_or_default();
    match payment_service.create_portal_session(&customer, &return_url).await {
        Ok(url) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "url": url })),
        Err(e) => {
            app_log!(error, error = %e, "Failed to open the customer portal for a licence");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "The subscription page could not be opened")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_licence_is_four_groups_of_five_unambiguous_characters() {
        let l = new_licence();
        assert_eq!(l.len(), 23);
        assert_eq!(l.matches('-').count(), 3);
        assert!(l.replace('-', "").bytes().all(|b| LICENCE_ALPHABET.contains(&b)));
        assert_ne!(new_licence(), l);
    }

    #[test]
    fn a_licence_is_found_however_it_is_typed() {
        let l = new_licence();
        assert_eq!(normalise_licence(&l).as_deref(), Some(l.as_str()));
        let sloppy = format!(" {} ", l.replace('-', " ").to_lowercase());
        assert_eq!(normalise_licence(&sloppy).as_deref(), Some(l.as_str()));
        assert_eq!(normalise_licence("ABCDE-FGHJK-LMNPQ"), None, "too short");
        assert_eq!(normalise_licence("ABCDE-FGHJK-LMNPQ-RSTU0"), None, "0 is not in the alphabet");
    }

    #[test]
    fn an_email_needs_a_name_and_a_domain() {
        assert!(plausible_email("anna@band.ch"));
        for bad in ["", "anna", "anna@", "@band.ch", "anna@band", "an na@band.ch"] {
            assert!(!plausible_email(bad), "{bad:?}");
        }
    }
}
