// src/payment/workspace_plans.rs
//
// Workspace plans: what api0 sells to companies. A workspace pays a monthly
// price that includes some seats and a monthly allowance of credits; each
// member past the included seats costs a seat price; credits used past the
// allowance are overage, billed on the next invoice at the plan's rate.
//
//   Starter / Growth / Business   on sale, bought with Stripe Checkout
//   Enterprise (custom)           never on sale: granted by the super admin and
//                                 invoiced by hand
//
// Kept apart from the mayorana app plans (payment::plans, payment::subscription):
// own tables (workspace_plans, workspace_subscriptions), own Stripe metadata
// purpose ("workspace_plan", which subscription::sync_subscription skips), own
// customer portal configuration (no plan switching there, so an app plan and a
// workspace plan are never swapped for each other). Nothing here reads or
// writes `plans` or `subscriptions`.
//
// Spending (payment::tenant_billing): a workspace that bills itself
// (billing_mode 'tenant') and has a live plan spends the plan's allowance
// first, then overage when the plan prices it, else its prepaid balance. A
// workspace whose members pay from their own plan ('user') is not touched:
// that is the mayorana plans' path. Buying a workspace plan sets the mode to
// 'tenant', since the workspace is then the one paying.
//
// Seats: Stripe bills max(0, members − included_seats) of the seat price.
// reconcile_seats, every few minutes, moves that quantity to the member count
// now (prorated). Members are tenant_users with a member role (owner, admin,
// member); consumers of a published MCP server are not seats.
//
// Overage: each new period moves the ended period's overage_credits to
// overage_pending. When Stripe drafts the renewal invoice (invoice.created,
// billing_reason subscription_cycle) the pending overage is added to it as one
// line and cleared. Overage of a plan that ends is not billed by Stripe: the
// last period has no renewal invoice; the admin view shows it.
//
//   GET  /api/workspace-plans                   the plans on sale
//   GET  /api/workspace-plans/current?email=    the caller's active workspace: its
//                                               plan, seats, credits and overage
//   POST /api/workspace-plans/checkout          { email, plan_id, locale? } → { url }
//   POST /api/workspace-plans/change            { email, plan_id } → switch, prorated
//   POST /api/workspace-plans/portal            { email } → { url }
//   GET  /api/admin/workspace-plans             every plan, and every workspace subscription
//   PUT  /api/admin/workspace-plans/{id}        create or change a plan
//   POST /api/admin/workspace-plans/grant       { tenant_id, plan_id } → a custom plan, by hand
//   POST /api/admin/workspace-plans/revoke      { subscription_id } → ends a grant now
//
// All require X-Internal-Secret; the gateway restricts the admin routes to the
// super admin and checkout, change and portal to the workspace's owners and admins.

use crate::app_log;
use crate::endpoint_store::tenant_management::{get_default_tenant, MEMBER_ROLES};
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::service::PaymentService;
use actix_web::http::StatusCode;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use chrono::{DateTime, Months, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use stripe::Subscription;

/// Metadata `purpose` on workspace plan subscriptions, Checkout sessions,
/// products and prices.
pub const PURPOSE_WORKSPACE_PLAN: &str = "workspace_plan";

const CURRENCIES: &[&str] = &["usd", "chf", "eur"];
const LIVE_STATUSES: &[&str] = &["active", "trialing"];
const PORTAL_CONFIG_KEY: &str = "workspace_plans.portal_configuration";

/// What a workspace with priced overage may spend in one go, beyond its
/// allowance: the gateway refuses a call only when its cost is over what is
/// available, and overage is billed afterwards, so this is "no limit" with a
/// ceiling against a runaway loop.
pub const OVERAGE_HEADROOM: i64 = 1_000_000;

// ── Plans ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct WorkspacePlan {
    pub id: String,
    pub name: String,
    pub description: String,
    pub currency: String,
    pub price_cents: i64,
    pub included_seats: i32,
    pub seat_price_cents: i64,
    pub monthly_credits: i64,
    /// What 1,000 credits past the allowance cost; 0 means no overage.
    pub overage_cents_per_1k: i64,
    /// Enterprise: granted by hand, never on sale.
    pub custom: bool,
    pub stripe_product_id: Option<String>,
    pub stripe_price_id: Option<String>,
    pub stripe_seat_price_id: Option<String>,
    pub active: bool,
    pub sort_order: i32,
}

const PLAN_COLUMNS: &str = "id, name, description, currency, price_cents, included_seats, seat_price_cents, \
                            monthly_credits, overage_cents_per_1k, custom, stripe_product_id, stripe_price_id, \
                            stripe_seat_price_id, active, sort_order";

fn plan_from_row(r: &tokio_postgres::Row) -> WorkspacePlan {
    WorkspacePlan {
        id: r.get(0),
        name: r.get(1),
        description: r.get(2),
        currency: r.get(3),
        price_cents: r.get(4),
        included_seats: r.get(5),
        seat_price_cents: r.get(6),
        monthly_credits: r.get(7),
        overage_cents_per_1k: r.get(8),
        custom: r.get(9),
        stripe_product_id: r.get(10),
        stripe_price_id: r.get(11),
        stripe_seat_price_id: r.get(12),
        active: r.get(13),
        sort_order: r.get(14),
    }
}

impl WorkspacePlan {
    /// Can be bought with Checkout now.
    pub fn on_sale(&self) -> bool {
        self.active && !self.custom && self.stripe_price_id.is_some()
    }

    fn public(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "description": self.description,
            "currency": self.currency,
            "price_cents": self.price_cents,
            "included_seats": self.included_seats,
            "seat_price_cents": self.seat_price_cents,
            "monthly_credits": self.monthly_credits,
            "overage_cents_per_1k": self.overage_cents_per_1k,
            "custom": self.custom,
            "sort_order": self.sort_order,
        })
    }
}

