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
//   POST /api/plans/free                    { email } → { emailed: true }: the free allowance's
//                                           licence, sent to that email only, which proves it
//   GET  /api/plans/session/{id}            the licence a finished Checkout session bought
//   GET  /api/plans/subscription/{email}    the current subscription and its credits, or null
//   POST /api/plans/portal                  { email } → { url } of Stripe's customer portal
//   POST /api/plans/cancel                  { email, cancel } → cancels the email's paid plan at
//                                           the end of its month (cancel: true), or resumes it
//   POST /api/internal/licence/status       { licence } → what it unlocks, and its credits
//   POST /api/internal/licence/portal       { licence } → { url } of Stripe's customer portal
//   POST /api/internal/licence/email        { licence, app, title, intro, items } → emails
//                                           what an app found to the licence's own email,
//                                           never anyone else; LICENCE_EMAILS_PER_DAY a licence
//
// All require X-Internal-Secret; the gateway is the only caller.
//
// Credits: each paid month sets credits_granted to the plan's monthly credits
// and credits_used to 0, so unused credits lapse. A plan switched mid-month
// (customer portal) raises the allowance at once and lowers it at the next
// month. The AI endpoint reads the licence's credits before a call and
// charges spend_credits after it.
//
// The free allowance (an active plan priced 0, never listed) has no Stripe
// behind it: its subscription is made here, one per email, and the store
// starts each new month itself when the licence is next checked. When the same email first pays for a plan, the
// free licence moves to that subscription, so the number in the apps stays.
//
// The webhook (payment::webhook) calls sync_subscription on
// checkout.session.completed, invoice.paid, invoice.payment_failed and
// customer.subscription.created/updated/deleted. It always re-reads the
// subscription from Stripe, so events arriving out of order end in the same state.
//
// Emails: the licence once the subscription is first live; a confirmation when
// it is cancelled, however that happened (here, the portal, or Stripe giving up
// on a card); and one "come back" reminder a week after a cancelled plan ended,
// unless they have a plan again by then (send_come_back_reminders, daily).

use crate::app_log;
use crate::email::{send_async, DigestItem, EmailKind};
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
    /// Only ever shown to the person signed in with this email.
    pub licence: Option<String>,
    /// The apps it makes Pro; "*" is every app.
    pub products: Vec<String>,
    pub is_free: bool,
}

/// The subscription that counts for `email`: a live one if any, a paid or
/// granted plan before the free allowance, else the latest, so a lapsed
/// subscriber still sees what they had.
pub async fn current_subscription(store: &EndpointStore, email: &str) -> Result<Option<SubscriptionView>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT s.id, s.plan_id, p.name, s.status, \
                (s.status IN {live} AND s.period_end > NOW()) AS live, \
                s.cancel_at_period_end, s.period_start, s.period_end, s.credits_granted, s.credits_used, \
                s.licence, p.products, s.is_free \
         FROM subscriptions s JOIN plans p ON p.id = s.plan_id \
         WHERE s.email = $1 AND s.status <> 'upgraded' \
         ORDER BY live DESC, s.is_free ASC, s.period_end DESC LIMIT 1",
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
            licence: r.get(10),
            products: r.get(11),
            is_free: r.get(12),
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
    let Some(licence) = normalise_licence(typed) else {
        return Ok(None);
    };
    start_free_month(store, &licence).await?;
    licence_where(store, "s.licence = $1", &licence).await
}

