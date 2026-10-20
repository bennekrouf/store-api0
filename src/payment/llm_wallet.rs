// src/payment/llm_wallet.rs
//
// The store's half of the LLM provider: a tenant (LLM_PROVIDER_TENANT_ID) that
// serves chat completions to the desktop apps through the gateway's
// /v1/chat/completions. Calls are paid from the monthly plan credits of the
// key holder's email (payment::subscription).
//
//   POST /api/internal/llm/authorize  { license_key, model? }
//        → who it is, their plan credits left, the models on offer and, when
//          one matches, the upstream provider to forward to (with its key)
//   POST /api/internal/llm/charge     one completion's tokens
//        → { credits, balance }: spends plan credits and records the call
//          under the provider tenant in llm_usage
//
// Both need X-Internal-Secret. Two kinds of key are accepted, signed alike:
//   v1  an app licence (payment::license): must be in `licenses`, not refunded
//   v2  a plan key {"v":2,"product":"plan","expires",…}: valid until `expires`
// Either way it is the email's plan that pays; an app licence alone carries no
// AI credits. The upstream keys are the platform ones the super admin sets in
// Admin → Messaging assistant (assistant_config).

use crate::admin::assistant_config::{self, PROVIDERS};
use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::license;
use crate::payment::subscription::{self, SpendError};
use actix_web::{http::StatusCode, web, HttpRequest, HttpResponse, Responder};
use chrono::{NaiveDate, Utc};
use serde::Deserialize;
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

// ── Keys ─────────────────────────────────────────────────────────────────────

/// The fields both key versions share, plus v2's expiry.
#[derive(Deserialize)]
struct SignedKey {
    v: u8,
    id: String,
    product: String,
    email: String,
    #[serde(default)]
    expires: Option<NaiveDate>,
}

/// Who a key belongs to. `id` is the licence or subscription id.
struct Holder {
    id: String,
    email: String,
    product: String,
}

enum KeyCheck {
    /// v1: still has to be found in `licenses`.
    Licence(SignedKey),
    Plan(Holder),
    Refused(&'static str),
}

/// What the signature and the payload alone can say about a key.
fn classify(key: SignedKey, today: NaiveDate) -> KeyCheck {
    match key.v {
        1 => KeyCheck::Licence(key),
        2 if key.product == subscription::KEY_PRODUCT => match key.expires {
            Some(d) if d >= today => KeyCheck::Plan(Holder { id: key.id, email: key.email.to_lowercase(), product: key.product }),
            Some(_) => KeyCheck::Refused("This plan key has expired"),
            None => KeyCheck::Refused("Invalid key"),
        },
        _ => KeyCheck::Refused("Invalid key"),
    }
}

/// The key's holder, or the response refusing it.
async fn holder(store: &EndpointStore, key: &str) -> Result<Holder, HttpResponse> {
    let signed: SignedKey = match license::verify_signed(key) {
        Ok(p) => p,
        Err(e) if e.contains("LICENSE_SIGNING_KEY") => {
            app_log!(error, error = %e, "Keys cannot be checked");
            return Err(fail(StatusCode::SERVICE_UNAVAILABLE, "Keys cannot be checked right now"));
        }
        Err(_) => return Err(fail(StatusCode::UNAUTHORIZED, "Invalid key")),
    };
    let licence = match classify(signed, Utc::now().date_naive()) {
        KeyCheck::Plan(h) => return Ok(h),
        KeyCheck::Refused(why) => return Err(fail(StatusCode::UNAUTHORIZED, why)),
        KeyCheck::Licence(l) => l,
    };
    // The signature proves the store issued it; the row says it still stands.
    let client = store.get_admin_conn().await.map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let row = client
        .query_opt(
            "SELECT email, product, revoked_at IS NOT NULL FROM licenses WHERE id = $1 AND key = $2",
            &[&licence.id, &key],
        )
        .await
        .map_err(|e| fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| fail(StatusCode::UNAUTHORIZED, "Unknown licence key"))?;
    if row.get::<_, bool>(2) {
        return Err(fail(StatusCode::UNAUTHORIZED, "This licence was refunded"));
    }
    Ok(Holder { id: licence.id, email: row.get(0), product: row.get(1) })
}

// ── POST /api/internal/llm/authorize ─────────────────────────────────────────

#[derive(Deserialize)]
pub struct AuthorizeRequest {
    pub license_key: String,
    #[serde(default)]
    pub model: Option<String>,
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
    let who = match holder(&store, body.license_key.trim()).await {
        Ok(h) => h,
        Err(r) => return r,
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
    let credits_left = match subscription::credits_left(&store, &who.email).await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "Could not read plan credits");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not read the plan");
        }
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
    let choice = assistant_config::read_choice(&store).await.ok().flatten();
    let default = choice.as_ref().map(|(p, m)| (p.as_str(), m.as_str()));
    let picked = pick_model(body.model.as_deref().unwrap_or_default(), &tenant_name, default, &offered);