pub async fn list_plans(store: &EndpointStore) -> Result<Vec<WorkspacePlan>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!("SELECT {} FROM workspace_plans ORDER BY sort_order, id", PLAN_COLUMNS);
    let rows = client.query(sql.as_str(), &[]).await.map_err(|e| e.to_string())?;
    Ok(rows.iter().map(plan_from_row).collect())
}

pub async fn get_plan(store: &EndpointStore, id: &str) -> Result<Option<WorkspacePlan>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!("SELECT {} FROM workspace_plans WHERE id = $1", PLAN_COLUMNS);
    let row = client.query_opt(sql.as_str(), &[&id]).await.map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(plan_from_row))
}

/// The plan whose base price is `price_id`, archived prices included as long
/// as the plan still points at it.
async fn plan_for_price(store: &EndpointStore, price_id: &str) -> Result<Option<WorkspacePlan>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!("SELECT {} FROM workspace_plans WHERE stripe_price_id = $1", PLAN_COLUMNS);
    let row = client.query_opt(sql.as_str(), &[&price_id]).await.map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(plan_from_row))
}

// ── Arithmetic (pure, tested) ────────────────────────────────────────────────

/// Seats Stripe bills past those the plan includes.
pub fn extra_seats(members: i64, included: i32) -> u64 {
    (members - included.max(0) as i64).max(0) as u64
}

/// What `credits` of overage cost, rounded up to the cent.
pub fn overage_cents(credits: i64, cents_per_1k: i64) -> i64 {
    if credits <= 0 || cents_per_1k <= 0 {
        return 0;
    }
    (credits * cents_per_1k + 999) / 1000
}

/// How a charge of `amount` splits when the plan has `left` this period:
/// (from the allowance, as overage, still to pay from the balance).
pub fn split_charge(left: i64, amount: i64, overage_priced: bool) -> (i64, i64, i64) {
    let from_plan = amount.min(left.max(0));
    let rest = amount - from_plan;
    if overage_priced {
        (from_plan, rest, 0)
    } else {
        (from_plan, 0, rest)
    }
}

// ── Subscriptions ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceSubscription {
    pub id: String,
    pub tenant_id: String,
    pub plan_id: String,
    pub status: String,
    pub live: bool,
    pub cancel_at_period_end: bool,
    pub period_start: DateTime<Utc>,
    pub period_end: DateTime<Utc>,
    pub credits_granted: i64,
    pub credits_used: i64,
    pub credits_left: i64,
    pub overage_credits: i64,
    pub overage_pending: i64,
    pub seats_billed: i32,
    /// Bought with Checkout; a grant (Enterprise) is not.
    pub on_stripe: bool,
}

const SUB_COLUMNS: &str = "id, tenant_id, plan_id, status, \
     (status = ANY($LIVE) AND period_end > NOW()) AS live, cancel_at_period_end, period_start, period_end, \
     credits_granted, credits_used, overage_credits, overage_pending, seats_billed, stripe_customer_id";

fn sub_columns() -> String {
    SUB_COLUMNS.replace("$LIVE", "ARRAY['active','trialing']")
}

fn sub_from_row(r: &tokio_postgres::Row) -> WorkspaceSubscription {
    let (granted, used): (i64, i64) = (r.get(8), r.get(9));
    let customer: String = r.get(13);
    WorkspaceSubscription {
        id: r.get(0),
        tenant_id: r.get(1),
        plan_id: r.get(2),
        status: r.get(3),
        live: r.get(4),
        cancel_at_period_end: r.get(5),
        period_start: r.get(6),
        period_end: r.get(7),
        credits_granted: granted,
        credits_used: used,
        credits_left: (granted - used).max(0),
        overage_credits: r.get(10),
        overage_pending: r.get(11),
        seats_billed: r.get(12),
        on_stripe: !customer.is_empty(),
    }
}

/// The subscription that counts for a workspace: a live one if any, else the
/// latest, so a lapsed workspace still sees what it had.
pub async fn current_subscription(store: &EndpointStore, tenant_id: &str) -> Result<Option<WorkspaceSubscription>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT {} FROM workspace_subscriptions WHERE tenant_id = $1 \
         ORDER BY live DESC, period_end DESC LIMIT 1",
        sub_columns()
    );
    let row = client.query_opt(sql.as_str(), &[&tenant_id]).await.map_err(|e| e.to_string())?;
    Ok(row.as_ref().map(sub_from_row))
}

/// The workspace's live plan and its subscription, if it has one.
pub async fn live_plan(store: &EndpointStore, tenant_id: &str) -> Result<Option<(WorkspaceSubscription, WorkspacePlan)>, String> {
    let Some(sub) = current_subscription(store, tenant_id).await?.filter(|s| s.live) else {
        return Ok(None);
    };
    let plan = get_plan(store, &sub.plan_id).await?.ok_or_else(|| format!("No workspace plan {}", sub.plan_id))?;
    Ok(Some((sub, plan)))
}

async fn member_count(store: &EndpointStore, tenant_id: &str) -> Result<i64, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let row = client
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM tenant_users WHERE tenant_id = $1 AND role = ANY($2)",
            &[&tenant_id, &MEMBER_ROLES],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.get(0))
}

// ── Spending ─────────────────────────────────────────────────────────────────

/// What a workspace with a live plan can spend now: the allowance left, plus
/// OVERAGE_HEADROOM when the plan prices overage, else plus its balance.
pub async fn available(store: &EndpointStore, tenant_id: &str) -> Result<Option<i64>, String> {
    let Some((sub, plan)) = live_plan(store, tenant_id).await? else {
        return Ok(None);
    };
    if plan.overage_cents_per_1k > 0 {
        return Ok(Some(sub.credits_left + OVERAGE_HEADROOM));
    }
    let balance = store.get_credit_balance(tenant_id).await.map_err(|e| e.to_string())?;
    Ok(Some(sub.credits_left + balance.max(0)))
}

#[derive(Debug, PartialEq)]
pub struct PlanCharge {
    pub from_plan: i64,
    pub overage: i64,
    /// Not covered by the plan: the caller takes it from the balance.
    pub rest: i64,
    pub subscription_id: String,
}

