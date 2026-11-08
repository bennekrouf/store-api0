// src/admin/llm_recommendations.rs
//
// Once a week, the cheapest capable model of each region (llm_regions.rs),
// read from OpenRouter's public model list (GET /api/v1/models: no key, no
// cost). The super admin sees it next to what the region runs now, in the
// Platform tab; nothing switches on its own.
//
// A candidate: a model of a provider api0 calls (OPENROUTER_AUTHORS), of the
// region that provider belongs to, that calls tools and holds 128K tokens or
// more, came out in the past year (MAX_AGE_SECS), and is not a free, batch,
// preview, image or small variant, nor open weights its provider's API does
// not serve (gemma, gpt-oss). They are
// ranked by blended price (llm_models::INPUT_SHARE); the first three are kept.
//
// Prices are OpenRouter's, in US dollars a million tokens; the provider's own
// may differ. The ids are OpenRouter's: a model is added to the list
// (llm_models.rs) under its provider's own id before a region can run it.
//
// Stored in system_config under CONFIG_KEY and returned by
// GET /api/admin/llm-regions as `recommendations`.

use crate::admin::llm_models::INPUT_SHARE;
use crate::app_log;
use crate::endpoint_store::EndpointStore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const CONFIG_KEY: &str = "llm.recommendations";
const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
const WEEK_SECS: i64 = 7 * 24 * 3600;
const KEEP: usize = 3;
const MIN_CONTEXT: i64 = 128_000;
const MAX_AGE_SECS: i64 = 365 * 24 * 3600;

/// OpenRouter's author → api0's provider (assistant_config::PROVIDERS) and its region.
const OPENROUTER_AUTHORS: &[(&str, &str, &str)] = &[
    ("mistralai", "mistral", "europe"),
    ("deepseek", "deepseek", "china"),
    ("qwen", "qwen", "china"),
    ("z-ai", "zai", "china"),
    ("anthropic", "anthropic", "usa"),
    ("openai", "openai", "usa"),
    ("google", "google", "usa"),
];