/// A free licence whose month is over starts the next one, with the plan's
/// credits as they are now. Paid ones are renewed by Stripe instead.
async fn start_free_month(store: &EndpointStore, licence: &str) -> Result<(), String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    client
        .execute(
            "UPDATE subscriptions s SET period_start = NOW(), period_end = NOW() + INTERVAL '1 month', \
                credits_used = 0, credits_granted = p.monthly_credits, updated_at = NOW() \
             FROM plans p \
             WHERE p.id = s.plan_id AND s.is_free AND s.licence = $1 AND s.period_end <= NOW()",
            &[&licence],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The same for the email's free licence, when its credits are read by email
/// (a partner's balance read or charge) rather than by licence: someone who only
/// uses the allowance through a partner still gets a new month.
pub async fn start_free_month_for_email(store: &EndpointStore, email: &str) -> Result<(), String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    client
        .execute(
            "UPDATE subscriptions s SET period_start = NOW(), period_end = NOW() + INTERVAL '1 month', \
                credits_used = 0, credits_granted = p.monthly_credits, updated_at = NOW() \
             FROM plans p \
             WHERE p.id = s.plan_id AND s.is_free AND s.email = $1 AND s.status = 'active' AND s.period_end <= NOW()",
            &[&email.trim().to_lowercase()],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
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
    let pricing = plans::effective_pricing(store).await.map_err(SpendError::Store)?;
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

    // A free licence of the same email moves to the first paid subscription,
    // so the number already pasted into the apps keeps working — but only
    // once this one is paid, and before its own licence has been emailed.
    if ["active", "trialing"].contains(&sub.status.as_str()) {
        let taken = client
            .query_opt(
                "UPDATE subscriptions SET licence = NULL, status = 'upgraded', updated_at = NOW() \
                 WHERE email = $1 AND is_free AND licence IS NOT NULL \
                   AND EXISTS (SELECT 1 FROM subscriptions WHERE id = $2 AND licence_emailed_at IS NULL) \
                 RETURNING licence",
                &[&email, &sub.id.to_string()],
            )
            .await
            .map_err(|e| e.to_string())?;
        if let Some(row) = taken {
            let licence: String = row.get(0);
            client
                .execute("UPDATE subscriptions SET licence = $2 WHERE id = $1", &[&sub.id.to_string(), &licence])
                .await
                .map_err(|e| e.to_string())?;
            app_log!(info, subscription_id = %sub.id, email = %email, "Free licence moved to a paid subscription");
        }
    }

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
        send_async(Arc::clone(store), email.clone(), licence_email(&plan, licence));
    }

    // Resumed: a later cancel is confirmed again.
    client
        .execute(
            "UPDATE subscriptions SET cancel_emailed_at = NULL \
             WHERE id = $1 AND NOT cancel_at_period_end AND status <> 'canceled'",
            &[&sub.id.to_string()],
        )
        .await
        .map_err(|e| e.to_string())?;
    // Cancelled, once, and only a subscription that was ever live: an
    // abandoned Checkout expires without anyone having subscribed.
    let cancelled = client
        .query_opt(
            "UPDATE subscriptions SET cancel_emailed_at = NOW() \
             WHERE id = $1 AND cancel_emailed_at IS NULL AND licence_emailed_at IS NOT NULL \
               AND (cancel_at_period_end OR status = 'canceled') \
             RETURNING GREATEST(period_end, NOW())",
            &[&sub.id.to_string()],
        )
        .await
        .map_err(|e| e.to_string())?;
    if let Some(row) = cancelled {
        let ends: DateTime<Utc> = row.get(0);
        app_log!(info, subscription_id = %sub.id, email = %email, "Plan cancelled");
        send_async(
            Arc::clone(store),
            email,
            EmailKind::PlanCancelled { plan_name: plan.name.clone(), ends_on: ends.format("%-d %B %Y").to_string() },
        );
    }
    Ok(())
}

/// How long after a cancelled plan ends the one reminder goes out, and the
/// oldest ending still reminded (so turning this on does not mail everyone who
/// ever left).
const COME_BACK_AFTER: &str = "7 days";
const COME_BACK_UNTIL: &str = "30 days";