/// Takes up to `amount` from the workspace's live plan, overage included when
/// priced, in one statement so two charges at once cannot spend the same
/// credits. `None` when the workspace has no live plan.
pub async fn take(store: &EndpointStore, tenant_id: &str, amount: i64) -> Result<Option<PlanCharge>, String> {
    let Some((sub, plan)) = live_plan(store, tenant_id).await? else {
        return Ok(None);
    };
    let priced = plan.overage_cents_per_1k > 0;
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let row = client
        .query_opt(
            "WITH cur AS ( \
                 SELECT id, GREATEST(credits_granted - credits_used, 0) AS left_now \
                 FROM workspace_subscriptions WHERE id = $1 FOR UPDATE) \
             UPDATE workspace_subscriptions s SET \
                 credits_used = s.credits_used + LEAST(cur.left_now, $2), \
                 overage_credits = s.overage_credits + CASE WHEN $3 THEN GREATEST($2 - cur.left_now, 0) ELSE 0 END, \
                 updated_at = NOW() \
             FROM cur WHERE s.id = cur.id \
             RETURNING cur.left_now",
            &[&sub.id, &amount, &priced],
        )
        .await
        .map_err(|e| e.to_string())?;
    let Some(row) = row else { return Ok(None) };
    let (from_plan, overage, rest) = split_charge(row.get(0), amount, priced);
    Ok(Some(PlanCharge { from_plan, overage, rest, subscription_id: sub.id }))
}

