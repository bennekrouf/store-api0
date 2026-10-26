// src/admin/llm_models.rs
//
// The models the super admin can put behind a region (llm_regions.rs), with
// what the dashboard needs to choose between them: the region each belongs to
// (where its provider is based), the tasks it suits (TASKS), its price per
// million tokens, its context window and a note. Prices change often, so the list is the super admin's to
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

/// What a model can be good at, by who calls it: code (GitAgent's reviews and
/// commit messages), chat (the messaging assistant on Telegram, WhatsApp and
/// Teams), text (PublishKit's posts). A model's tasks only say what it is good
/// for; a region runs another model for a task when the super admin says so
/// (llm_regions.rs).
pub(crate) const TASKS: &[&str] = &["code", "chat", "text"];

/// When DEFAULTS' prices were looked up.
pub(crate) const DEFAULTS_PRICED_ON: &str = "2026-10-22";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CatalogModel {
    pub provider: String,
    pub model: String,
    pub label: String,
    /// The region it belongs to (llm_regions::REGIONS).
    pub origin: String,
    /// The TASKS it suits, in TASKS' order; empty in a list saved before tasks.
    #[serde(default)]
    pub tasks: Vec<String>,
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

// One row of the DEFAULTS table, so each reads on a line or two.
#[allow(clippy::too_many_arguments)]
fn entry(
    provider: &str,
    model: &str,
    label: &str,
    origin: &str,
    tasks: &[&str],
    prices: Option<(f64, f64)>,
    cached: Option<f64>,
    note: &str,
) -> CatalogModel {
    CatalogModel {
        provider: provider.into(),
        model: model.into(),
        label: label.into(),
        origin: origin.into(),
        tasks: tasks.iter().map(|t| t.to_string()).collect(),
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
        entry("mistral", "mistral-small-latest", "Mistral Small", "europe", &["chat", "text"], Some((0.15, 0.60)), None,
            "Cheap: PR comments and descriptions."),
        entry("mistral", "codestral-latest", "Codestral", "europe", &["code"], Some((0.30, 0.90)), None,
            "Made for code: a good default for reviews."),
        entry("mistral", "mistral-medium-latest", "Mistral Medium", "europe", &["code", "chat", "text"], Some((1.50, 7.50)), None,
            "Strongest here, about 10× Small."),
        // China
        entry("deepseek", "deepseek-flash", "DeepSeek Flash", "china", &["code", "chat", "text"], Some((0.30, 1.20)), Some(0.003),
            "Peak price shown; half at other hours and weekends. Good for reviews and comments."),
        entry("deepseek", "deepseek-v4-pro", "DeepSeek V4 Pro", "china", &["code", "text"], Some((1.32, 3.96)), None,
            "Stronger, about 4× Flash. Peak price shown."),
        entry("deepseek", "deepseek-chat", "DeepSeek (old name)", "china", &["chat", "text"], Some((0.30, 1.20)), Some(0.003),
            "Old name, billed as Flash."),
        entry("qwen", "qwen3-coder-next", "Qwen3 Coder Next", "china", &["code"], Some((0.11, 0.80)), None,
            "Id and price from a third-party listing — test before use. Looped on JSON reviews at temperature 0."),
        entry("zai", "glm-5.3-flash", "GLM-5.3 Flash", "china", &["chat", "text"], Some((0.15, 0.50)), None, UNCONFIRMED),
        // United States
        entry("anthropic", "claude-haiku-5-5", "Claude Haiku 5.5", "usa", &["code", "chat"], Some((0.10, 0.50)), None,
            "Good for reviews. Price for prompts up to 100K tokens; 5× above that."),
        entry("anthropic", "claude-sonnet-5-5", "Claude Sonnet 5.5", "usa", &["code", "text"], Some((2.00, 10.00)), None,
            "Premium, about 20× Haiku."),
        entry("anthropic", "claude-opus-5-5", "Claude Opus 5.5", "usa", &["code", "text"], Some((4.00, 20.00)), None,
            "Most capable and most expensive, about 2× Sonnet."),
        entry("google", "gemini-3.1-flash-lite", "Gemini 3.1 Flash-Lite", "usa", &["chat", "text"], Some((0.25, 1.50)), None, UNCONFIRMED),
    ]
}

/// What the gateway sets on a call to a model known to loop without it: a
/// temperature floor, and a presence penalty when the app sends none. By model
/// id prefix, in code rather than the list, so it holds whatever the super
/// admin saves. Qwen3's card warns that greedy decoding repeats endlessly.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(crate) struct Sampling {
    pub min_temperature: f64,
    pub presence_penalty: f64,
}

const SAMPLING: &[(&str, Sampling)] = &[
    ("qwen3", Sampling { min_temperature: 0.7, presence_penalty: 1.0 }),
];

pub(crate) fn sampling(model: &str) -> Option<Sampling> {
    let model = model.to_ascii_lowercase();
    SAMPLING.iter().find(|(prefix, _)| model.starts_with(prefix)).map(|(_, s)| *s)
}

/// The share of a call's tokens that are input, for one blended price per
/// model (the dashboard's LlmRegionsPanel uses the same).
pub(crate) const INPUT_SHARE: f64 = 0.9;

