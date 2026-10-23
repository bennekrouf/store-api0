// src/admin/llm_regions.rs
//
// What a desktop app's user chooses between: three regions, never a model.
// The super admin puts a provider and one of its models behind each region; the
// app sends the region's id as its `model` (gateway /v1/chat/completions) and
// only ever sees the region back: in /v1/models, in the `model` of every
// answer, and in its usage on the dashboard. Which provider and model ran stays
// on the server, in llm_usage and the super admin's ledger.
//
//   GET /api/admin/llm-regions   every region with what is behind it, the
//                                providers (with whether their key is set) and
//                                the models to choose from, priced (llm_models.rs)
//   PUT /api/admin/llm-regions   { regions: [{ id, provider, model, tasks? }] } —
//                                an empty provider takes the region off offer;
//                                `tasks` is { "<task>": { provider, model } }
//
// A region may run another model for one task (llm_models::TASKS): code for
// GitAgent, text for PublishKit. A call's task is the one it names in
// `X-Task`, else the one its app does (APP_TASKS), and a task with nothing of
// its own runs on the region's model. The app still sees only the region.
//
// Stored in system_config as `llm.regions`:
// { "<region>": { provider, model, tasks: { "<task>": { provider, model } } } };
// until it is saved, defaults() stands. A region is offered only once its
// provider's key is set.
// The keys are the platform ones set in Admin → Platform (assistant_config).
// Both need X-Internal-Secret; the gateway restricts them to the super admin.

use crate::admin::assistant_config::{self, PROVIDERS};
use crate::admin::llm_models::{self, CatalogModel};
use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{http::StatusCode, web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
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

/// A provider and one of its models.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Pick {
    pub provider: String,
    pub model: String,
}

/// The provider and model behind a region, and the tasks that run on another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Backing {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tasks: BTreeMap<String, Pick>,
}

impl Backing {
    /// What runs `task`: its own model when its provider has a key, else the region's.
    pub(crate) fn for_task<'a>(&'a self, task: Option<&str>, keyed: &[String]) -> (&'a str, &'a str) {
        match task.and_then(|t| self.tasks.get(t)).filter(|p| keyed.contains(&p.provider)) {
            Some(p) => (&p.provider, &p.model),
            None => (&self.provider, &self.model),
        }
    }

    /// Every provider and model it runs on.
    pub(crate) fn picks(&self) -> impl Iterator<Item = (&str, &str)> {
        std::iter::once((self.provider.as_str(), self.model.as_str()))
            .chain(self.tasks.values().map(|p| (p.provider.as_str(), p.model.as_str())))
    }
}

/// The task each app's calls do, by the id it sends in X-Client-App.
const APP_TASKS: &[(&str, &str)] = &[
    ("gitagent", "code"),
    ("publishkit", "text"),
    ("jobscout", "text"),
    ("cvenom", "text"),
    ("bridge", "chat"),
];

/// A call's task: the one it names, if known, else its app's; None for neither.
pub(crate) fn task_of(named: Option<&str>, app: &str) -> Option<&'static str> {
    let named = named.map(|t| t.trim().to_lowercase());
    llm_models::TASKS
        .iter()
        .find(|t| named.as_deref() == Some(**t))
        .or_else(|| APP_TASKS.iter().find(|(a, _)| *a == app).map(|(_, t)| t))
        .copied()
}

fn backing(provider: &str, model: &str, tasks: &[(&str, &str, &str)]) -> Backing {
    Backing {
        provider: provider.into(),
        model: model.into(),
        tasks: tasks
            .iter()
            .map(|(t, p, m)| (t.to_string(), Pick { provider: p.to_string(), model: m.to_string() }))
            .collect(),
    }
}

/// The regions before the super admin saves any: the strongest all-rounder
/// of each region by default, and per task the model that suits it at the
/// best margin. A task on the region's own model is left out.
pub(crate) fn defaults() -> HashMap<String, Backing> {
    HashMap::from([
        // Only three models here, so Small covers both chat and text.
        ("europe".to_string(), backing("mistral", "mistral-medium-latest", &[
            ("code", "mistral", "codestral-latest"),
            ("chat", "mistral", "mistral-small-latest"),
            ("text", "mistral", "mistral-small-latest"),
        ])),
        // DeepSeek V4 Pro is the other choice for code, within DeepSeek.
        ("china".to_string(), backing("deepseek", "deepseek-flash", &[
            ("code", "qwen", "qwen3-coder-next"),
            ("chat", "zai", "glm-5.3-flash"),
            ("text", "zai", "glm-5.3-flash"),
        ])),
        // Haiku is the only code model with a confirmed price, so code stays on it.
        ("usa".to_string(), backing("anthropic", "claude-haiku-5-5", &[
            ("chat", "google", "gemini-3.1-flash-lite"),
            ("text", "google", "gemini-3.1-flash-lite"),
        ])),
    ])
}

/// What `llm.regions` holds, without regions that no longer exist.
fn parse(value: &str) -> HashMap<String, Backing> {
    serde_json::from_str::<HashMap<String, Backing>>(value)
        .unwrap_or_default()
        .into_iter()
        .filter(|(id, _)| region(id).is_some())
        .collect()
}