/// Undoes a `take` whose rest the balance then could not pay.
pub async fn give_back(store: &EndpointStore, charge: &PlanCharge) -> Result<(), String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    client
        .execute(
            "UPDATE workspace_subscriptions SET credits_used = GREATEST(credits_used - $2, 0), \
                 overage_credits = GREATEST(overage_credits - $3, 0), updated_at = NOW() WHERE id = $1",
            &[&charge.subscription_id, &charge.from_plan, &charge.overage],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ── Keeping in step with Stripe ──────────────────────────────────────────────

fn timestamp(t: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(t, 0).unwrap_or_else(Utc::now)
}

/// Whether a Stripe subscription is a workspace plan's.
pub fn is_workspace_subscription(sub: &Subscription) -> bool {
    sub.metadata.get("purpose").map(String::as_str) == Some(PURPOSE_WORKSPACE_PLAN)
}

/// Writes `sub` as Stripe has it now. A later period start is a new month:
/// the ended month's overage waits for the renewal invoice, the allowance
/// starts afresh.
pub async fn sync_subscription(store: &Arc<EndpointStore>, sub: &Subscription) -> Result<(), String> {
    if !is_workspace_subscription(sub) {
        return Ok(());
    }
    let meta = &sub.metadata;
    let tenant_id = meta
        .get("tenant_id")
        .filter(|t| !t.is_empty())
        .ok_or_else(|| format!("Workspace subscription {} has no tenant_id", sub.id))?
        .clone();
    let bought_by = meta.get("email").cloned().unwrap_or_default();

    // The base price names the plan (a switch changes it); the seat item, if
    // any, is the plan's seat price.
    let prices: Vec<(String, u64)> = sub
        .items
        .data
        .iter()
        .filter_map(|i| Some((i.price.as_ref()?.id.to_string(), i.quantity.unwrap_or(0))))
        .collect();
    let mut plan = None;
    for (price, _) in &prices {
        if let Some(p) = plan_for_price(store, price).await? {
            plan = Some(p);
            break;
        }
    }
    if plan.is_none() {
        if let Some(id) = meta.get("plan_id") {
            plan = get_plan(store, id).await?;
        }
    }
    let plan = plan.ok_or_else(|| format!("Workspace subscription {} matches no plan", sub.id))?;
    let seats_billed = plan
        .stripe_seat_price_id
        .as_deref()
        .and_then(|seat| prices.iter().find(|(p, _)| p == seat))
        .map(|(_, q)| *q as i32)
        .unwrap_or(0);

    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    client
        .execute(
            "INSERT INTO workspace_subscriptions (id, tenant_id, plan_id, stripe_customer_id, status, cancel_at_period_end, \
                                                  period_start, period_end, credits_granted, seats_billed, bought_by) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
             ON CONFLICT (id) DO UPDATE SET \
                plan_id = EXCLUDED.plan_id, status = EXCLUDED.status, \
                stripe_customer_id = EXCLUDED.stripe_customer_id, \
                cancel_at_period_end = EXCLUDED.cancel_at_period_end, \
                seats_billed = EXCLUDED.seats_billed, \
                overage_pending = CASE WHEN EXCLUDED.period_start > workspace_subscriptions.period_start \
                                       THEN workspace_subscriptions.overage_pending + workspace_subscriptions.overage_credits \
                                       ELSE workspace_subscriptions.overage_pending END, \
                overage_credits = CASE WHEN EXCLUDED.period_start > workspace_subscriptions.period_start \
                                       THEN 0 ELSE workspace_subscriptions.overage_credits END, \
                credits_granted = CASE WHEN EXCLUDED.period_start > workspace_subscriptions.period_start \
                                       THEN EXCLUDED.credits_granted \
                                       ELSE GREATEST(workspace_subscriptions.credits_granted, EXCLUDED.credits_granted) END, \
                credits_used = CASE WHEN EXCLUDED.period_start > workspace_subscriptions.period_start \
                                    THEN 0 ELSE workspace_subscriptions.credits_used END, \
                period_start = GREATEST(workspace_subscriptions.period_start, EXCLUDED.period_start), \
                period_end = GREATEST(workspace_subscriptions.period_end, EXCLUDED.period_end), \
                updated_at = NOW()",
            &[
                &sub.id.to_string(),
                &tenant_id,
                &plan.id,
                &sub.customer.id().to_string(),
                &sub.status.as_str(),
                &sub.cancel_at_period_end,
                &timestamp(sub.current_period_start),
                &timestamp(sub.current_period_end),
                &plan.monthly_credits,
                &seats_billed,
                &bought_by,
            ],
        )
        .await
        .map_err(|e| e.to_string())?;

    // The workspace pays now: its keys spend its plan, not each person's.
    if LIVE_STATUSES.contains(&sub.status.as_str()) {
        let changed = client
            .execute(
                "UPDATE tenants SET billing_mode = 'tenant' WHERE id = $1 AND billing_mode <> 'tenant'",
                &[&tenant_id],
            )
            .await
            .map_err(|e| e.to_string())?;
        if changed > 0 {
            app_log!(info, tenant_id = %tenant_id, "Workspace plan live: the workspace now pays for its calls");
        }
    }
    app_log!(info, subscription_id = %sub.id, tenant_id = %tenant_id, plan = %plan.id, status = %sub.status.as_str(),
        seats_billed = seats_billed, "Workspace subscription synced");
    Ok(())
}

/// invoice.created: puts the ended periods' overage on a workspace's renewal
/// invoice while it is still a draft. Returns whether Stripe should retry.
pub async fn on_invoice_created(store: &Arc<EndpointStore>, stripe: &PaymentService, invoice: &serde_json::Value) -> bool {
    if invoice["billing_reason"].as_str() != Some("subscription_cycle") || invoice["status"].as_str() != Some("draft") {
        return false;
    }
    let (Some(sub_id), Some(invoice_id), Some(customer)) =
        (invoice["subscription"].as_str(), invoice["id"].as_str(), invoice["customer"].as_str())
    else {
        return false;
    };
    // The renewal may arrive before customer.subscription.updated: sync first,
    // so the month that ended has moved to overage_pending.
    let sub = match stripe.retrieve_subscription(sub_id).await {
        Ok(s) => s,
        Err(e) => {
            app_log!(error, error = %e, subscription_id = %sub_id, "Overage: could not fetch the subscription");
            return true;
        }
    };
    if !is_workspace_subscription(&sub) {
        return false;
    }
    if let Err(e) = sync_subscription(store, &sub).await {
        app_log!(error, error = %e, subscription_id = %sub_id, "Overage: could not sync the subscription");
        return true;
    }
    let taken = async {
        let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
        let row = client
            .query_opt(
                "WITH cur AS (SELECT id, overage_pending FROM workspace_subscriptions WHERE id = $1 FOR UPDATE) \
                 UPDATE workspace_subscriptions s SET overage_pending = 0, updated_at = NOW() \
                 FROM cur WHERE s.id = cur.id AND cur.overage_pending > 0 \
                 RETURNING cur.overage_pending, s.plan_id",
                &[&sub_id],
            )
            .await
            .map_err(|e| e.to_string())?;
        Ok::<_, String>(row.map(|r| (r.get::<_, i64>(0), r.get::<_, String>(1))))
    }
    .await;
    let (credits, plan_id) = match taken {
        Ok(Some(t)) => t,
        Ok(None) => return false,
        Err(e) => {
            app_log!(error, error = %e, subscription_id = %sub_id, "Overage: could not read it");
            return true;
        }
    };
    let plan = match get_plan(store, &plan_id).await {
        Ok(Some(p)) => p,
        _ => {
            app_log!(error, subscription_id = %sub_id, plan = %plan_id, "Overage: the plan is gone; not billed");
            return false;
        }
    };
    let cents = overage_cents(credits, plan.overage_cents_per_1k);
    if cents <= 0 {
        return false;
    }
    let currency = invoice["currency"].as_str().unwrap_or(&plan.currency);
    let description = format!("Overage: {} credits past the {} allowance", credits, plan.name);
    match stripe.add_invoice_item(customer, invoice_id, cents, currency, &description).await {
        Ok(()) => {
            app_log!(info, subscription_id = %sub_id, invoice_id = %invoice_id, credits = credits, cents = cents, "Overage added to the renewal invoice");
            false
        }
        Err(e) => {
            // Put it back, so a retry bills it.
            app_log!(error, error = %e, subscription_id = %sub_id, credits = credits, "Overage: Stripe refused the invoice line");
            if let Ok(client) = store.get_admin_conn().await {
                let _ = client
                    .execute(
                        "UPDATE workspace_subscriptions SET overage_pending = overage_pending + $2 WHERE id = $1",
                        &[&sub_id, &credits],
                    )
                    .await;
            }
            true
        }
    }
}

/// Moves each live Stripe workspace subscription's seat quantity to its
/// member count now, prorated; and starts the next month of each grant whose
/// month is over. Run every few minutes.
pub async fn reconcile(store: &Arc<EndpointStore>, stripe: &PaymentService) {
    let rows = async {
        let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
        // Grants: a month at a time until revoked. Their overage is invoiced by
        // hand, so it moves to pending like a Stripe renewal's.
        client
            .execute(
                "UPDATE workspace_subscriptions s SET \
                     overage_pending = s.overage_pending + s.overage_credits, overage_credits = 0, credits_used = 0, \
                     credits_granted = p.monthly_credits, period_start = s.period_end, \
                     period_end = s.period_end + INTERVAL '1 month', updated_at = NOW() \
                 FROM workspace_plans p \
                 WHERE p.id = s.plan_id AND s.stripe_customer_id = '' AND s.status = 'active' \
                   AND NOT s.cancel_at_period_end AND s.period_end <= NOW()",
                &[],
            )
            .await
            .map_err(|e| e.to_string())?;
        client
            .query(
                "SELECT s.id, s.tenant_id, s.seats_billed, p.included_seats, p.stripe_seat_price_id \
                 FROM workspace_subscriptions s JOIN workspace_plans p ON p.id = s.plan_id \
                 WHERE s.stripe_customer_id <> '' AND s.status IN ('active', 'trialing') AND s.period_end > NOW() \
                   AND p.stripe_seat_price_id IS NOT NULL",
                &[],
            )
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, "Workspace plans: could not read the subscriptions to reconcile");
            return;
        }
    };
    for row in rows {
        let (sub_id, tenant_id, billed, included, seat_price): (String, String, i32, i32, String) =
            (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4));
        let members = match member_count(store, &tenant_id).await {
            Ok(n) => n,
            Err(e) => {
                app_log!(warn, error = %e, tenant_id = %tenant_id, "Workspace plans: could not count members");
                continue;
            }
        };
        let wanted = extra_seats(members, included);
        if wanted == billed.max(0) as u64 {
            continue;
        }
        match stripe.set_subscription_items(&sub_id, &[(seat_price, wanted)], &[]).await {
            Ok(sub) => {
                app_log!(info, subscription_id = %sub_id, tenant_id = %tenant_id, members = members, seats_billed = wanted,
                    "Workspace seats billed changed");
                if let Err(e) = sync_subscription(store, &sub).await {
                    app_log!(warn, error = %e, subscription_id = %sub_id, "Workspace plans: could not sync after a seat change");
                }
            }
            Err(e) => app_log!(error, error = %e, subscription_id = %sub_id, "Workspace plans: Stripe refused the seat change"),
        }
    }
}

