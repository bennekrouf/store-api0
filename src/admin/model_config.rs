// src/admin/model_config.rs
//
// The AI provider and model behind the YAML import (the ai-uploader service).
//
//   GET  /api/admin/config/models  — requires X-Internal-Secret (gateway-facing)
//   PUT  /api/admin/config/models  — requires X-Internal-Secret (gateway-facing)
//   GET  /api/system/ai-config     — no auth (internal network, read-only, for ai-uploader)
//
// The gateway restricts the admin routes to the super admin. API keys are the
// shared ones in assistant_config.rs, set in the dashboard: the store hands the
// chosen provider, model and key to the uploader with every request
// (`uploader_llm`), so the uploader's host needs no key of its own. A key in the
// uploader's environment still works, as a fallback.

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

// DeepSeek is the default; Mistral the alternative.
const VALID_PROVIDERS: &[&str] = &["deepseek", "mistral"];

const VALID_DEEPSEEK_MODELS: &[&str] = &["deepseek-v4-pro", "deepseek-v4-flash", "deepseek-chat", "deepseek-reasoner"];
const VALID_MISTRAL_MODELS: &[&str] = &["mistral-medium-latest"];

const DEFAULT_PROVIDER: &str = "deepseek";
const DEFAULT_MODEL: &str = "deepseek-chat";

fn check_internal_secret(req: &HttpRequest) -> bool {
    let expected = match std::env::var("API0_INTERNAL_SECRET") {
        Ok(s) if !s.is_empty() => s,
        _ => return false,
    };
    req.headers()
        .get("X-Internal-Secret")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == expected)
        .unwrap_or(false)
}

fn default_ai_config() -> (String, String) {
    (DEFAULT_PROVIDER.into(), DEFAULT_MODEL.into())
}

pub(crate) async fn read_ai_config(store: &EndpointStore) -> (String, String) {
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return default_ai_config(),
    };
    let rows = match client
        .query(
            "SELECT key, value FROM system_config WHERE key LIKE 'ai_uploader.%'",
            &[],
        )
        .await
    {
        Ok(r) => r,
        Err(_) => return default_ai_config(),
    };

    let mut provider = DEFAULT_PROVIDER.to_string();
    let mut model = DEFAULT_MODEL.to_string();
    for row in rows {
        let key: &str = row.get(0);
        let value: &str = row.get(1);
        match key {
            "ai_uploader.provider" => provider = value.to_string(),
            "ai_uploader.model" => model = value.to_string(),
            _ => {}
        }
    }
    // A provider saved before Cohere and Claude were dropped falls back to the
    // default rather than handing the uploader a model nobody serves.
    if !VALID_PROVIDERS.contains(&provider.as_str()) {
        return default_ai_config();
    }
    (provider, model)
}

fn models_for(provider: &str) -> &'static [&'static str] {
    match provider {
        "deepseek" => VALID_DEEPSEEK_MODELS,
        "mistral" => VALID_MISTRAL_MODELS,
        _ => &[],
    }
}

/// What the uploader is to use for one request. `api_key` is `None` when the
/// super admin has not set one in the dashboard; the uploader then falls back
/// to its own environment.
pub struct UploaderLlm {
    pub provider: String,
    pub model: String,
    pub api_key: Option<String>,
}

pub async fn uploader_llm(store: &EndpointStore) -> UploaderLlm {
    let (provider, model) = read_ai_config(store).await;
    let api_key = match crate::admin::assistant_config::api_key(store, &provider).await {
        Ok(k) => k,
        Err(e) => {
            app_log!(error, error = %e, provider = %provider, "Could not read the YAML import's API key");
            None
        }
    };
    UploaderLlm { provider, model, api_key }
}

pub async fn get_model_config(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }
    let (provider, model) = read_ai_config(&store).await;
    let in_dashboard = match crate::admin::assistant_config::providers_with_key(&store).await {
        Ok(k) => k,
        Err(e) => {
            app_log!(error, error = %e, "Could not read the AI provider keys");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Could not read the configuration"}));
        }
    };
    // Keys still in the uploader's environment. None when it cannot be asked —
    // then only the dashboard's keys count.
    let on_server = usable_providers().await.unwrap_or_default();
    let providers: Vec<serde_json::Value> = VALID_PROVIDERS
        .iter()
        .map(|id| {
            let label = crate::admin::assistant_config::provider(id).map_or(*id, |p| p.label);
            let key_source = if in_dashboard.iter().any(|k| k == id) {
                Some("dashboard")
            } else if on_server.iter().any(|k| k == id) {
                Some("server")
            } else {
                None
            };
            serde_json::json!({
                "id": id,
                "label": label,
                "models": models_for(id),
                "key_set": key_source.is_some(),
                "key_source": key_source,
            })
        })
        .collect();
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "provider": provider,
        "model": model,
        "providers": providers,
    }))
}

#[derive(Debug, Deserialize)]
pub struct UpdateModelConfigRequest {
    pub provider: String,
    pub model: String,
}

/// Which providers have a key in the uploader's own environment — the way
/// keys were given before they could be set in the dashboard. None when the
/// uploader could not be reached.
async fn usable_providers() -> Option<Vec<String>> {
    let url = std::env::var("AI_UPLOADER_URL").ok()?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;

    let body: serde_json::Value = client
        .get(format!("{}/providers", url.trim_end_matches('/')))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;

    Some(
        body["providers"]
            .as_array()?
            .iter()
            .filter(|p| p["configured"].as_bool().unwrap_or(false))
            .filter_map(|p| p["provider"].as_str().map(str::to_string))
            .collect(),
    )
}

pub async fn update_model_config(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<UpdateModelConfigRequest>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }
    let bad = |msg: String| HttpResponse::BadRequest().json(serde_json::json!({"success": false, "error": msg}));

    if !VALID_PROVIDERS.contains(&body.provider.as_str()) {
        return bad(format!("Unknown provider '{}'. Valid: {}", body.provider, VALID_PROVIDERS.join(", ")));
    }
    let models = models_for(&body.provider);
    if !models.contains(&body.model.as_str()) {
        return bad(format!("'{}' is not a {} model: {}", body.model, body.provider, models.join(", ")));
    }

    // A provider without a key saves fine and then fails every import, in a
    // log nobody reads — so it is refused here.
    let in_dashboard = crate::admin::assistant_config::providers_with_key(&store)
        .await
        .unwrap_or_default()
        .contains(&body.provider);
    if !in_dashboard && !usable_providers().await.unwrap_or_default().contains(&body.provider) {
        app_log!(warn, provider = %body.provider, "Refused a YAML import provider with no API key");
        return bad(format!("Add a {} API key before choosing it", body.provider));
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, "DB error in update_model_config: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Database error"}));
        }
    };
    for (key, value) in [("ai_uploader.provider", &body.provider), ("ai_uploader.model", &body.model)] {
        if let Err(e) = client
            .execute(
                "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW())
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
                &[&key, value],
            )
            .await
        {
            app_log!(error, "Failed to update {}: {}", key, e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Update failed"}));
        }
    }

    app_log!(info, provider = %body.provider, model = %body.model, "YAML import provider changed");
    HttpResponse::Ok().json(serde_json::json!({"success": true, "provider": body.provider, "model": body.model}))
}

// Public read-only endpoint for internal services (ai-uploader, no auth). Never
// carries a key: the store sends that with each formatting request.
pub async fn get_ai_config_public(store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    let (provider, model) = read_ai_config(&store).await;
    HttpResponse::Ok().json(serde_json::json!({
        "provider": provider,
        "model": model,
    }))
}
