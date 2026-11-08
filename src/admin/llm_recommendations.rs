// src/admin/llm_recommendations.rs
//
// Once a week, the cheapest capable model for every region (llm_regions.rs)
// and task (llm_models::TASKS), read from OpenRouter's public model lists
// (GET /api/v1/models: no key, no cost). The super admin sees them in the
// Platform tab's region × task table; nothing switches on its own.
//
// A candidate: a model of a provider api0 calls (OPENROUTER_AUTHORS), of the
// region that provider belongs to, that came out in the past year
// (MAX_AGE_SECS) and is not a free, batch, preview, image or small variant,
// nor open weights its provider's API does not serve (gemma, gpt-oss).
//
//   code, chat, text   calls tools and holds 128K tokens or more; ranked by
//                      blended price (llm_models::INPUT_SHARE). Code keeps the
//                      ones OpenRouter ranks for programming, text the ones it
//                      ranks for marketing (?category=), else coding models
//                      for code, else every candidate.
//   transcribe         a model that outputs a transcription
//                      (?output_modalities=transcription), priced by the
//                      second of audio; ranked by price a minute.
//
// The first three of each are kept. Prices are OpenRouter's, in US dollars;
// the provider's own may differ. The ids are OpenRouter's: a model is added to
// the list (llm_models.rs) under its provider's own id before a region runs it.
//
// Stored in system_config under CONFIG_KEY and returned by
// GET /api/admin/llm-regions as `recommendations`.

use crate::admin::llm_models::INPUT_SHARE;
use crate::app_log;
use crate::endpoint_store::EndpointStore;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

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
    "lightning", "lite-image", "customtools", "-exp", "ultraspeed", "gemma", "gpt-oss", "medical",
];