async fn portal_configuration(store: &EndpointStore, stripe: &PaymentService) -> Option<String> {
    let client = store.get_admin_conn().await.ok()?;
    let existing: Option<String> = client
        .query_opt("SELECT value FROM system_config WHERE key = $1", &[&PORTAL_CONFIG_KEY])
        .await
        .ok()?
        .map(|r| r.get::<_, String>(0))
        .filter(|v| !v.is_empty());
    if existing.is_some() {
        return existing;
    }
    let made = match stripe.save_workspace_portal_configuration(None).await {
        Ok(id) => id,
        Err(e) => {
            app_log!(error, error = %e, "Could not create the workspace plans' portal configuration");
            return None;
        }
    };
    let _ = client
        .execute(
            "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW()) \
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
            &[&PORTAL_CONFIG_KEY, &made],
        )
        .await;
    Some(made)
}

// ── Admin: saving a plan ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct PlanInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub currency: String,
    #[serde(default)]
    pub price_cents: i64,
    pub included_seats: i32,
    #[serde(default)]
    pub seat_price_cents: i64,
    pub monthly_credits: i64,
    #[serde(default)]
    pub overage_cents_per_1k: i64,
    #[serde(default)]
    pub custom: bool,
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
        if !self.custom && self.price_cents < 50 {
            return Err("A plan on sale costs at least 0.50 a month".into());
        }
        if self.price_cents < 0 || self.seat_price_cents < 0 || self.overage_cents_per_1k < 0 {
            return Err("Prices cannot be negative".into());
        }
        if self.seat_price_cents != 0 && self.seat_price_cents < 50 {
            return Err("The seat price must be 0 (no extra seats billed) or at least 0.50".into());
        }
        if self.included_seats < 1 {
            return Err("A plan includes at least one seat".into());
        }
        if self.monthly_credits < 0 {
            return Err("Monthly credits cannot be negative".into());
        }
        Ok(())
    }
}

async fn save_plan(store: &EndpointStore, stripe: &PaymentService, id: &str, input: PlanInput) -> Result<WorkspacePlan, String> {
    let existing = get_plan(store, id).await?;
    let mut product_id = existing.as_ref().and_then(|p| p.stripe_product_id.clone());
    let mut price_id = existing.as_ref().and_then(|p| p.stripe_price_id.clone());
    let mut seat_price_id = existing.as_ref().and_then(|p| p.stripe_seat_price_id.clone());
    let metadata = HashMap::from([
        ("purpose".to_string(), PURPOSE_WORKSPACE_PLAN.to_string()),
        ("plan_id".to_string(), id.to_string()),
    ]);

    // A custom plan is never sold: no Stripe prices.
    if input.custom {
        for old in [price_id.take(), seat_price_id.take()].into_iter().flatten() {
            if let Err(e) = stripe.archive_price(&old).await {
                app_log!(warn, error = %e, price_id = %old, "Could not archive a workspace plan's Stripe price");
            }
        }
    } else if input.active {
        let base_changed = existing
            .as_ref()
            .map(|p| p.price_cents != input.price_cents || p.currency != input.currency)
            .unwrap_or(true);
        if base_changed || price_id.is_none() {
            let (product, price) = stripe
                .create_plan_price(product_id.as_deref(), &input.name, input.price_cents, &input.currency, metadata.clone())
                .await
                .map_err(|e| format!("Stripe refused the plan price: {}", e))?;
            if let Some(old) = price_id.as_deref() {
                if let Err(e) = stripe.archive_price(old).await {
                    app_log!(warn, error = %e, price_id = %old, "Could not archive a workspace plan's previous price");
                }
            }
            product_id = Some(product);
            price_id = Some(price);
        }
        let seat_changed = existing
            .as_ref()
            .map(|p| p.seat_price_cents != input.seat_price_cents || p.currency != input.currency)
            .unwrap_or(true);
        if input.seat_price_cents == 0 {
            if let Some(old) = seat_price_id.take() {
                if let Err(e) = stripe.archive_price(&old).await {
                    app_log!(warn, error = %e, price_id = %old, "Could not archive a workspace plan's seat price");
                }
            }
        } else if seat_changed || seat_price_id.is_none() {
            let mut seat_meta = metadata.clone();
            seat_meta.insert("seat".to_string(), "true".to_string());
            let (_, seat) = stripe
                .create_plan_price(product_id.as_deref(), &input.name, input.seat_price_cents, &input.currency, seat_meta)
                .await
                .map_err(|e| format!("Stripe refused the seat price: {}", e))?;
            // Subscriptions on the old seat price keep it: a seat change moves
            // only the quantity of the price they have (reconcile).
            seat_price_id = Some(seat);
        }
    }
    if let (Some(product), Some(old)) = (product_id.as_deref(), existing.as_ref()) {
        if old.name != input.name {
            if let Err(e) = stripe.rename_product(product, &input.name).await {
                app_log!(warn, error = %e, product_id = %product, "Could not rename a workspace plan's product");
            }
        }
    }

    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!(
        "INSERT INTO workspace_plans (id, name, description, currency, price_cents, included_seats, seat_price_cents, \
                                      monthly_credits, overage_cents_per_1k, custom, stripe_product_id, stripe_price_id, \
                                      stripe_seat_price_id, active, sort_order, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, NOW()) \
         ON CONFLICT (id) DO UPDATE SET \
            name = EXCLUDED.name, description = EXCLUDED.description, currency = EXCLUDED.currency, \
            price_cents = EXCLUDED.price_cents, included_seats = EXCLUDED.included_seats, \
            seat_price_cents = EXCLUDED.seat_price_cents, monthly_credits = EXCLUDED.monthly_credits, \
            overage_cents_per_1k = EXCLUDED.overage_cents_per_1k, custom = EXCLUDED.custom, \
            stripe_product_id = EXCLUDED.stripe_product_id, stripe_price_id = EXCLUDED.stripe_price_id, \
            stripe_seat_price_id = EXCLUDED.stripe_seat_price_id, active = EXCLUDED.active, \
            sort_order = EXCLUDED.sort_order, updated_at = NOW() \
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
                &input.currency,
                &input.price_cents,
                &input.included_seats,
                &input.seat_price_cents,
                &input.monthly_credits,
                &input.overage_cents_per_1k,
                &input.custom,
                &product_id,
                &price_id,
                &seat_price_id,
                &input.active,
                &input.sort_order,
            ],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(plan_from_row(&row))
}

