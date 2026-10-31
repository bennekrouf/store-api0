// src/payment/llm_wallet.rs
//
// The store's half of the LLM provider: a tenant (LLM_PROVIDER_TENANT_ID) that
// serves chat completions to the desktop apps through the gateway's
// /v1/chat/completions. An app sends the licence number of a plan
// subscription, and calls are paid from that subscription's monthly credits
// (payment::subscription).
//
//   POST /api/internal/llm/authorize  { licence, model?, app?, task? }
//        → whose licence it is, the credits left, the regions on offer (with
//          what a credit buys on each) and, when one matches, the upstream
//          provider to forward to (with its key) and the name the app may see.
//          For the task "transcribe" (llm_models::SPEECH, the gateway's
//          /v1/audio/transcriptions) only the region's speech model is an
//          upstream, with its `transcription_url`; a region without one has none
//
// An app names a region ("europe", admin/llm_regions.rs) and is only ever told
// the region back, whichever model its task runs on (llm_regions::task_of).
// "default" (or the tenant's name) is the platform's choice;
// a model named directly still works for older apps, but is never listed.
//   POST /api/internal/llm/charge     one completion's tokens
//        → { credits, balance }: spends the subscription's credits and records
//          the call under the provider tenant in llm_usage
//   GET  /api/plans/usage/{email}?limit=N&offset=M
//        → { usage: [...], total }: a page of the email's calls, newest first,
//          and how many there are, for the dashboard (the gateway puts the
//          signed-in email in)
//
// api0's own AI calls (YAML import) are recorded in llm_usage too, by
// record_platform_call, with no licence and no credits.
//
// All need X-Internal-Secret. The upstream keys are the platform ones the
// super admin sets in Admin → Platform (assistant_config).

use crate::admin::assistant_config::{self, PROVIDERS};
use crate::admin::llm_models::{self, SPEECH};
use crate::admin::llm_regions::{self, Backing, Region};
use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::subscription::{self, SpendError};
use actix_web::{http::StatusCode, web, HttpRequest, HttpResponse, Responder};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The (provider, model) a request names, among those on offer. An empty name,
/// "default", or the provider tenant's own name means the platform's choice.
pub fn pick_model<'a>(
    requested: &str,
    tenant_name: &str,
    default: Option<(&'a str, &'a str)>,
    offered: &[(&'a str, &'a str)],
) -> Option<(&'a str, &'a str)> {
    let r = requested.trim();
    if r.is_empty() || r.eq_ignore_ascii_case("default") || r.eq_ignore_ascii_case(tenant_name.trim()) {
        return default
            .filter(|(p, _)| offered.iter().any(|(op, _)| op == p))
            .or_else(|| offered.first().copied());
    }
    offered.iter().find(|(_, m)| *m == r).copied()
}

/// The region a request names, among those live.
pub(crate) fn pick_region<'a>(
    requested: &str,
    live: &[(&'static Region, &'a Backing)],
) -> Option<(&'static Region, &'a Backing)> {
    live.iter().find(|(r, _)| r.id.eq_ignore_ascii_case(requested.trim())).copied()
}

/// The name an answer carries back to the app: the region, "default", or the
/// model it named itself — never a model it did not name.
fn public_name(requested: &str, region: Option<&Region>, tenant_name: &str) -> String {
    let r = requested.trim();
    match region {
        Some(region) => region.id.to_string(),
        None if r.is_empty() || r.eq_ignore_ascii_case("default") || r.eq_ignore_ascii_case(tenant_name.trim()) => {
            "default".to_string()
        }
        None => r.to_string(),
    }
}

fn fail(status: StatusCode, msg: impl Into<String>) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({ "success": false, "error": msg.into() }))
}

fn provider_tenant() -> Result<String, HttpResponse> {
    match std::env::var("LLM_PROVIDER_TENANT_ID") {
        Ok(t) if !t.trim().is_empty() => Ok(t.trim().to_string()),
        _ => {
            app_log!(error, "LLM_PROVIDER_TENANT_ID is not set — the LLM endpoint is off");
            Err(fail(StatusCode::SERVICE_UNAVAILABLE, "The LLM service is not configured"))
        }
    }
}

