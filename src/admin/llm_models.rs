// src/admin/llm_models.rs
//
// The models the super admin can put behind a region (llm_regions.rs), with
// what the dashboard needs to choose between them: the region each belongs to
// (where its provider is based), its price per million tokens, its context
// window and a note. Prices change often, so the list is the super admin's to
// edit; until it is saved, DEFAULTS stands.
//
//   PUT  /api/admin/llm-models        { models: [...] } — the whole list
//   POST /api/admin/llm-models/test   { provider, model } → one tiny real call:
//                                     { ok, status, ms, reply | error }
//
// GET /api/admin/llm-regions returns the list with the regions. All need
// X-Internal-Secret; the gateway restricts them to the super admin.
//
// Prices are in US dollars, at the provider's full rate (DeepSeek's peak
// hours), so an estimate made from them errs high. Entries whose id came from
// a third-party listing say so in their note: test them before use.

use crate::admin::assistant_config;
use crate::admin::llm_regions;
use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{http::StatusCode, web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

const CONFIG_KEY: &str = "llm.models";

/// When DEFAULTS' prices were looked up.
pub(crate) const DEFAULTS_PRICED_ON: &str = "2026-10-22";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CatalogModel {
    pub provider: String,
    pub model: String,
    pub label: String,
    /// The region it belongs to (llm_regions::REGIONS).
    pub origin: String,
    /// $ per million tokens; `None` while unknown.
    #[serde(default)]
    pub input_price: Option<f64>,
    #[serde(default)]
    pub cached_input_price: Option<f64>,
    #[serde(default)]
    pub output_price: Option<f64>,
    /// Thousands of tokens.
    #[serde(default)]
    pub context_k: Option<i64>,
    #[serde(default)]
    pub note: String,
}

fn entry(
    provider: &str,
    model: &str,
    label: &str,
    origin: &str,
    prices: Option<(f64, f64)>,
    cached: Option<f64>,
    note: &str,
) -> CatalogModel {
    CatalogModel {
        provider: provider.into(),
        model: model.into(),
        label: label.into(),
        origin: origin.into(),
        input_price: prices.map(|p| p.0),
        cached_input_price: cached,
        output_price: prices.map(|p| p.1),
        context_k: None,
        note: note.into(),
    }
}

const UNCONFIRMED: &str = "Id and price from a third-party listing — test before use.";

/// The list before the super admin saves one.
pub(crate) fn defaults() -> Vec<CatalogModel> {
    vec![
        // Europe
        entry("mistral", "mistral-small-latest", "Mistral Small", "europe", Some((0.15, 0.60)), None,
            "Cheap: PR comments and descriptions."),
        entry("mistral", "codestral-latest", "Codestral", "europe", Some((0.30, 0.90)), None,
            "Made for code: a good default for reviews."),
        entry("mistral", "mistral-medium-latest", "Mistral Medium", "europe", Some((1.50, 7.50)), None,
            "Strongest here, about 10× Small."),
        // China
        entry("deepseek", "deepseek-flash", "DeepSeek Flash", "china", Some((0.30, 1.20)), Some(0.003),
            "Peak price shown; half at other hours and weekends. Good for reviews and comments."),
        entry("deepseek", "deepseek-v4-pro", "DeepSeek V4 Pro", "china", Some((1.32, 3.96)), None,
            "Stronger, about 4× Flash. Peak price shown."),
        entry("deepseek", "deepseek-chat", "DeepSeek (old name)", "china", Some((0.30, 1.20)), Some(0.003),
            "Old name, billed as Flash."),
        entry("qwen", "qwen3-coder-next", "Qwen3 Coder Next", "china", Some((0.11, 0.80)), None, UNCONFIRMED),
        entry("zai", "glm-5.3-flash", "GLM-5.3 Flash", "china", Some((0.15, 0.50)), None, UNCONFIRMED),
        // United States
        entry("anthropic", "claude-haiku-5-5", "Claude Haiku 5.5", "usa", Some((0.10, 0.50)), None,
            "Good for reviews. Price from a third-party listing — check Anthropic's."),
        entry("anthropic", "claude-sonnet-5-5", "Claude Sonnet 5.5", "usa", None, None,
            "Premium. Set its price from Anthropic's page before offering it."),
        entry("anthropic", "claude-opus-5-5", "Claude Opus 5.5", "usa", None, None,
            "Most capable and most expensive. Set its price before offering it."),
        entry("google", "gemini-3.1-flash-lite", "Gemini 3.1 Flash-Lite", "usa", Some((0.25, 1.50)), None, UNCONFIRMED),
    ]
}

/// The saved list, or DEFAULTS.
pub(crate) async fn read_catalog(store: &EndpointStore) -> Result<Vec<CatalogModel>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let row = client
        .query_opt("SELECT value FROM system_config WHERE key = $1", &[&CONFIG_KEY])
        .await
        .map_err(|e| e.to_string())?;
    Ok(row
        .and_then(|r| serde_json::from_str::<Vec<CatalogModel>>(r.get::<_, &str>(0)).ok())
        .unwrap_or_else(defaults))
}

fn fail(status: StatusCode, msg: impl Into<String>) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({ "success": false, "error": msg.into() }))
}

// ── PUT /api/admin/llm-models ────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SaveModelsRequest {
    pub models: Vec<CatalogModel>,
}