/// Variants that are not a model to run a region on.
const SKIP: &[&str] = &[
    ":free", ":batch", "preview", "image", "vision", "legal", "safety", "omni", "audio", "embed", "-3b", "-8b", "nano",
    "lightning", "lite-image", "customtools", "-exp", "ultraspeed", "gemma", "gpt-oss",
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Recommended {
    /// OpenRouter's id, e.g. "z-ai/glm-5.3-flash".
    pub id: String,
    pub name: String,
    /// api0's provider id.
    pub provider: String,
    pub input_price: f64,
    pub output_price: f64,
    /// $ a million tokens at INPUT_SHARE.
    pub blended_price: f64,
    pub context_k: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Recommendations {
    /// Unix seconds.
    pub computed_at: i64,
    pub source: String,
    /// By region id, cheapest first.
    pub regions: BTreeMap<String, Vec<Recommended>>,
}

#[derive(Deserialize)]
struct ListResponse {
    data: Vec<ListedModel>,
}

#[derive(Deserialize)]
struct ListedModel {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    created: i64,
    #[serde(default)]
    context_length: Option<i64>,
    #[serde(default)]
    supported_parameters: Vec<String>,
    pricing: ListedPricing,
}

#[derive(Deserialize)]
struct ListedPricing {
    #[serde(default)]
    prompt: String,
    #[serde(default)]
    completion: String,
}

/// $ a token as OpenRouter writes it → $ a million tokens.
fn per_million(v: &str) -> Option<f64> {
    v.parse::<f64>().ok().filter(|p| *p > 0.0).map(|p| p * 1_000_000.0)
}

fn candidate(m: &ListedModel, now: i64) -> Option<(&'static str, Recommended)> {
    let id = m.id.to_ascii_lowercase();
    let author = id.split('/').next()?;
    let (_, provider, region) = OPENROUTER_AUTHORS.iter().find(|(a, _, _)| *a == author)?;
    if id.starts_with('~') || SKIP.iter().any(|s| id.contains(s)) || now - m.created > MAX_AGE_SECS {
        return None;
    }
    if !m.supported_parameters.iter().any(|p| p == "tools") || m.context_length.unwrap_or(0) < MIN_CONTEXT {
        return None;
    }
    let (input, output) = (per_million(&m.pricing.prompt)?, per_million(&m.pricing.completion)?);
    let name = m.name.split_once(": ").map(|(_, n)| n).unwrap_or(&m.name).to_string();
    Some((
        region,
        Recommended {
            id: m.id.clone(),
            name: if name.is_empty() { m.id.clone() } else { name },
            provider: provider.to_string(),
            input_price: input,
            output_price: output,
            blended_price: INPUT_SHARE * input + (1.0 - INPUT_SHARE) * output,
            context_k: m.context_length.unwrap_or(0) / 1000,
        },
    ))
}

/// The cheapest KEEP candidates of each region.
fn rank(models: &[ListedModel], now: i64) -> Recommendations {
    let mut regions: BTreeMap<String, Vec<Recommended>> = BTreeMap::new();
    for (region, r) in models.iter().filter_map(|m| candidate(m, now)) {
        regions.entry(region.to_string()).or_default().push(r);
    }
    for list in regions.values_mut() {
        list.sort_by(|a, b| a.blended_price.total_cmp(&b.blended_price).then_with(|| a.id.cmp(&b.id)));
        list.truncate(KEEP);
    }
    Recommendations { computed_at: now, source: MODELS_URL.into(), regions }
}

pub(crate) async fn read(store: &EndpointStore) -> Option<Recommendations> {
    let client = store.get_admin_conn().await.ok()?;
    let row = client.query_opt("SELECT value FROM system_config WHERE key = $1", &[&CONFIG_KEY]).await.ok()??;
    serde_json::from_str(row.get::<_, &str>(0)).ok()
}

async fn compute(store: &EndpointStore) -> Result<Recommendations, String> {
    let list: ListResponse = reqwest::Client::new()
        .get(MODELS_URL)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let recs = rank(&list.data, chrono::Utc::now().timestamp());
    let value = serde_json::to_string(&recs).map_err(|e| e.to_string())?;
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    client
        .execute(
            "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW()) \
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
            &[&CONFIG_KEY, &value],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(recs)
}

/// Recomputes when the last list is a week old or missing. Called every few hours.
pub(crate) async fn refresh_if_due(store: &EndpointStore) {
    let now = chrono::Utc::now().timestamp();
    if read(store).await.is_some_and(|r| now - r.computed_at < WEEK_SECS) {
        return;
    }
    match compute(store).await {
        Ok(r) => app_log!(info, regions = r.regions.len(), "Recommended models recomputed from OpenRouter"),
        Err(e) => app_log!(warn, error = %e, "Could not recompute the recommended models"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listed(id: &str, prompt: &str, completion: &str, tools: bool, ctx: i64) -> ListedModel {
        ListedModel {
            id: id.into(),
            name: format!("Author: {id}"),
            created: 1,
            context_length: Some(ctx),
            supported_parameters: if tools { vec!["tools".into()] } else { vec![] },
            pricing: ListedPricing { prompt: prompt.into(), completion: completion.into() },
        }
    }

    #[test]
    fn the_cheapest_capable_models_of_each_region_are_kept() {
        let models = vec![
            listed("z-ai/glm-5.3-flash", "0.000000034", "0.000000083", true, 1_048_576),
            listed("deepseek/deepseek-v4-pro", "0.000000262", "0.000000523", true, 1_048_576),
            listed("deepseek/deepseek-v4.1-flash:batch", "0.00000001", "0.0000002", true, 1_048_576),
            listed("qwen/qwen3.7-flash", "0.00000003", "0.00000013", false, 1_000_000),
            listed("mistralai/mistral-small-2603", "0.00000015", "0.0000006", true, 262_144),
            listed("mistralai/mistral-small-24b", "0.00000005", "0.00000008", true, 32_768),
            listed("mistralai/ministral-3b-2512", "0.0000001", "0.0000001", true, 131_072),
            listed("nvidia/nemotron-3-super", "0.0000001", "0.0000001", true, 1_000_000),
            listed("openai/gpt-6-luna", "0", "0", true, 1_000_000),
            ListedModel { created: -MAX_AGE_SECS, ..listed("qwen/qwen3-old", "0.00000001", "0.00000001", true, 1_000_000) },
        ];
        let r = rank(&models, 1);
        let ids = |region: &str| r.regions.get(region).map(|l| l.iter().map(|m| m.id.as_str()).collect::<Vec<_>>());
        assert_eq!(ids("china"), Some(vec!["z-ai/glm-5.3-flash", "deepseek/deepseek-v4-pro"]), "batch and no-tools left out");
        assert_eq!(ids("europe"), Some(vec!["mistralai/mistral-small-2603"]), "small context and 3B left out");
        assert_eq!(ids("usa"), None, "no price, no recommendation");
        let glm = &r.regions["china"][0];
        assert_eq!(glm.provider, "zai");
        assert_eq!(glm.name, "z-ai/glm-5.3-flash");
        assert!((glm.blended_price - (0.9 * 0.034 + 0.1 * 0.083)).abs() < 1e-9);
    }
}