    let upstream = match picked.and_then(|(p, m)| assistant_config::provider(p).map(|prov| (prov, m))) {
        Some((prov, model)) => match assistant_config::api_key(&store, prov.id).await {
            Ok(Some(key)) => serde_json::json!({
                "provider": prov.id, "base_url": prov.base_url, "model": model, "api_key": key,
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
        "license_id": who.id,
        "email": who.email,
        "product": who.product,
        "tenant_id": tenant_id,
        "tenant_name": tenant_name,
        "plan": credits_left.is_some(),
        "balance": credits_left.unwrap_or(0),
        "models": offered.iter().map(|(_, m)| *m).collect::<Vec<_>>(),
        "upstream": upstream,
    }))
}

// ── POST /api/internal/llm/charge ────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ChargeRequest {
    pub license_id: String,
    pub email: String,
    pub product: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    #[serde(default)]
    pub estimated: bool,
    pub status_code: i32,
    #[serde(default)]
    pub response_time_ms: Option<i64>,
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
        match subscription::spend_plan_credits(&store, &b.email, &b.model, total).await {
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
                input_tokens, output_tokens, total_tokens, estimated, credits, status_code, response_time_ms)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            &[
                &usage_id, &tenant_id, &b.license_id, &b.email, &b.product, &b.provider, &b.model,
                &b.input_tokens, &b.output_tokens, &total, &b.estimated, &credits, &b.status_code,
                &b.response_time_ms,
            ],
        )
        .await;
    if let Err(e) = result {
        app_log!(error, error = %e, license_id = %b.license_id, credits = credits, tokens = total, "Could not record an LLM call");
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not record the call");
    }

    app_log!(info, license_id = %b.license_id, product = %b.product, model = %b.model,
        tokens = total, credits = credits, balance = ?balance, "LLM call charged");
    HttpResponse::Ok().json(serde_json::json!({ "success": true, "credits": credits, "balance": balance }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(v: u8, product: &str, expires: Option<&str>) -> SignedKey {
        SignedKey {
            v,
            id: "k1".into(),
            product: product.into(),
            email: "A@B.c".into(),
            expires: expires.map(|d| d.parse().unwrap()),
        }
    }

    #[test]
    fn a_plan_key_holds_until_its_expiry_day_and_a_licence_goes_to_the_table() {
        let today: NaiveDate = "2026-10-19".parse().unwrap();
        assert!(matches!(classify(key(2, "plan", Some("2026-10-19")), today), KeyCheck::Plan(h) if h.email == "a@b.c"));
        assert!(matches!(classify(key(2, "plan", Some("2026-10-18")), today), KeyCheck::Refused(_)));
        assert!(matches!(classify(key(2, "plan", None), today), KeyCheck::Refused(_)));
        assert!(matches!(classify(key(2, "gitagent", Some("2027-01-01")), today), KeyCheck::Refused(_)));
        assert!(matches!(classify(key(1, "gitagent", None), today), KeyCheck::Licence(_)));
        assert!(matches!(classify(key(3, "plan", Some("2027-01-01")), today), KeyCheck::Refused(_)));
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
}