// ── HTTP ─────────────────────────────────────────────────────────────────────

fn fail(status: StatusCode, message: &str) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({ "success": false, "message": message }))
}

fn internal(what: &str, e: String) -> HttpResponse {
    app_log!(error, error = %e, "Workspace plans: {}", what);
    fail(StatusCode::INTERNAL_SERVER_ERROR, &format!("Could not {}", what))
}

/// GET /api/workspace-plans
pub async fn public_plans_handler(req: HttpRequest, store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match list_plans(&store).await {
        Ok(plans) => {
            let plans: Vec<_> = plans.iter().filter(|p| p.on_sale()).map(WorkspacePlan::public).collect();
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "plans": plans }))
        }
        Err(e) => internal("read the plans", e),
    }
}

#[derive(Deserialize)]
pub struct EmailQuery {
    pub email: String,
}

/// The caller's active workspace, and whether they may buy or change its plan.
async fn workspace_of(store: &EndpointStore, email: &str) -> Result<(String, String, i64, bool), String> {
    let tenant = get_default_tenant(store, email).await.map_err(|e| e.to_string())?;
    let can_edit = crate::payment::tenant_billing::can_edit(store, &tenant.id, email).await?;
    Ok((tenant.id, tenant.name, tenant.credit_balance, can_edit))
}

/// GET /api/workspace-plans/current?email=
pub async fn current_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let (tenant_id, tenant_name, balance, can_edit) = match workspace_of(&store, &query.email).await {
        Ok(w) => w,
        Err(e) => return internal("find the workspace", e),
    };
    let members = match member_count(&store, &tenant_id).await {
        Ok(n) => n,
        Err(e) => return internal("count the members", e),
    };
    let sub = match current_subscription(&store, &tenant_id).await {
        Ok(s) => s,
        Err(e) => return internal("read the subscription", e),
    };
    let plan = match &sub {
        Some(s) => match get_plan(&store, &s.plan_id).await {
            Ok(p) => p,
            Err(e) => return internal("read the plan", e),
        },
        None => None,
    };
    let view = match (&sub, &plan) {
        (Some(s), Some(p)) => serde_json::json!({
            "id": s.id,
            "status": s.status,
            "live": s.live,
            "cancel_at_period_end": s.cancel_at_period_end,
            "period_start": s.period_start,
            "period_end": s.period_end,
            "credits_granted": s.credits_granted,
            "credits_used": s.credits_used,
            "credits_left": s.credits_left,
            "overage_credits": s.overage_credits,
            "overage_cents": overage_cents(s.overage_credits, p.overage_cents_per_1k),
            "seats_included": p.included_seats,
            "seats_billed": s.seats_billed,
            "on_stripe": s.on_stripe,
            "plan": p.public(),
        }),
        _ => serde_json::Value::Null,
    };
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tenant_id": tenant_id,
        "tenant_name": tenant_name,
        "can_edit": can_edit,
        "members": members,
        "balance": balance,
        "subscription": view,
    }))
}

#[derive(Deserialize)]
pub struct CheckoutRequest {
    pub email: String,
    pub plan_id: String,
    #[serde(default)]
    pub locale: Option<String>,
}

fn dashboard_url() -> String {
    std::env::var("DASHBOARD_URL")
        .ok()
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://app.api0.ai".to_string())
}

/// The Stripe customer a workspace paid with before, so it keeps one.
async fn earlier_customer(store: &EndpointStore, tenant_id: &str) -> Option<String> {
    let client = store.get_admin_conn().await.ok()?;
    let row = client
        .query_opt(
            "SELECT stripe_customer_id FROM workspace_subscriptions \
             WHERE tenant_id = $1 AND stripe_customer_id <> '' ORDER BY created_at DESC LIMIT 1",
            &[&tenant_id],
        )
        .await
        .ok()??;
    Some(row.get(0))
}