/// Made for code, when OpenRouter ranks none of a region's for programming.
const CODING: &[&str] = &["coder", "codestral", "devstral", "codex"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Recommended {
    /// OpenRouter's id, e.g. "z-ai/glm-5.3-flash".
    pub id: String,
    pub name: String,
    /// api0's provider id.
    pub provider: String,
    /// $ a million tokens at INPUT_SHARE; 0 for a speech model.
    pub blended_price: f64,
    /// $ a minute of audio, for a speech model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minute_price: Option<f64>,
    pub context_k: i64,
}

/// By region, then task, cheapest first.
pub(crate) type ByRegion = BTreeMap<String, BTreeMap<String, Vec<Recommended>>>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct Recommendations {
    /// Unix seconds.
    pub computed_at: i64,
    pub source: String,
    pub regions: ByRegion,
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

/// OpenRouter's lists for one week's ranking.
struct Lists {
    all: Vec<ListedModel>,
    programming: Vec<ListedModel>,
    marketing: Vec<ListedModel>,
    transcription: Vec<ListedModel>,
}

fn price(v: &str) -> Option<f64> {
    v.parse::<f64>().ok().filter(|p| *p > 0.0)
}

/// The region and api0 provider of a model worth showing, or None.
fn placed(m: &ListedModel, now: i64) -> Option<(&'static str, &'static str)> {
    let id = m.id.to_ascii_lowercase();
    let author = id.split('/').next()?;
    let (_, provider, region) = OPENROUTER_AUTHORS.iter().find(|(a, _, _)| *a == author)?;
    if id.starts_with('~') || SKIP.iter().any(|s| id.contains(s)) || now - m.created > MAX_AGE_SECS {
        return None;
    }
    Some((region, provider))
}

fn display_name(m: &ListedModel) -> String {
    let name = m.name.split_once(": ").map(|(_, n)| n).unwrap_or(&m.name);
    if name.is_empty() { m.id.clone() } else { name.to_string() }
}

/// A text model that calls tools: its region and how it ranks.
fn text_candidate(m: &ListedModel, now: i64) -> Option<(&'static str, Recommended)> {
    let (region, provider) = placed(m, now)?;
    if !m.supported_parameters.iter().any(|p| p == "tools") || m.context_length.unwrap_or(0) < MIN_CONTEXT {
        return None;
    }
    let (input, output) = (price(&m.pricing.prompt)? * 1e6, price(&m.pricing.completion)? * 1e6);
    Some((
        region,
        Recommended {
            id: m.id.clone(),
            name: display_name(m),
            provider: provider.to_string(),
            blended_price: INPUT_SHARE * input + (1.0 - INPUT_SHARE) * output,
            minute_price: None,
            context_k: m.context_length.unwrap_or(0) / 1000,
        },
    ))
}

/// A transcription model priced by the second of audio (none by the token).
fn speech_candidate(m: &ListedModel, now: i64) -> Option<(&'static str, Recommended)> {
    let (region, provider) = placed(m, now)?;
    if price(&m.pricing.completion).is_some() {
        return None;
    }
    let per_second = price(&m.pricing.prompt)?;
    Some((
        region,
        Recommended {
            id: m.id.clone(),
            name: display_name(m),
            provider: provider.to_string(),
            blended_price: 0.0,
            minute_price: Some(per_second * 60.0),
            context_k: 0,
        },
    ))
}

fn cheapest(mut list: Vec<Recommended>) -> Vec<Recommended> {
    let key = |r: &Recommended| r.minute_price.unwrap_or(r.blended_price);
    list.sort_by(|a, b| key(a).total_cmp(&key(b)).then_with(|| a.id.cmp(&b.id)));
    list.dedup_by(|a, b| a.id == b.id);
    list.truncate(KEEP);
    list
}

/// The first non-empty of `lists`, cheapest first.
fn first_found(lists: impl IntoIterator<Item = Vec<Recommended>>) -> Vec<Recommended> {
    lists.into_iter().find(|l| !l.is_empty()).map(cheapest).unwrap_or_default()
}

fn rank(lists: &Lists, now: i64) -> Recommendations {
    let ids = |l: &[ListedModel]| l.iter().map(|m| m.id.clone()).collect::<HashSet<_>>();
    let (programming, marketing) = (ids(&lists.programming), ids(&lists.marketing));
    let mut text: BTreeMap<&str, Vec<Recommended>> = BTreeMap::new();
    for (region, r) in lists.all.iter().filter_map(|m| text_candidate(m, now)) {
        text.entry(region).or_default().push(r);
    }
    let mut speech: BTreeMap<&str, Vec<Recommended>> = BTreeMap::new();
    for (region, r) in lists.transcription.iter().filter_map(|m| speech_candidate(m, now)) {
        speech.entry(region).or_default().push(r);
    }
    let mut regions = ByRegion::new();
    for (_, _, region) in OPENROUTER_AUTHORS {
        if regions.contains_key(*region) {
            continue;
        }
        let all = text.get(region).cloned().unwrap_or_default();
        let only = |keep: &dyn Fn(&Recommended) -> bool| all.iter().filter(|r| keep(r)).cloned().collect::<Vec<_>>();
        let mut tasks = BTreeMap::new();
        tasks.insert(
            "code".to_string(),
            first_found([
                only(&|r| programming.contains(&r.id)),
                only(&|r| CODING.iter().any(|c| r.id.contains(c))),
                all.clone(),
            ]),
        );
        tasks.insert("chat".to_string(), first_found([all.clone()]));
        tasks.insert("text".to_string(), first_found([only(&|r| marketing.contains(&r.id)), all.clone()]));
        tasks.insert("transcribe".to_string(), first_found([speech.get(region).cloned().unwrap_or_default()]));
        tasks.retain(|_, l| !l.is_empty());
        if !tasks.is_empty() {
            regions.insert(region.to_string(), tasks);
        }
    }
    Recommendations { computed_at: now, source: MODELS_URL.into(), regions }
}

pub(crate) async fn read(store: &EndpointStore) -> Option<Recommendations> {
    let client = store.get_admin_conn().await.ok()?;
    let row = client.query_opt("SELECT value FROM system_config WHERE key = $1", &[&CONFIG_KEY]).await.ok()??;
    serde_json::from_str(row.get::<_, &str>(0)).ok()
}

async fn fetch(http: &reqwest::Client, query: &str) -> Result<Vec<ListedModel>, String> {
    let list: ListResponse = http
        .get(format!("{MODELS_URL}{query}"))
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    Ok(list.data)
}

async fn compute(store: &EndpointStore) -> Result<Recommendations, String> {
    let http = reqwest::Client::new();
    let lists = Lists {
        all: fetch(&http, "").await?,
        programming: fetch(&http, "?category=programming").await.unwrap_or_default(),
        marketing: fetch(&http, "?category=marketing").await.unwrap_or_default(),
        transcription: fetch(&http, "?output_modalities=transcription").await.unwrap_or_default(),
    };
    let recs = rank(&lists, chrono::Utc::now().timestamp());
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

/// Recomputes when the last list is a week old, missing or of an older shape.
/// Called every few hours.
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

    fn ids(r: &Recommendations, region: &str, task: &str) -> Vec<String> {
        r.regions.get(region).and_then(|t| t.get(task)).map(|l| l.iter().map(|m| m.id.clone()).collect()).unwrap_or_default()
    }

    #[test]
    fn every_region_and_task_gets_its_cheapest_capable_models() {
        let all = vec![
            listed("z-ai/glm-5.3-flash", "0.000000034", "0.000000083", true, 1_048_576),
            listed("deepseek/deepseek-v4-pro", "0.000000262", "0.000000523", true, 1_048_576),
            listed("deepseek/deepseek-v4.1-flash:batch", "0.00000001", "0.0000002", true, 1_048_576),
            listed("qwen/qwen3.7-flash", "0.00000003", "0.00000013", false, 1_000_000),
            ListedModel { created: -MAX_AGE_SECS, ..listed("qwen/qwen3-old", "0.00000001", "0.00000001", true, 1_000_000) },
            listed("mistralai/mistral-small-2603", "0.00000015", "0.0000006", true, 262_144),
            listed("mistralai/devstral-2512", "0.0000004", "0.000002", true, 262_144),
            listed("mistralai/mistral-small-24b", "0.00000005", "0.00000008", true, 32_768),
            listed("mistralai/ministral-3b-2512", "0.0000001", "0.0000001", true, 131_072),
            listed("nvidia/nemotron-3-super", "0.0000001", "0.0000001", true, 1_000_000),
            listed("openai/gpt-6-luna", "0", "0", true, 1_000_000),
        ];
        let lists = Lists {
            programming: vec![listed("deepseek/deepseek-v4-pro", "", "", true, 0)],
            marketing: vec![],
            transcription: vec![
                listed("mistralai/voxtral-mini-transcribe-3-0", "0.0000583333", "", false, 0),
                listed("mistralai/voxtral-small-transcribe-3-0", "0.0000666667", "", false, 0),
                listed("google/gemini-3.5-transcribe", "0.000002", "0.000012", false, 0),
            ],
            all,
        };
        let r = rank(&lists, 1);
        assert_eq!(ids(&r, "china", "chat"), ["z-ai/glm-5.3-flash", "deepseek/deepseek-v4-pro"], "batch, no tools, old left out");
        assert_eq!(ids(&r, "china", "code"), ["deepseek/deepseek-v4-pro"], "ranked for programming first");
        assert_eq!(ids(&r, "china", "text"), ids(&r, "china", "chat"), "nothing ranked for marketing: every candidate");
        assert_eq!(ids(&r, "europe", "code"), ["mistralai/devstral-2512"], "else a coding model");
        assert_eq!(ids(&r, "europe", "chat"), ["mistralai/mistral-small-2603", "mistralai/devstral-2512"]);
        assert_eq!(
            ids(&r, "europe", "transcribe"),
            ["mistralai/voxtral-mini-transcribe-3-0", "mistralai/voxtral-small-transcribe-3-0"]
        );
        assert!(ids(&r, "usa", "transcribe").is_empty(), "a token-priced transcriber is left out");
        assert!(!r.regions.contains_key("usa"), "no price, no recommendation");
        let glm = &r.regions["china"]["chat"][0];
        assert_eq!((glm.provider.as_str(), glm.name.as_str()), ("zai", "z-ai/glm-5.3-flash"));
        assert!((glm.blended_price - (0.9 * 0.034 + 0.1 * 0.083)).abs() < 1e-9);
        let voxtral = r.regions["europe"]["transcribe"][0].minute_price.unwrap();
        assert!((voxtral - 0.0035).abs() < 1e-6);
    }
}