/// The list as saved: trimmed, checked, one entry per provider and model.
fn validate(models: Vec<CatalogModel>) -> Result<Vec<CatalogModel>, String> {
    let mut out: Vec<CatalogModel> = Vec::new();
    for mut m in models {
        m.provider = m.provider.trim().to_string();
        m.model = m.model.trim().to_string();
        m.label = m.label.trim().to_string();
        m.note = m.note.trim().to_string();
        if m.model.is_empty() {
            return Err("Every model needs its id".into());
        }
        if assistant_config::provider(&m.provider).is_none() {
            return Err(format!("Unknown provider '{}' for {}", m.provider, m.model));
        }
        if llm_regions::region(&m.origin).is_none() {
            return Err(format!("Unknown region '{}' for {}", m.origin, m.model));
        }
        for price in [m.input_price, m.cached_input_price, m.output_price].into_iter().flatten() {
            if !(price.is_finite() && price >= 0.0) {
                return Err(format!("{}: a price must be 0 or more", m.model));
            }
        }
        if out.iter().any(|o| o.provider == m.provider && o.model == m.model) {
            return Err(format!("{} is listed twice", m.model));
        }
        if m.label.is_empty() {
            m.label = m.model.clone();
        }
        out.push(m);
    }
    Ok(out)
}

pub async fn save_models(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SaveModelsRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let models = match validate(body.into_inner().models) {
        Ok(m) => m,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    // A model a region runs on cannot leave the list from under it.
    if let Ok(regions) = llm_regions::read_regions(&store).await {
        for r in llm_regions::REGIONS {
            if let Some(b) = regions.get(r.id) {
                if !models.iter().any(|m| m.provider == b.provider && m.model == b.model) {
                    return fail(
                        StatusCode::BAD_REQUEST,
                        format!("{} runs on {} — put another model behind it first", r.label, b.model),
                    );
                }
            }
        }
    }
    let value = serde_json::to_string(&models).unwrap_or_else(|_| "[]".into());
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return fail(StatusCode::INTERNAL_SERVER_ERROR, "Database unavailable"),
    };
    if let Err(e) = client
        .execute(
            "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW())
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
            &[&CONFIG_KEY, &value],
        )
        .await
    {
        app_log!(error, error = %e, "Could not save the model list");
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not save the models");
    }
    app_log!(info, models = models.len(), "LLM model list changed");
    HttpResponse::Ok().json(serde_json::json!({ "success": true }))
}

// ── POST /api/admin/llm-models/test ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TestRequest {
    pub provider: String,
    pub model: String,
}

/// One short real call, so a model id and its key are proved before a region
/// is put on it. Costs a few tokens.
pub async fn test_model(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<TestRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let Some(p) = assistant_config::provider(body.provider.trim()) else {
        return fail(StatusCode::BAD_REQUEST, format!("Unknown provider '{}'", body.provider));
    };
    let key = match assistant_config::api_key(&store, p.id).await {
        Ok(Some(k)) => k,
        Ok(None) => return fail(StatusCode::BAD_REQUEST, format!("Add a {} API key first", p.label)),
        Err(e) => {
            app_log!(error, error = %e, "Could not read a provider key for a test");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not read the provider key");
        }
    };
    let client = match reqwest::Client::builder().timeout(Duration::from_secs(30)).build() {
        Ok(c) => c,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let started = Instant::now();
    let sent = client
        .post(p.chat_url())
        .bearer_auth(key)
        .json(&serde_json::json!({
            "model": body.model.trim(),
            "messages": [{ "role": "user", "content": "Reply with the single word OK." }],
            "max_tokens": 16,
        }))
        .send()
        .await;
    let ms = started.elapsed().as_millis() as i64;
    let resp = match sent {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse::Ok().json(serde_json::json!({
                "success": true, "ok": false, "status": 0, "ms": ms, "error": format!("Unreachable: {}", e),
            }))
        }
    };
    let status = resp.status().as_u16();
    let v: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    let ok = (200..300).contains(&status);
    let error = v["error"]["message"].as_str().or(v["message"].as_str()).or(v["error"].as_str());
    app_log!(info, provider = %p.id, model = %body.model, status = status, ms = ms, "LLM model tested");
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "ok": ok,
        "status": status,
        "ms": ms,
        "reply": v["choices"][0]["message"]["content"].as_str(),
        "error": if ok { None } else { Some(error.unwrap_or("The provider refused the call").to_string()) },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_names_a_known_provider_and_region_once() {
        let d = defaults();
        assert_eq!(validate(d.clone()).unwrap(), d);
        for r in llm_regions::REGIONS {
            assert!(d.iter().any(|m| m.origin == r.id), "{} has no model", r.id);
        }
    }

    #[test]
    fn the_list_is_checked_before_it_is_saved() {
        let mut m = defaults().remove(0);
        m.label = " ".into();
        assert_eq!(validate(vec![m.clone()]).unwrap()[0].label, m.model, "an empty label is the id");
        assert!(validate(vec![m.clone(), m.clone()]).is_err(), "twice");
        let mut bad = m.clone();
        bad.provider = "nobody".into();
        assert!(validate(vec![bad]).is_err());
        let mut bad = m.clone();
        bad.origin = "mars".into();
        assert!(validate(vec![bad]).is_err());
        let mut bad = m;
        bad.output_price = Some(-1.0);
        assert!(validate(vec![bad]).is_err());
    }
}