/// Daily: the "come back" reminder, once per cancelled plan, to people who
/// have no plan again. It is optional, so opted-out addresses are skipped when
/// sending.
pub async fn send_come_back_reminders(store: &Arc<EndpointStore>) {
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "Could not connect to send come-back reminders");
            return;
        }
    };
    let sql = format!(
        "UPDATE subscriptions s SET comeback_emailed_at = NOW() \
         FROM plans p \
         WHERE p.id = s.plan_id AND NOT s.is_free \
           AND s.cancel_emailed_at IS NOT NULL AND s.comeback_emailed_at IS NULL \
           AND s.period_end < NOW() - INTERVAL '{COME_BACK_AFTER}' \
           AND s.period_end > NOW() - INTERVAL '{COME_BACK_UNTIL}' \
           AND NOT EXISTS (SELECT 1 FROM subscriptions o \
                           WHERE o.email = s.email AND NOT o.is_free \
                             AND o.status IN {live} AND o.period_end > NOW()) \
         RETURNING s.email, p.name",
        live = LIVE_STATUSES
    );
    let rows = match client.query(sql.as_str(), &[]).await {
        Ok(rows) => rows,
        Err(e) => {
            app_log!(error, error = %e, "Could not pick the come-back reminders");
            return;
        }
    };
    // Two old plans of the same person are both marked, and mailed once.
    let mut sent = std::collections::HashSet::new();
    for row in rows {
        let email: String = row.get(0);
        if sent.insert(email.clone()) {
            send_async(Arc::clone(store), email, EmailKind::PlanComeBack { plan_name: row.get(1) });
        }
    }
    app_log!(info, count = sent.len(), "Come-back reminders sent");
}

fn licence_email(plan: &plans::Plan, licence: String) -> EmailKind {
    EmailKind::LicenceIssued {
        plan_name: plan.name.clone(),
        licence,
        apps: if plans::includes(&plan.products, "*") { "*".into() } else { plan.products.join(", ") },
        monthly_credits: plan.monthly_credits,
    }
}

/// How long before the same email can be sent a free licence again.
const FREE_RESEND_AFTER: &str = "1 hour";

/// A free plan's licence for `email`: made the first time, sent again on
/// later asks (at most once an hour, so the form cannot flood an inbox).
/// Only ever emailed — the caller never sees it.
async fn give_free_plan(store: &Arc<EndpointStore>, plan: &plans::Plan, email: &str) -> Result<(), String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let resend_sql = format!(
        "UPDATE subscriptions SET licence_emailed_at = NOW() \
         WHERE email = $1 AND is_free AND licence IS NOT NULL \
           AND (licence_emailed_at IS NULL OR licence_emailed_at < NOW() - INTERVAL '{FREE_RESEND_AFTER}') \
         RETURNING licence"
    );
    if let Some(row) = client.query_opt(resend_sql.as_str(), &[&email]).await.map_err(|e| e.to_string())? {
        app_log!(info, email = %email, "Free licence sent again");
        send_async(Arc::clone(store), email.to_string(), licence_email(plan, row.get(0)));
        return Ok(());
    }
    let id = format!("free_{}", uuid::Uuid::new_v4().simple());
    let inserted = client
        .query_opt(
            "INSERT INTO subscriptions (id, email, plan_id, stripe_customer_id, status, period_start, period_end, \
                                        credits_granted, credits_used, licence, licence_emailed_at, is_free) \
             VALUES ($1, $2, $3, '', 'active', NOW(), NOW() + INTERVAL '1 month', $4, 0, $5, NOW(), TRUE) \
             ON CONFLICT DO NOTHING \
             RETURNING licence",
            &[&id, &email, &plan.id, &plan.monthly_credits, &new_licence()],
        )
        .await
        .map_err(|e| e.to_string())?;
    match inserted {
        Some(row) => {
            app_log!(info, email = %email, plan = %plan.id, "Free licence issued");
            send_async(Arc::clone(store), email.to_string(), licence_email(plan, row.get(0)));
        }
        // Already has one, sent within the hour: nothing more to send.
        None => app_log!(info, email = %email, "Free licence asked for again within the hour; not resent"),
    }
    Ok(())
}

/// Fetches the subscription from Stripe and syncs it.
pub async fn sync_by_id(store: &Arc<EndpointStore>, stripe: &PaymentService, subscription_id: &str) -> Result<(), String> {
    let sub = stripe.retrieve_subscription(subscription_id).await.map_err(|e| e.to_string())?;
    sync_subscription(store, &sub).await
}