impl CatalogModel {
    /// $ per million tokens at INPUT_SHARE; None while either price is unknown.
    pub(crate) fn blended_price(&self) -> Option<f64> {
        Some(INPUT_SHARE * self.input_price? + (1.0 - INPUT_SHARE) * self.output_price?)
    }
}

/// A credit multiplier for every model the live regions run (llm_regions),
/// from its price: the cheapest is ×1 and the others cost as many times more
/// as they do at the provider, to one decimal. A model with no price in the
/// catalog gets none. Multipliers the super admin set win over these
/// (plans::Pricing::with_derived).
pub(crate) fn derived_multipliers(
    catalog: &[CatalogModel],
    regions: &std::collections::HashMap<String, llm_regions::Backing>,
    keyed: &[String],
) -> std::collections::BTreeMap<String, f64> {
    let priced: Vec<(&str, f64)> = llm_regions::live(regions, keyed)
        .iter()
        .flat_map(|(_, b)| b.picks())
        .filter(|(p, _)| keyed.iter().any(|k| k == p))
        .filter_map(|(p, m)| {
            let price = catalog.iter().find(|c| c.provider == p && c.model == m)?.blended_price()?;
            (price > 0.0).then_some((m, price))
        })
        .collect();
    let Some(cheapest) = priced.iter().map(|(_, p)| *p).reduce(f64::min) else {
        return Default::default();
    };
    priced
        .into_iter()
        .map(|(m, p)| (m.to_string(), ((p / cheapest) * 10.0).round().max(10.0) / 10.0))
        .collect()
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
        if let Some(t) = m.tasks.iter().find(|t| !TASKS.contains(&t.trim())) {
            return Err(format!("Unknown task '{}' for {}", t, m.model));
        }
        m.tasks = TASKS.iter().filter(|t| m.tasks.iter().any(|x| x.trim() == **t)).map(|t| t.to_string()).collect();
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
    // A model a region runs on, for any task, cannot leave the list from under it.
    if let Ok(regions) = llm_regions::read_regions(&store).await {
        for r in llm_regions::REGIONS {
            for (provider, model) in regions.get(r.id).into_iter().flat_map(|b| b.picks()) {
                if !models.iter().any(|m| m.provider == provider && m.model == model) {
                    return fail(
                        StatusCode::BAD_REQUEST,
                        format!("{} runs on {} — put another model behind it first", r.label, model),
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
    fn only_models_known_to_loop_get_a_sampling_guard() {
        assert_eq!(sampling("Qwen3-Coder-Next").map(|s| s.min_temperature), Some(0.7));
        assert!(sampling("deepseek-flash").is_none());
        assert!(sampling("codestral-latest").is_none());
    }

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
        let mut bad = m.clone();
        bad.output_price = Some(-1.0);
        assert!(validate(vec![bad]).is_err());
        let mut bad = m.clone();
        bad.tasks = vec!["poetry".into()];
        assert!(validate(vec![bad]).is_err());
        let mut messy = m;
        messy.tasks = vec!["text".into(), " code".into(), "text".into()];
        assert_eq!(validate(vec![messy]).unwrap()[0].tasks, ["code", "text"], "in TASKS' order, once");
    }

    #[test]
    fn multipliers_follow_the_prices_of_what_the_regions_run() {
        let keyed: Vec<String> = ["mistral", "deepseek", "anthropic"].iter().map(|s| s.to_string()).collect();
        let d = derived_multipliers(&defaults(), &llm_regions::defaults(), &keyed);
        // Haiku ($0.10 in, $0.50 out), the USA bots' model, is the cheapest
        // the default regions run.
        assert_eq!(d["claude-haiku-5-5"], 1.0);
        assert_eq!(d["deepseek-flash"], 2.8);
        assert_eq!(d["deepseek-v4-pro"], 11.3);
        assert_eq!(d["mistral-medium-latest"], 15.0);
        assert_eq!(d["claude-sonnet-5-5"], 20.0);
        assert!(!d.contains_key("claude-opus-5-5"), "not run by any region");

        // A region off offer (no key) neither gets multipliers nor sets the ×1.
        let no_usa: Vec<String> = keyed.iter().filter(|k| *k != "anthropic").cloned().collect();
        let d = derived_multipliers(&defaults(), &llm_regions::defaults(), &no_usa);
        assert_eq!(d["deepseek-flash"], 1.0);
        assert!(!d.contains_key("claude-haiku-5-5"));
        assert!(derived_multipliers(&defaults(), &llm_regions::defaults(), &[]).is_empty());
    }

    #[test]
    fn a_model_without_a_price_gets_no_multiplier() {
        let mut catalog = defaults();
        catalog.iter_mut().filter(|m| m.model == "deepseek-flash").for_each(|m| m.output_price = None);
        let keyed = vec!["deepseek".to_string()];
        let d = derived_multipliers(&catalog, &llm_regions::defaults(), &keyed);
        assert!(!d.contains_key("deepseek-flash"));
        assert_eq!(d["deepseek-v4-pro"], 1.0);
    }
}