/// POST /api/workspace-plans/checkout
pub async fn checkout_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    stripe: web::Data<Arc<PaymentService>>,
    body: web::Json<CheckoutRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let email = body.email.trim().to_lowercase();
    let (tenant_id, _, _, can_edit) = match workspace_of(&store, &email).await {
        Ok(w) => w,
        Err(e) => return internal("find the workspace", e),
    };
    if !can_edit {
        return fail(StatusCode::FORBIDDEN, "Only the workspace's owners and admins can buy a plan");
    }
    let plan = match get_plan(&store, &body.plan_id).await {
        Ok(Some(p)) if p.on_sale() => p,
        Ok(_) => return fail(StatusCode::NOT_FOUND, "Unknown plan"),
        Err(e) => return internal("read the plan", e),
    };
    match current_subscription(&store, &tenant_id).await {
        Ok(Some(s)) if s.live => {
            return fail(StatusCode::CONFLICT, "This workspace already has a plan — change it instead");
        }
        Ok(_) => {}
        Err(e) => return internal("read the subscription", e),
    }
    let members = match member_count(&store, &tenant_id).await {
        Ok(n) => n,
        Err(e) => return internal("count the members", e),
    };
    let mut lines = vec![(plan.stripe_price_id.clone().unwrap_or_default(), 1)];
    if let Some(seat) = plan.stripe_seat_price_id.clone() {
        lines.push((seat, extra_seats(members, plan.included_seats)));
    }
    let metadata = HashMap::from([
        ("purpose".to_string(), PURPOSE_WORKSPACE_PLAN.to_string()),
        ("plan_id".to_string(), plan.id.clone()),
        ("tenant_id".to_string(), tenant_id.clone()),
        ("email".to_string(), email.clone()),
        ("locale".to_string(), body.locale.clone().unwrap_or_default()),
    ]);
    let back = format!("{}/?view=settings&tab=workspace-plan", dashboard_url());
    let success = format!("{}&checkout=success", back);
    let customer = earlier_customer(&store, &tenant_id).await;
    match stripe.create_workspace_checkout(&lines, customer.as_deref(), &email, &success, &back, metadata).await {
        Ok(session) => {
            app_log!(info, tenant_id = %tenant_id, plan = %plan.id, members = members, "Workspace plan checkout started");
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "url": session.url }))
        }
        Err(e) => internal("start the checkout", e.to_string()),
    }
}

/// POST /api/workspace-plans/change — a switch moves the base price and the
/// seat price together, prorated.
pub async fn change_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    stripe: web::Data<Arc<PaymentService>>,
    body: web::Json<CheckoutRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let email = body.email.trim().to_lowercase();
    let (tenant_id, _, _, can_edit) = match workspace_of(&store, &email).await {
        Ok(w) => w,
        Err(e) => return internal("find the workspace", e),
    };
    if !can_edit {
        return fail(StatusCode::FORBIDDEN, "Only the workspace's owners and admins can change the plan");
    }
    let (sub, old) = match live_plan(&store, &tenant_id).await {
        Ok(Some((s, p))) if s.on_stripe => (s, p),
        Ok(Some(_)) => return fail(StatusCode::CONFLICT, "This plan was set up by api0; contact us to change it"),
        Ok(None) => return fail(StatusCode::NOT_FOUND, "This workspace has no plan to change"),
        Err(e) => return internal("read the plan", e),
    };
    let new = match get_plan(&store, &body.plan_id).await {
        Ok(Some(p)) if p.on_sale() && p.id != old.id => p,
        Ok(_) => return fail(StatusCode::NOT_FOUND, "Unknown plan"),
        Err(e) => return internal("read the plan", e),
    };
    if new.currency != old.currency {
        return fail(StatusCode::BAD_REQUEST, "That plan is in another currency; cancel this one and buy it instead");
    }
    let members = match member_count(&store, &tenant_id).await {
        Ok(n) => n,
        Err(e) => return internal("count the members", e),
    };
    let mut wanted = vec![(new.stripe_price_id.clone().unwrap_or_default(), 1)];
    if let Some(seat) = new.stripe_seat_price_id.clone() {
        wanted.push((seat, extra_seats(members, new.included_seats)));
    }
    let drop: Vec<String> = [old.stripe_price_id.clone(), old.stripe_seat_price_id.clone()].into_iter().flatten().collect();
    match stripe.set_subscription_items(&sub.id, &wanted, &drop).await {
        Ok(updated) => {
            if let Err(e) = sync_subscription(&store, &updated).await {
                return internal("save the new plan", e);
            }
            app_log!(info, tenant_id = %tenant_id, from = %old.id, to = %new.id, "Workspace plan changed");
            HttpResponse::Ok().json(serde_json::json!({ "success": true }))
        }
        Err(e) => internal("change the plan on Stripe", e.to_string()),
    }
}

/// POST /api/workspace-plans/portal
pub async fn portal_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    stripe: web::Data<Arc<PaymentService>>,
    body: web::Json<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let (tenant_id, _, _, can_edit) = match workspace_of(&store, &body.email).await {
        Ok(w) => w,
        Err(e) => return internal("find the workspace", e),
    };
    if !can_edit {
        return fail(StatusCode::FORBIDDEN, "Only the workspace's owners and admins can manage billing");
    }
    let Some(customer) = earlier_customer(&store, &tenant_id).await else {
        return fail(StatusCode::NOT_FOUND, "This workspace has never paid with Stripe");
    };
    let config = portal_configuration(&store, &stripe).await;
    let back = format!("{}/?view=settings&tab=workspace-plan", dashboard_url());
    match stripe.create_portal_session(&customer, &back, config.as_deref()).await {
        Ok(url) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "url": url })),
        Err(e) => internal("open the billing portal", e.to_string()),
    }
}

/// GET /api/admin/workspace-plans
pub async fn admin_handler(req: HttpRequest, store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let plans = match list_plans(&store).await {
        Ok(p) => p,
        Err(e) => return internal("read the plans", e),
    };
    let subs = async {
        let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
        let sql = format!(
            "SELECT {}, (SELECT name FROM tenants t WHERE t.id = s.tenant_id), s.bought_by \
             FROM workspace_subscriptions s ORDER BY s.updated_at DESC LIMIT 500",
            sub_columns()
        );
        client.query(sql.as_str(), &[]).await.map_err(|e| e.to_string())
    }
    .await;
    let subs = match subs {
        Ok(rows) => rows
            .iter()
            .map(|r| {
                let mut v = serde_json::to_value(sub_from_row(r)).unwrap_or_default();
                v["tenant_name"] = serde_json::json!(r.get::<_, Option<String>>(14));
                v["bought_by"] = serde_json::json!(r.get::<_, String>(15));
                v
            })
            .collect::<Vec<_>>(),
        Err(e) => return internal("read the subscriptions", e),
    };
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "plans": plans,
        "subscriptions": subs,
        "currencies": CURRENCIES,
    }))
}