// ── HTTP ─────────────────────────────────────────────────────────────────────

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
    let plan = match plans::get_plan(&store, &body.plan_id).await {
        Ok(Some(p)) if p.active && !p.is_free() && p.stripe_price_id.is_some() => p,
        Ok(_) => return fail(StatusCode::NOT_FOUND, "Unknown plan"),
        Err(e) => {
            app_log!(error, error = %e, "Could not read a plan for checkout");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Checkout could not be started");
        }
    };
    // A paid plan already running wins over anything asked for here.
    match has_live_paid_plan(&store, &email).await {
        Ok(true) => {
            return fail(StatusCode::CONFLICT, "You already have a plan — change it from Manage subscription");
        }
        Ok(false) => {}
        Err(e) => {
            app_log!(error, error = %e, "Could not read subscriptions for checkout");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Checkout could not be started");
        }
    }
    let site = crate::email::plans_page();
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

#[derive(Deserialize)]
pub struct FreeRequest {
    pub email: String,
}

/// POST /api/plans/free — the free allowance, for an app to offer. Answers the
/// same whether the licence is new or sent again, and never contains it. A
/// partner whose workspace bills each person's plan starts it for a new user
/// (service key with credits.write).
pub async fn free_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<FreeRequest>,
) -> impl Responder {
    use actix_web::http::StatusCode;
    if let Err(deny) = crate::middleware::service_key::require_scope(&req, crate::middleware::service_key::Scope::CreditsWrite) {
        return deny;
    }
    let email = body.email.trim().to_lowercase();
    if !plausible_email(&email) {
        return fail(StatusCode::BAD_REQUEST, "Enter the email the licence should go to");
    }
    match has_live_paid_plan(&store, &email).await {
        Ok(true) => return fail(StatusCode::CONFLICT, "This email already has a plan: use its licence"),
        Ok(false) => {}
        Err(e) => {
            app_log!(error, error = %e, "Could not read subscriptions for a free licence");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "The free licence could not be sent");
        }
    }
    let plan = match plans::list_plans(&store, true).await {
        Ok(ps) => ps.into_iter().find(|p| p.is_free()),
        Err(e) => {
            app_log!(error, error = %e, "Could not read the plans for a free licence");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "The free licence could not be sent");
        }
    };
    let Some(plan) = plan else {
        return fail(StatusCode::NOT_FOUND, "There are no free credits right now");
    };
    match give_free_plan(&store, &plan, &email).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "emailed": true,
            "message": "Your licence number is on its way to that email.",
        })),
        Err(e) => {
            app_log!(error, error = %e, "Could not give a free licence");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "The free licence could not be sent")
        }
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
    let return_url = crate::email::plans_page();
    match payment_service
        .create_portal_session(&customer, &return_url, plans::portal_configuration(&store).await.as_deref())
        .await {
        Ok(url) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "url": url })),
        Err(e) => {
            app_log!(error, error = %e, "Failed to open the customer portal for a licence");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "The subscription page could not be opened")
        }
    }
}

/// How many emails one licence may have an app send a day.
const LICENCE_EMAILS_PER_DAY: u32 = 30;

#[derive(Deserialize)]
pub struct LicenceEmailRequest {
    pub licence: String,
    /// The app's name as the holder knows it ("JobScout"), shown in the email.
    pub app: String,
    pub title: String,
    #[serde(default)]
    pub intro: String,
    #[serde(default)]
    pub items: Vec<DigestItem>,
}

/// One line of an app's text, trimmed, within `max` characters and without
/// control characters (it can reach the Subject header).
fn digest_text(raw: &str, max: usize) -> Option<String> {
    let t = raw.trim();
    (t.chars().count() <= max && !t.chars().any(|c| c.is_control())).then(|| t.to_string())
}

/// An app's request, checked: a short single-line app name and title, a short
/// intro, at most 50 items, and only plain https links.
fn digest_of(body: &LicenceEmailRequest) -> Result<EmailKind, &'static str> {
    let app = digest_text(&body.app, 40).filter(|a| !a.is_empty()).ok_or("app is a short name")?;
    let title = digest_text(&body.title, 150).filter(|t| !t.is_empty()).ok_or("title is one short line")?;
    let intro = body.intro.trim();
    if intro.chars().count() > 2000 {
        return Err("intro is at most 2000 characters");
    }
    if body.items.is_empty() || body.items.len() > 50 {
        return Err("items holds 1 to 50 lines");
    }
    let mut items = Vec::with_capacity(body.items.len());
    for i in &body.items {
        let title = digest_text(&i.title, 200).filter(|t| !t.is_empty()).ok_or("each item has a short title")?;
        let detail = digest_text(&i.detail, 500).ok_or("an item's detail is one line of at most 500 characters")?;
        let url = i.url.trim().to_string();
        if !url.is_empty()
            && !(url.starts_with("https://")
                && url.len() <= 2000
                && url.chars().all(|c| c.is_ascii_graphic() && !matches!(c, '"' | '\'' | '<' | '>')))
        {
            return Err("an item's url is a plain https link");
        }
        items.push(DigestItem { title, detail, url });
    }
    Ok(EmailKind::AppDigest { app, title, intro: intro.to_string(), items })
}