// ── POST /api/internal/llm/authorize ─────────────────────────────────────────

#[derive(Deserialize)]
pub struct AuthorizeRequest {
    pub licence: String,
    #[serde(default)]
    pub model: Option<String>,
    /// The app calling, for the usage log ("gitagent"); free text.
    #[serde(default)]
    pub app: Option<String>,
    /// What the call does (llm_models::TASKS), from X-Task; else the app's.
    #[serde(default)]
    pub task: Option<String>,
}

/// An app id for the usage log: short, plain, never empty.
fn app_label(app: Option<&str>) -> String {
    let a: String = app
        .unwrap_or_default()
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(40)
        .collect();
    if a.is_empty() { "unknown".into() } else { a }
}

pub async fn authorize_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<AuthorizeRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant_id = match provider_tenant() {
        Ok(t) => t,
        Err(r) => return r,
    };
    let who = match subscription::licence_status(&store, &body.licence).await {
        Ok(Some(l)) => l,
        Ok(None) => return fail(StatusCode::UNAUTHORIZED, "Unknown licence"),
        Err(e) => {
            app_log!(error, error = %e, "Could not read a licence");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not check the licence");
        }
    };

    let tenant_name = match store.get_admin_conn().await {
        Ok(c) => match c.query_opt("SELECT name FROM tenants WHERE id = $1", &[&tenant_id]).await {
            Ok(Some(r)) => r.get::<_, String>(0),
            Ok(None) => {
                app_log!(error, tenant_id = %tenant_id, "LLM_PROVIDER_TENANT_ID names no tenant");
                return fail(StatusCode::SERVICE_UNAVAILABLE, "The LLM service is not configured");
            }
            Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        },
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let keyed = match assistant_config::providers_with_key(&store).await {
        Ok(k) => k,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let offered: Vec<(&str, &str)> = PROVIDERS
        .iter()
        .filter(|p| keyed.iter().any(|k| k == p.id))
        .flat_map(|p| p.models.iter().map(move |m| (p.id, *m)))
        .collect();
    let regions = match llm_regions::read_regions(&store).await {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, "Could not read the LLM regions");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not read the regions");
        }
    };
    let live = llm_regions::live(&regions, &keyed);
    // What the app is told a credit buys must be what the charge spends.
    let pricing = match crate::payment::plans::effective_pricing(&store).await {
        Ok(p) => p,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e),
    };

    let app = app_label(body.app.as_deref());
    let task = llm_regions::task_of(body.task.as_deref(), &app);
    let requested = body.model.as_deref().unwrap_or_default();
    let region = pick_region(requested, &live);
    let choice = match region {
        Some(_) => None,
        None => assistant_config::read_choice(&store).await.ok().flatten(),
    };
    let speech = task == Some(SPEECH);
    let picked: Option<(&str, &str)> = match region {
        Some((_, b)) if speech => b.speech(&keyed),
        Some((_, b)) => Some(b.for_task(task, &keyed)),
        // Only a region's speech model transcribes.
        None if speech => None,
        None => {
            let default = choice.as_ref().map(|(p, m)| (p.as_str(), m.as_str()));
            pick_model(requested, &tenant_name, default, &offered)
        }
    };

    let upstream = match picked.and_then(|(p, m)| assistant_config::provider(p).map(|prov| (prov, m))) {
        Some((prov, model)) => match assistant_config::api_key(&store, prov.id).await {
            Ok(Some(key)) => serde_json::json!({
                "provider": prov.id, "base_url": prov.base_url, "chat_url": prov.chat_url(),
                "transcription_url": prov.transcription_url(),
                "model": model, "api_key": key,
                "region": region.map(|(r, _)| r.id),
                "task": task,
                "public_model": public_name(requested, region.map(|(r, _)| r), &tenant_name),
                "sampling": llm_models::sampling(model),
            }),
            Ok(None) => serde_json::Value::Null,
            Err(e) => {
                app_log!(error, error = %e, "Could not read an upstream LLM key");
                return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not read the provider key");
            }
        },
        None => serde_json::Value::Null,
    };

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "subscription_id": who.subscription_id,
        "email": who.email,
        "app": app,
        "tenant_id": tenant_id,
        "tenant_name": tenant_name,
        "plan": who.active,
        "plan_name": who.plan_name,
        "balance": if who.active { who.credits_left } else { 0 },
        // What the app may list: the regions, never the models behind them.
        "models": live.iter().map(|(r, _)| r.id).collect::<Vec<_>>(),
        "regions": live
            .iter()
            .map(|(r, b)| serde_json::json!({
                "id": r.id,
                "label": r.label,
                "flag": r.flag,
                // What a credit buys for this call's task, so the app shows its real price.
                "multiplier": (if speech { b.speech(&keyed) } else { Some(b.for_task(task, &keyed)) })
                    .and_then(|(_, m)| pricing.model_multipliers.get(m).copied())
                    .unwrap_or(1.0),
            }))
            .collect::<Vec<_>>(),
        "tokens_per_credit": pricing.tokens_per_credit,
        "upstream": upstream,
    }))
}