/// PUT /api/admin/workspace-plans/{id}
pub async fn save_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    stripe: web::Data<Arc<PaymentService>>,
    id: web::Path<String>,
    body: web::Json<PlanInput>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let id = id.into_inner();
    let mut input = body.into_inner();
    if let Err(why) = input.check(&id) {
        return fail(StatusCode::BAD_REQUEST, &why);
    }
    match save_plan(&store, &stripe, &id, input).await {
        Ok(plan) => {
            app_log!(info, plan = %id, "Workspace plan saved");
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "plan": plan }))
        }
        Err(e) => internal("save the plan", e),
    }
}

#[derive(Deserialize)]
pub struct GrantRequest {
    pub tenant_id: String,
    pub plan_id: String,
}

/// POST /api/admin/workspace-plans/grant — Enterprise, or any plan given by
/// hand: a month at a time until revoked, invoiced outside Stripe.
pub async fn grant_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<GrantRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let plan = match get_plan(&store, &body.plan_id).await {
        Ok(Some(p)) => p,
        Ok(None) => return fail(StatusCode::NOT_FOUND, "Unknown plan"),
        Err(e) => return internal("read the plan", e),
    };
    match current_subscription(&store, &body.tenant_id).await {
        Ok(Some(s)) if s.live => return fail(StatusCode::CONFLICT, "This workspace already has a live plan"),
        Ok(_) => {}
        Err(e) => return internal("read the subscription", e),
    }
    let now = Utc::now();
    let end = now.checked_add_months(Months::new(1)).unwrap_or(now);
    let id = format!("grant_{}", uuid::Uuid::new_v4().simple());
    let result = async {
        let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
        client
            .execute(
                "INSERT INTO workspace_subscriptions (id, tenant_id, plan_id, status, period_start, period_end, credits_granted) \
                 VALUES ($1, $2, $3, 'active', $4, $5, $6)",
                &[&id, &body.tenant_id, &plan.id, &now, &end, &plan.monthly_credits],
            )
            .await
            .map_err(|e| e.to_string())?;
        client
            .execute("UPDATE tenants SET billing_mode = 'tenant' WHERE id = $1", &[&body.tenant_id])
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    match result {
        Ok(_) => {
            app_log!(info, tenant_id = %body.tenant_id, plan = %plan.id, subscription_id = %id, "Workspace plan granted");
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "subscription_id": id }))
        }
        Err(e) => internal("grant the plan", e),
    }
}

#[derive(Deserialize)]
pub struct RevokeRequest {
    pub subscription_id: String,
}

/// POST /api/admin/workspace-plans/revoke — ends a granted plan now. A
/// Stripe plan is cancelled by its workspace, on the portal.
pub async fn revoke_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<RevokeRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let result = async {
        let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
        client
            .execute(
                "UPDATE workspace_subscriptions SET status = 'canceled', period_end = LEAST(period_end, NOW()), updated_at = NOW() \
                 WHERE id = $1 AND stripe_customer_id = ''",
                &[&body.subscription_id],
            )
            .await
            .map_err(|e| e.to_string())
    }
    .await;
    match result {
        Ok(0) => fail(StatusCode::NOT_FOUND, "No granted plan with that id"),
        Ok(_) => {
            app_log!(info, subscription_id = %body.subscription_id, "Workspace plan grant revoked");
            HttpResponse::Ok().json(serde_json::json!({ "success": true }))
        }
        Err(e) => internal("revoke the grant", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seats_past_those_included_are_billed() {
        assert_eq!(extra_seats(0, 3), 0);
        assert_eq!(extra_seats(3, 3), 0);
        assert_eq!(extra_seats(5, 3), 2);
        assert_eq!(extra_seats(1, 0), 1);
    }

    #[test]
    fn overage_is_rounded_up_to_the_cent() {
        assert_eq!(overage_cents(0, 500), 0);
        assert_eq!(overage_cents(1000, 500), 500);
        assert_eq!(overage_cents(1, 500), 1);
        assert_eq!(overage_cents(1500, 333), 500);
        assert_eq!(overage_cents(10_000, 0), 0);
    }

    #[test]
    fn a_charge_spends_the_allowance_then_overage_or_the_balance() {
        assert_eq!(split_charge(100, 30, true), (30, 0, 0));
        assert_eq!(split_charge(10, 30, true), (10, 20, 0));
        assert_eq!(split_charge(10, 30, false), (10, 0, 20));
        assert_eq!(split_charge(0, 30, true), (0, 30, 0));
        assert_eq!(split_charge(-5, 30, false), (0, 0, 30));
    }

    fn input() -> PlanInput {
        PlanInput {
            name: " Growth ".into(),
            description: String::new(),
            currency: "USD".into(),
            price_cents: 49_900,
            included_seats: 10,
            seat_price_cents: 3_900,
            monthly_credits: 100_000,
            overage_cents_per_1k: 300,
            custom: false,
            active: true,
            sort_order: 2,
        }
    }

    #[test]
    fn a_plan_is_checked() {
        let mut p = input();
        assert!(p.check("growth").is_ok());
        assert_eq!((p.name.as_str(), p.currency.as_str()), ("Growth", "usd"));
        assert!(input().check("Growth Plan").is_err());
        assert!(PlanInput { price_cents: 0, ..input() }.check("x").is_err());
        assert!(PlanInput { price_cents: 0, custom: true, ..input() }.check("enterprise").is_ok());
        assert!(PlanInput { included_seats: 0, ..input() }.check("x").is_err());
        assert!(PlanInput { seat_price_cents: 10, ..input() }.check("x").is_err());
        assert!(PlanInput { seat_price_cents: 0, ..input() }.check("x").is_ok());
        assert!(PlanInput { currency: "gbp".into(), ..input() }.check("x").is_err());
    }
}