/// Counts one email for `licence` today; false once it has had its day's.
fn take_licence_email(licence: &str) -> bool {
    use std::sync::Mutex;
    static SENT: Mutex<Option<HashMap<String, (chrono::NaiveDate, u32)>>> = Mutex::new(None);
    let today = Utc::now().date_naive();
    let mut sent = SENT.lock().unwrap_or_else(|e| e.into_inner());
    let sent = sent.get_or_insert_with(HashMap::new);
    sent.retain(|_, (day, _)| *day == today);
    let n = sent.entry(licence.to_string()).or_insert((today, 0));
    if n.1 >= LICENCE_EMAILS_PER_DAY {
        return false;
    }
    n.1 += 1;
    true
}

/// POST /api/internal/licence/email — an app emailing its own user what it
/// found. Goes to the licence's email only, so the licence is all the proof
/// needed; the licence must be active (free allowance included).
pub async fn licence_email_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<LicenceEmailRequest>,
) -> impl Responder {
    use actix_web::http::StatusCode;
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let licence = match licence_status(&store, &body.licence).await {
        Ok(Some(l)) => l,
        Ok(None) => return fail(StatusCode::NOT_FOUND, "Unknown licence"),
        Err(e) => {
            app_log!(error, error = %e, "Could not read a licence for an app email");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Licence unavailable");
        }
    };
    if !licence.active {
        return fail(StatusCode::PAYMENT_REQUIRED, "This licence's subscription is not active");
    }
    let kind = match digest_of(&body) {
        Ok(k) => k,
        Err(why) => return fail(StatusCode::BAD_REQUEST, why),
    };
    if !take_licence_email(&licence.licence) {
        return fail(StatusCode::TOO_MANY_REQUESTS, "Too many emails from this licence today");
    }
    let to = licence.email.clone();
    send_async(store.get_ref().clone(), to.clone(), kind);
    HttpResponse::Accepted().json(serde_json::json!({ "success": true, "emailed": true, "to": to }))
}

#[derive(Deserialize)]
pub struct CancelRequest {
    pub email: String,
    /// False resumes a plan that was cancelled but has not ended yet.
    pub cancel: bool,
}

/// POST /api/plans/cancel — answers with the subscription as it now is.
pub async fn cancel_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    payment_service: web::Data<Arc<PaymentService>>,
    body: web::Json<CancelRequest>,
) -> impl Responder {
    use actix_web::http::StatusCode;
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let email = body.email.trim().to_lowercase();
    let sub_id = match live_paid_subscription(&store, &email).await {
        Ok(Some(id)) => id,
        Ok(None) => return fail(StatusCode::NOT_FOUND, "No plan to cancel"),
        Err(e) => {
            app_log!(error, error = %e, "Could not read subscriptions to cancel");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "The plan could not be changed");
        }
    };
    let sub = match payment_service.set_cancel_at_period_end(&sub_id, body.cancel).await {
        Ok(sub) => sub,
        Err(e) => {
            app_log!(error, error = %e, subscription_id = %sub_id, "Stripe refused to change cancel_at_period_end");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "The plan could not be changed");
        }
    };
    app_log!(info, subscription_id = %sub_id, email = %email, cancel = body.cancel, "Plan cancel changed by its owner");
    // Synced now, so the answer and the email do not wait for the webhook.
    if let Err(e) = sync_subscription(&store, &sub).await {
        app_log!(error, error = %e, subscription_id = %sub_id, "Could not sync a subscription after cancelling");
    }
    match current_subscription(&store, &email).await {
        Ok(sub) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "subscription": sub })),
        Err(e) => {
            app_log!(error, error = %e, "Could not read a subscription after cancelling");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "Subscription unavailable")
        }
    }
}

