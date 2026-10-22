// src/admin/llm_regions.rs
//
// What a desktop app's user chooses between: three regions, never a model.
// The super admin puts a provider and one of its models behind each region; the
// app sends the region's id as its `model` (gateway /v1/chat/completions) and
// only ever sees the region back: in /v1/models, in the `model` of every
// answer, and in its usage on the dashboard. Which provider and model ran stays
// on the server, in llm_usage and the super admin's ledger.
//
//   GET /api/admin/llm-regions   every region with what is behind it, and the
//                                providers on offer (with whether their key is set)
//   PUT /api/admin/llm-regions   { regions: [{ id, provider, model }] } — an
//                                empty provider takes the region off offer
//
// Stored in system_config as `llm.regions`: { "<region>": { provider, model } }.
// The keys are the platform ones set in Admin → Platform (assistant_config).
// Both need X-Internal-Secret; the gateway restricts them to the super admin.

use crate::admin::assistant_config::{self, PROVIDERS};
use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{http::StatusCode, web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

const CONFIG_KEY: &str = "llm.regions";

pub(crate) struct Region {
    pub id: &'static str,
    pub label: &'static str,
    pub flag: &'static str,
}

/// In the order the apps list them.
pub(crate) const REGIONS: &[Region] = &[
    Region { id: "europe", label: "Europe", flag: "🇪🇺" },
    Region { id: "china", label: "China", flag: "🇨🇳" },
    Region { id: "usa", label: "United States", flag: "🇺🇸" },
];

pub(crate) fn region(id: &str) -> Option<&'static Region> {
    REGIONS.iter().find(|r| r.id.eq_ignore_ascii_case(id.trim()))
}

/// The provider and model behind a region.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Backing {
    pub provider: String,
    pub model: String,
}

/// What `llm.regions` holds, without regions that no longer exist.
fn parse(value: &str) -> HashMap<String, Backing> {
    serde_json::from_str::<HashMap<String, Backing>>(value)
        .unwrap_or_default()
        .into_iter()
        .filter(|(id, _)| region(id).is_some())
        .collect()
}

/// Every region with something behind it, by region id.
pub(crate) async fn read_regions(store: &EndpointStore) -> Result<HashMap<String, Backing>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let row = client
        .query_opt("SELECT value FROM system_config WHERE key = $1", &[&CONFIG_KEY])
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.map(|r| parse(r.get::<_, &str>(0))).unwrap_or_default())
}

/// The regions an app can use now — set, and their provider's key too — in
/// the order of REGIONS.
pub(crate) fn live<'a>(regions: &'a HashMap<String, Backing>, keyed: &[String]) -> Vec<(&'static Region, &'a Backing)> {
    REGIONS
        .iter()
        .filter_map(|r| regions.get(r.id).map(|b| (r, b)))
        .filter(|(_, b)| keyed.iter().any(|k| *k == b.provider))
        .collect()
}

/// Whether a region is set to `provider`, so its key must stay.
pub(crate) async fn uses_provider(store: &EndpointStore, provider: &str) -> Option<&'static str> {
    let regions = read_regions(store).await.ok()?;
    REGIONS.iter().find(|r| regions.get(r.id).is_some_and(|b| b.provider == provider)).map(|r| r.label)
}

fn fail(status: StatusCode, msg: impl Into<String>) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({ "success": false, "error": msg.into() }))
}

// ── GET /api/admin/llm-regions ───────────────────────────────────────────────

pub async fn get_regions(req: HttpRequest, store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let (regions, keyed) = match (read_regions(&store).await, assistant_config::providers_with_key(&store).await) {
        (Ok(r), Ok(k)) => (r, k),
        (Err(e), _) | (_, Err(e)) => {
            app_log!(error, error = %e, "Could not read the LLM regions");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not read the regions");
        }
    };
    let list: Vec<serde_json::Value> = REGIONS
        .iter()
        .map(|r| {
            let b = regions.get(r.id);
            serde_json::json!({
                "id": r.id,
                "label": r.label,
                "flag": r.flag,
                "provider": b.map(|b| &b.provider),
                "model": b.map(|b| &b.model),
                "live": b.is_some_and(|b| keyed.iter().any(|k| *k == b.provider)),
            })
        })
        .collect();
    let providers: Vec<serde_json::Value> = PROVIDERS
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "label": p.label,
                "models": p.models,
                "key_set": keyed.iter().any(|k| k == p.id),
            })
        })
        .collect();
    HttpResponse::Ok().json(serde_json::json!({ "success": true, "regions": list, "providers": providers }))
}