// ── POST /api/internal/llm/charge ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ChargeRequest {
    pub subscription_id: String,
    pub email: String,
    pub app: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    #[serde(default)]
    pub estimated: bool,
    pub status_code: i32,
    #[serde(default)]
    pub response_time_ms: Option<i64>,
    /// The region the app chose, when it chose one.
    #[serde(default)]
    pub region: Option<String>,
    /// The answer stopped at max_tokens.
    #[serde(default)]
    pub cut_off: bool,
}

pub async fn charge_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<ChargeRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant_id = match provider_tenant() {
        Ok(t) => t,
        Err(r) => return r,
    };
    let b = body.into_inner();
    let total = b.input_tokens.max(0) + b.output_tokens.max(0);

    // Spent first, so llm_usage holds what was actually charged. A call is
    // recorded even when nothing could be charged, so it can be reconciled.
    let (credits, balance) = if total == 0 {
        (0, None)
    } else {
        match subscription::spend_credits(&store, &b.subscription_id, &b.model, total).await {
            Ok(s) => (s.credits, Some(s.credits_left)),
            Err(SpendError::NoPlan) => {
                app_log!(warn, email = %b.email, tokens = total, "LLM call ended after the plan did; not charged");
                (0, None)
            }
            Err(e) => {
                app_log!(error, error = %e, email = %b.email, tokens = total, "LLM call not charged");
                (0, None)
            }
        }
    };

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let usage_id = format!("llm_{}", uuid::Uuid::new_v4().simple());
    let result = client
        .execute(
            "INSERT INTO llm_usage (id, tenant_id, license_id, email, product, provider, model,
                input_tokens, output_tokens, total_tokens, estimated, credits, status_code, response_time_ms, region, cut_off)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
            &[
                &usage_id, &tenant_id, &b.subscription_id, &b.email, &b.app, &b.provider, &b.model,
                &b.input_tokens, &b.output_tokens, &total, &b.estimated, &credits, &b.status_code,
                &b.response_time_ms, &b.region, &b.cut_off,
            ],
        )
        .await;
    if let Err(e) = result {
        app_log!(error, error = %e, subscription_id = %b.subscription_id, credits = credits, tokens = total, "Could not record an LLM call");
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not record the call");
    }

    app_log!(info, subscription_id = %b.subscription_id, app = %b.app, model = %b.model,
        tokens = total, credits = credits, balance = ?balance, "LLM call charged");
    HttpResponse::Ok().json(serde_json::json!({ "success": true, "credits": credits, "balance": balance }))
}

// ── api0's own AI calls ─────────────────────────────────────────────────────