/// The email's paid plan that is running now, if any.
async fn live_paid_subscription(store: &EndpointStore, email: &str) -> Result<Option<String>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT id FROM subscriptions \
         WHERE email = $1 AND NOT is_free AND status IN {live} AND period_end > NOW() \
         ORDER BY period_end DESC LIMIT 1",
        live = LIVE_STATUSES
    );
    let row = client.query_opt(sql.as_str(), &[&email]).await.map_err(|e| e.to_string())?;
    Ok(row.map(|r| r.get(0)))
}

async fn has_live_paid_plan(store: &EndpointStore, email: &str) -> Result<bool, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT EXISTS (SELECT 1 FROM subscriptions \
                        WHERE email = $1 AND NOT is_free AND status IN {live} AND period_end > NOW())",
        live = LIVE_STATUSES
    );
    let row = client.query_one(sql.as_str(), &[&email]).await.map_err(|e| e.to_string())?;
    Ok(row.get(0))
}

/// The Stripe customer behind the email's latest paid subscription.
async fn earlier_customer(store: &EndpointStore, email: &str) -> Option<String> {
    let client = store.get_admin_conn().await.ok()?;
    client
        .query_opt(
            "SELECT stripe_customer_id FROM subscriptions \
             WHERE email = $1 AND stripe_customer_id <> '' ORDER BY created_at DESC LIMIT 1",
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
    let return_url = crate::email::plans_page();
    match payment_service
        .create_portal_session(&customer, &return_url, plans::portal_configuration(&store).await.as_deref())
        .await {
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

    fn digest(items: Vec<DigestItem>) -> LicenceEmailRequest {
        LicenceEmailRequest {
            licence: "ABCDE-FGHJK-LMNPQ-RSTUV".into(),
            app: "JobScout".into(),
            title: "3 new jobs for rust developer".into(),
            intro: String::new(),
            items,
        }
    }

    fn item(title: &str, url: &str) -> DigestItem {
        DigestItem { title: title.into(), detail: "Zurich · 82/100".into(), url: url.into() }
    }

    #[test]
    fn an_app_email_takes_short_lines_and_plain_https_links_only() {
        assert!(digest_of(&digest(vec![item("Rust developer", "https://example.ch/jobs/1")])).is_ok());
        assert!(digest_of(&digest(vec![item("No link", "")])).is_ok());
        assert!(digest_of(&digest(vec![])).is_err());
        assert!(digest_of(&digest(vec![item("x", "http://example.ch")])).is_err());
        assert!(digest_of(&digest(vec![item("x", "javascript:alert(1)")])).is_err());
        assert!(digest_of(&digest(vec![item("x", "https://a.ch/\"onmouseover=x")])).is_err());
        let mut header = digest(vec![item("x", "")]);
        header.title = "Hi\r\nBcc: someone@example.com".into();
        assert!(digest_of(&header).is_err());
        assert!(digest_of(&digest((0..51).map(|_| item("x", "")).collect())).is_err());
    }

    #[test]
    fn an_app_email_escapes_what_the_app_wrote() {
        let kind = digest_of(&digest(vec![item("<b>Rust</b> & co", "https://example.ch/jobs/1?a=1&b=2")])).unwrap();
        let html = kind.html_body_with(None);
        assert!(html.contains("&lt;b&gt;Rust&lt;/b&gt; &amp; co"));
        assert!(html.contains("https://example.ch/jobs/1?a=1&amp;b=2"));
        assert_eq!(kind.subject(), "JobScout: 3 new jobs for rust developer");
    }

    #[test]
    fn a_licence_gets_its_days_emails_and_no_more() {
        let licence = "TESTS-DAILY-LIMIT-ONLY0";
        for _ in 0..LICENCE_EMAILS_PER_DAY {
            assert!(take_licence_email(licence));
        }
        assert!(!take_licence_email(licence));
        assert!(take_licence_email("TESTS-ANOTHER-LICENCE0"));
    }
}