// ── PUT /api/admin/llm-regions ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegionChoice {
    pub id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveRegionsRequest {
    pub regions: Vec<RegionChoice>,
}

/// The regions as saved, or what is wrong with the request.
fn validate(choices: &[RegionChoice], keyed: &[String]) -> Result<HashMap<String, Backing>, String> {
    let mut out = HashMap::new();
    for c in choices {
        let Some(r) = region(&c.id) else {
            return Err(format!("Unknown region '{}'", c.id));
        };
        if c.provider.trim().is_empty() {
            continue;
        }
        let Some(p) = assistant_config::provider(c.provider.trim()) else {
            return Err(format!("Unknown provider '{}' for {}", c.provider, r.label));
        };
        let model = c.model.trim();
        if !p.models.contains(&model) {
            return Err(format!("'{}' is not a {} model: {}", model, p.label, p.models.join(", ")));
        }
        if !keyed.iter().any(|k| k == p.id) {
            return Err(format!("Add a {} API key before putting it behind {}", p.label, r.label));
        }
        out.insert(r.id.to_string(), Backing { provider: p.id.to_string(), model: model.to_string() });
    }
    Ok(out)
}

pub async fn save_regions(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SaveRegionsRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let keyed = match assistant_config::providers_with_key(&store).await {
        Ok(k) => k,
        Err(e) => {
            app_log!(error, error = %e, "Could not read the provider keys");
            return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not read the provider keys");
        }
    };
    let regions = match validate(&body.regions, &keyed) {
        Ok(r) => r,
        Err(e) => return fail(StatusCode::BAD_REQUEST, e),
    };
    let value = serde_json::to_string(&regions).unwrap_or_else(|_| "{}".into());
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
        app_log!(error, error = %e, "Could not save the LLM regions");
        return fail(StatusCode::INTERNAL_SERVER_ERROR, "Could not save the regions");
    }
    app_log!(info, regions = %value, "LLM regions changed");
    HttpResponse::Ok().json(serde_json::json!({ "success": true }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(id: &str, provider: &str, model: &str) -> RegionChoice {
        RegionChoice { id: id.into(), provider: provider.into(), model: model.into() }
    }

    #[test]
    fn a_region_needs_a_known_model_of_a_provider_with_a_key() {
        let keyed = vec!["deepseek".to_string(), "mistral".to_string()];
        let ok = validate(&[choice("china", "deepseek", "deepseek-chat"), choice("usa", "", "")], &keyed).unwrap();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok["china"], Backing { provider: "deepseek".into(), model: "deepseek-chat".into() });

        assert!(validate(&[choice("mars", "deepseek", "deepseek-chat")], &keyed).is_err());
        assert!(validate(&[choice("europe", "mistral", "gpt-5")], &keyed).is_err());
        assert!(validate(&[choice("usa", "anthropic", "claude-sonnet-5-5")], &keyed).is_err(), "no key yet");
    }

    #[test]
    fn only_regions_with_a_keyed_provider_are_live_in_order() {
        let saved = parse(
            r#"{"usa":{"provider":"anthropic","model":"claude-sonnet-5-5"},
                "china":{"provider":"deepseek","model":"deepseek-chat"},
                "europe":{"provider":"mistral","model":"mistral-medium-latest"},
                "mars":{"provider":"deepseek","model":"deepseek-chat"}}"#,
        );
        assert_eq!(saved.len(), 3, "an unknown region is dropped");
        let keyed = vec!["deepseek".to_string(), "mistral".to_string()];
        let ids: Vec<&str> = live(&saved, &keyed).iter().map(|(r, _)| r.id).collect();
        assert_eq!(ids, ["europe", "china"]);
        assert!(parse("not json").is_empty());
    }
}