/// Every region with something behind it, by region id: as saved, else defaults().
pub(crate) async fn read_regions(store: &EndpointStore) -> Result<HashMap<String, Backing>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let row = client
        .query_opt("SELECT value FROM system_config WHERE key = $1", &[&CONFIG_KEY])
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.map(|r| parse(r.get::<_, &str>(0))).unwrap_or_else(defaults))
}

/// The regions an app can use now — set, and their provider's key too — in
/// the order of REGIONS.
pub(crate) fn live<'a>(regions: &'a HashMap<String, Backing>, keyed: &[String]) -> Vec<(&'static Region, &'a Backing)> {
    REGIONS
        .iter()
        .filter_map(|r| regions.get(r.id).map(|b| (r, b)))
        .filter(|(_, b)| keyed.contains(&b.provider))
        .collect()
}

/// Whether a region is set to `provider`, so its key must stay.
pub(crate) async fn uses_provider(store: &EndpointStore, provider: &str) -> Option<&'static str> {
    let regions = read_regions(store).await.ok()?;
    REGIONS
        .iter()
        .find(|r| regions.get(r.id).is_some_and(|b| b.picks().any(|(p, _)| p == provider)))
        .map(|r| r.label)
}

fn fail(status: StatusCode, msg: impl Into<String>) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({ "success": false, "error": msg.into() }))
}

// ── GET /api/admin/llm-regions ───────────────────────────────────────────────

pub async fn get_regions(req: HttpRequest, store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let (regions, keyed, catalog) = match (
        read_regions(&store).await,
        assistant_config::providers_with_key(&store).await,
        llm_models::read_catalog(&store).await,
    ) {
        (Ok(r), Ok(k), Ok(c)) => (r, k, c),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => {
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
                "tasks": b.map(|b| &b.tasks),
                "live": b.is_some_and(|b| keyed.contains(&b.provider)),
            })
        })
        .collect();
    let providers: Vec<serde_json::Value> = PROVIDERS
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "label": p.label,
                "key_set": keyed.iter().any(|k| k == p.id),
            })
        })
        .collect();
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "regions": list,
        "providers": providers,
        "models": catalog,
        "defaults_priced_on": llm_models::DEFAULTS_PRICED_ON,
    }))
}

// ── PUT /api/admin/llm-regions ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RegionChoice {
    pub id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    /// By task; an empty provider leaves the task on the region's model.
    #[serde(default)]
    pub tasks: HashMap<String, PickChoice>,
}

#[derive(Debug, Deserialize)]
pub struct PickChoice {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
}

/// A provider and model as chosen: known, in the list, and with a key.
fn checked_pick(provider: &str, model: &str, what: &str, keyed: &[String], catalog: &[CatalogModel]) -> Result<Pick, String> {
    let Some(p) = assistant_config::provider(provider.trim()) else {
        return Err(format!("Unknown provider '{}' for {}", provider, what));
    };
    let model = model.trim();
    if !catalog.iter().any(|m| m.provider == p.id && m.model == model) {
        return Err(format!("'{}' is not in the model list for {}", model, p.label));
    }
    if !keyed.iter().any(|k| k == p.id) {
        return Err(format!("Add a {} API key before putting it behind {}", p.label, what));
    }
    Ok(Pick { provider: p.id.to_string(), model: model.to_string() })
}

#[derive(Debug, Deserialize)]
pub struct SaveRegionsRequest {
    pub regions: Vec<RegionChoice>,
}