/// Record an AI call api0 made for a person (a YAML import) in llm_usage, so
/// the super admin's ledger holds every call. No licence, nothing charged.
/// Best effort: a failure is logged, never passed on.
pub async fn record_platform_call(
    store: &EndpointStore,
    tenant_id: Option<&str>,
    email: &str,
    product: &str,
    spend: &crate::infra::formatter::LlmSpend,
) {
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, product = %product, "Could not record an AI call");
            return;
        }
    };
    let id = format!("llm_{}", uuid::Uuid::new_v4().simple());
    let total = spend.input_tokens.max(0) + spend.output_tokens.max(0);
    let result = client
        .execute(
            "INSERT INTO llm_usage (id, tenant_id, license_id, email, product, provider, model,
                input_tokens, output_tokens, total_tokens, estimated, credits, status_code, response_time_ms)
             VALUES ($1,
                COALESCE($2, (SELECT default_tenant_id FROM user_preferences WHERE email = $3), ''),
                NULL, $3, $4, $5, $6, $7, $8, $9, $10, 0, 200, $11)",
            &[
                &id, &tenant_id, &email, &product, &spend.provider, &spend.model,
                &spend.input_tokens, &spend.output_tokens, &total, &spend.estimated, &spend.response_time_ms,
            ],
        )
        .await;
    if let Err(e) = result {
        app_log!(error, error = %e, product = %product, model = %spend.model, tokens = total, "Could not record an AI call");
    }
}

// ── Usage, for the dashboard ────────────────────────────────────────────────

const USAGE_DEFAULT_LIMIT: i64 = 50;
const USAGE_MAX_LIMIT: i64 = 200;

/// One recorded call, as the dashboard lists it. `app` is the product that
/// made the call; `model` is the region it chose, or "api0" — the subscriber
/// never sees which model ran, unless the app named one itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UsageEntry {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub app: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub tokens: i64,
    pub credits: i64,
    pub ok: bool,
}

/// The columns USAGE_SQL selects, in order.
type UsageCols = (String, DateTime<Utc>, String, String, i64, i64, i64, i64, i32);

const USAGE_SQL: &str = "SELECT id, created_at, product, COALESCE(region, model), input_tokens, output_tokens, total_tokens, credits, status_code \
     FROM llm_usage WHERE email = $1 AND license_id IS NOT NULL ORDER BY created_at DESC LIMIT $2 OFFSET $3";

const USAGE_COUNT_SQL: &str = "SELECT COUNT(*) FROM llm_usage WHERE email = $1 AND license_id IS NOT NULL";

fn usage_entry((id, created_at, app, model, input_tokens, output_tokens, tokens, credits, status): UsageCols) -> UsageEntry {
    UsageEntry {
        id,
        created_at,
        app,
        model,
        input_tokens,
        output_tokens,
        tokens,
        credits,
        ok: (200..300).contains(&status),
    }
}

fn usage_limit(requested: Option<i64>) -> i64 {
    requested.unwrap_or(USAGE_DEFAULT_LIMIT).clamp(1, USAGE_MAX_LIMIT)
}

fn usage_offset(requested: Option<i64>) -> i64 {
    requested.unwrap_or(0).max(0)
}

/// A page of the email's calls, newest first, and how many there are in all.
pub async fn recent_usage(
    store: &EndpointStore,
    email: &str,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<(Vec<UsageEntry>, i64), String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let email = email.trim().to_lowercase();
    let rows = client
        .query(USAGE_SQL, &[&email, &usage_limit(limit), &usage_offset(offset)])
        .await
        .map_err(|e| e.to_string())?;
    let total: i64 = client
        .query_one(USAGE_COUNT_SQL, &[&email])
        .await
        .map_err(|e| e.to_string())?
        .get(0);
    let usage = rows
        .iter()
        .map(|r| usage_entry((r.get(0), r.get(1), r.get(2), r.get(3), r.get(4), r.get(5), r.get(6), r.get(7), r.get(8))))
        .collect();
    Ok((usage, total))
}