/// The regions as saved, or what is wrong with the request.
fn validate(choices: &[RegionChoice], keyed: &[String], catalog: &[CatalogModel]) -> Result<HashMap<String, Backing>, String> {
    let mut out = HashMap::new();
    for c in choices {
        let Some(r) = region(&c.id) else {
            return Err(format!("Unknown region '{}'", c.id));
        };
        let chosen: Vec<_> = c.tasks.iter().filter(|(_, t)| !t.provider.trim().is_empty()).collect();
        if c.provider.trim().is_empty() {
            if !chosen.is_empty() {
                return Err(format!("Choose {}'s own model before one for a task", r.label));
            }
            continue;
        }
        let main = checked_pick(&c.provider, &c.model, r.label, keyed, catalog)?;
        let mut tasks = BTreeMap::new();
        for (task, t) in chosen {
            let Some(task) = llm_models::TASKS.iter().find(|x| **x == task.trim()) else {
                return Err(format!("Unknown task '{}' for {}", task, r.label));
            };
            let pick = checked_pick(&t.provider, &t.model, &format!("{} {}", r.label, task), keyed, catalog)?;
            // The region's own model already runs it.
            if pick != main {
                tasks.insert(task.to_string(), pick);
            }
        }
        out.insert(r.id.to_string(), Backing { provider: main.provider, model: main.model, tasks });
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
    let catalog = match llm_models::read_catalog(&store).await {
        Ok(c) => c,
        Err(e) => return fail(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let regions = match validate(&body.regions, &keyed, &catalog) {
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
        RegionChoice { id: id.into(), provider: provider.into(), model: model.into(), tasks: HashMap::new() }
    }

    fn with_task(mut c: RegionChoice, task: &str, provider: &str, model: &str) -> RegionChoice {
        c.tasks.insert(task.into(), PickChoice { provider: provider.into(), model: model.into() });
        c
    }

    fn pick(provider: &str, model: &str) -> Pick {
        Pick { provider: provider.into(), model: model.into() }
    }

    #[test]
    fn a_region_needs_a_known_model_of_a_provider_with_a_key() {
        let keyed = vec!["deepseek".to_string(), "mistral".to_string()];
        let catalog = llm_models::defaults();
        let validate = |c: &[RegionChoice], k: &[String]| validate(c, k, &catalog);
        let ok = validate(&[choice("china", "deepseek", "deepseek-chat"), choice("usa", "", "")], &keyed).unwrap();
        assert_eq!(ok.len(), 1);
        assert_eq!(ok["china"], Backing { provider: "deepseek".into(), model: "deepseek-chat".into(), tasks: BTreeMap::new() });

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
        assert!(saved["china"].tasks.is_empty(), "a region saved before tasks has none");
    }

    #[test]
    fn a_task_may_run_on_its_own_model_of_a_provider_with_a_key() {
        let keyed = vec!["mistral".to_string(), "deepseek".to_string()];
        let catalog = llm_models::defaults();
        let europe = choice("europe", "mistral", "mistral-small-latest");
        let ok = validate(
            &[with_task(with_task(with_task(europe, "code", "mistral", "codestral-latest"), "text", "mistral", "mistral-small-latest"), "chat", "", "")],
            &keyed,
            &catalog,
        )
        .unwrap();
        let tasks = &ok["europe"].tasks;
        assert_eq!(tasks.len(), 1, "the region's own model and an empty choice are not kept");
        assert_eq!(tasks["code"], pick("mistral", "codestral-latest"));

        let europe = || choice("europe", "mistral", "mistral-small-latest");
        assert!(validate(&[with_task(europe(), "poetry", "mistral", "codestral-latest")], &keyed, &catalog).is_err());
        assert!(validate(&[with_task(europe(), "code", "mistral", "gpt-5")], &keyed, &catalog).is_err());
        assert!(validate(&[with_task(europe(), "code", "anthropic", "claude-opus-5-5")], &keyed, &catalog).is_err(), "no key");
        assert!(
            validate(&[with_task(choice("europe", "", ""), "code", "mistral", "codestral-latest")], &keyed, &catalog).is_err(),
            "a task needs the region's own model first"
        );
    }

    #[test]
    fn a_call_runs_on_its_task_model_else_the_regions() {
        let b = Backing {
            provider: "mistral".into(),
            model: "mistral-small-latest".into(),
            tasks: BTreeMap::from([
                ("code".to_string(), pick("mistral", "codestral-latest")),
                ("text".to_string(), pick("anthropic", "claude-sonnet-5-5")),
            ]),
        };
        let keyed = vec!["mistral".to_string()];
        assert_eq!(b.for_task(Some("code"), &keyed), ("mistral", "codestral-latest"));
        assert_eq!(b.for_task(Some("chat"), &keyed), ("mistral", "mistral-small-latest"));
        assert_eq!(b.for_task(None, &keyed), ("mistral", "mistral-small-latest"));
        assert_eq!(b.for_task(Some("text"), &keyed), ("mistral", "mistral-small-latest"), "its key is gone");
        assert_eq!(b.picks().count(), 3);
    }

    #[test]
    fn the_defaults_are_what_saving_them_would_keep() {
        let d = defaults();
        let keyed: Vec<String> = PROVIDERS.iter().map(|p| p.id.to_string()).collect();
        let choices: Vec<RegionChoice> = REGIONS
            .iter()
            .map(|r| {
                let b = &d[r.id];
                let mut c = choice(r.id, &b.provider, &b.model);
                for (t, p) in &b.tasks {
                    c = with_task(c, t, &p.provider, &p.model);
                }
                c
            })
            .collect();
        assert_eq!(validate(&choices, &keyed, &llm_models::defaults()).unwrap(), d);
        assert_eq!(d["usa"].for_task(Some("code"), &keyed), ("anthropic", "claude-haiku-5-5"));
        assert_eq!(d["china"].for_task(Some("code"), &keyed), ("qwen", "qwen3-coder-next"));
        assert_eq!(d["europe"].for_task(None, &keyed), ("mistral", "mistral-medium-latest"));
    }

    #[test]
    fn a_calls_task_is_the_one_it_names_else_its_apps() {
        assert_eq!(task_of(None, "gitagent"), Some("code"));
        assert_eq!(task_of(Some(" Text "), "gitagent"), Some("text"), "named wins");
        assert_eq!(task_of(Some("poetry"), "publishkit"), Some("text"), "unknown name: the app's");
        assert_eq!(task_of(None, "shipscreen"), None);
        assert_eq!(task_of(Some("chat"), "unknown"), Some("chat"));
    }
}