#[derive(Deserialize)]
pub struct UsageQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// GET /api/plans/usage/{email}?limit=N&offset=M — also for partners with
/// credits.read, whose users' history is their plan's when the workspace bills
/// each person's plan.
pub async fn usage_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
    query: web::Query<UsageQuery>,
) -> impl Responder {
    if let Err(deny) = crate::middleware::service_key::require_scope(&req, crate::middleware::service_key::Scope::CreditsRead) {
        return deny;
    }
    match recent_usage(&store, &email, query.limit, query.offset).await {
        Ok((usage, total)) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "usage": usage, "total": total })),
        Err(e) => {
            app_log!(error, error = %e, "Could not read LLM usage");
            fail(StatusCode::INTERNAL_SERVER_ERROR, "Usage unavailable")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_calling_app_is_logged_as_a_plain_id() {
        assert_eq!(app_label(Some(" GitAgent ")), "gitagent");
        assert_eq!(app_label(Some("job scout!")), "jobscout");
        assert_eq!(app_label(None), "unknown");
        assert_eq!(app_label(Some("  ")), "unknown");
    }

    #[test]
    fn a_model_is_picked_by_name_or_falls_back_to_the_platform_choice() {
        let offered = [("deepseek", "deepseek-chat"), ("deepseek", "deepseek-reasoner"), ("mistral", "mistral-medium-latest")];
        let default = Some(("mistral", "mistral-medium-latest"));
        assert_eq!(pick_model("deepseek-reasoner", "Mayo", default, &offered), Some(("deepseek", "deepseek-reasoner")));
        assert_eq!(pick_model("", "Mayo", default, &offered), default);
        assert_eq!(pick_model("mayo", "Mayo", default, &offered), default);
        assert_eq!(pick_model("gpt-5", "Mayo", default, &offered), None);
        // A choice whose provider has no key falls back to what is on offer.
        assert_eq!(pick_model("default", "Mayo", Some(("other", "x")), &offered), Some(("deepseek", "deepseek-chat")));
    }

    #[test]
    fn a_region_is_picked_by_id_and_the_answer_names_only_what_was_asked() {
        let china = Backing { provider: "deepseek".into(), model: "deepseek-chat".into(), tasks: Default::default() };
        let live = [(&llm_regions::REGIONS[1], &china)];
        let (r, b) = pick_region(" China ", &live).unwrap();
        assert_eq!((r.id, b.model.as_str()), ("china", "deepseek-chat"));
        assert!(pick_region("europe", &live).is_none(), "not live");
        assert!(pick_region("deepseek-chat", &live).is_none());

        assert_eq!(public_name("China", Some(r), "Mayo"), "china");
        assert_eq!(public_name("", None, "Mayo"), "default");
        assert_eq!(public_name("mayo", None, "Mayo"), "default");
        assert_eq!(public_name("deepseek-reasoner", None, "Mayo"), "deepseek-reasoner");
    }

    #[test]
    fn a_usage_row_maps_to_what_the_dashboard_lists() {
        let at: DateTime<Utc> = "2026-10-20T08:30:00Z".parse().unwrap();
        let entry = usage_entry(("llm_1".into(), at, "notes".into(), "deepseek-chat".into(), 1200, 300, 1500, 2, 200));
        assert_eq!(
            entry,
            UsageEntry {
                id: "llm_1".into(),
                created_at: at,
                app: "notes".into(),
                model: "deepseek-chat".into(),
                input_tokens: 1200,
                output_tokens: 300,
                tokens: 1500,
                credits: 2,
                ok: true,
            }
        );
        assert!(!usage_entry(("llm_2".into(), at, "notes".into(), "m".into(), 0, 0, 0, 0, 502)).ok);
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["app"], "notes");
        assert_eq!(json["tokens"], 1500);
        assert_eq!(json["created_at"], "2026-10-20T08:30:00Z");
    }

    #[test]
    fn the_usage_limit_has_a_default_and_bounds() {
        assert_eq!(usage_limit(None), USAGE_DEFAULT_LIMIT);
        assert_eq!(usage_limit(Some(10)), 10);
        assert_eq!(usage_limit(Some(0)), 1);
        assert_eq!(usage_limit(Some(10_000)), USAGE_MAX_LIMIT);
        assert_eq!(usage_offset(None), 0);
        assert_eq!(usage_offset(Some(-5)), 0);
        assert_eq!(usage_offset(Some(40)), 40);
    }
}
