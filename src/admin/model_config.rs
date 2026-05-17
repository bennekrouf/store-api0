// src/admin/model_config.rs
//
// Admin endpoints for managing AI model configuration.
//
//   GET  /api/admin/config/models  — requires X-Internal-Secret (gateway-facing)
//   PUT  /api/admin/config/models  — requires X-Internal-Secret (gateway-facing)
//   GET  /api/system/ai-config     — no auth (internal network, read-only, for ai-uploader)

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const VALID_PROVIDERS: &[&str] = &["cohere"];
const VALID_MODELS: &[&str] = &[
    "command-r7b-12-2024",
    "command-r-08-2024",
    "command-a-03-2025",
];

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

async fn read_ai_config(store: &EndpointStore) -> (String, String) {
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return ("cohere".into(), "command-r7b-12-2024".into()),
    };
    let rows = match client
        .query(
            "SELECT key, value FROM system_config WHERE key LIKE 'ai_uploader.%'",
            &[],
        )
        .await
    {
        Ok(r) => r,
        Err(_) => return ("cohere".into(), "command-r7b-12-2024".into()),
    };

    let mut provider = "cohere".to_string();
    let mut model = "command-r7b-12-2024".to_string();
    for row in rows {
        let key: &str = row.get(0);
        let value: &str = row.get(1);
        match key {
            "ai_uploader.provider" => provider = value.to_string(),
            "ai_uploader.model" => model = value.to_string(),
            _ => {}
        }
    }
    (provider, model)
}

#[derive(Debug, Serialize)]
pub struct ModelConfigResponse {
    pub success: bool,
    pub config: ModelConfigEntry,
    pub available_models: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelConfigEntry {
    pub provider: String,
    pub model: String,
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
    HttpResponse::Ok().json(ModelConfigResponse {
        success: true,
        config: ModelConfigEntry { provider, model },
        available_models: serde_json::json!({
            "cohere": VALID_MODELS,
        }),
    })
}

#[derive(Debug, Deserialize)]
pub struct UpdateModelConfigRequest {
    pub provider: Option<String>,
    pub model: Option<String>,
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

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, "DB error in update_model_config: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Database error"}));
        }
    };

    if let Some(ref provider) = body.provider {
        if !VALID_PROVIDERS.contains(&provider.as_str()) {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": format!("Invalid provider '{}'. Valid: {:?}", provider, VALID_PROVIDERS),
            }));
        }
        if let Err(e) = client
            .execute(
                "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW())
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
                &[&"ai_uploader.provider", provider],
            )
            .await
        {
            app_log!(error, "Failed to update provider: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Update failed"}));
        }
    }

    if let Some(ref model) = body.model {
        if !VALID_MODELS.contains(&model.as_str()) {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": format!("Invalid model '{}'. Valid: {:?}", model, VALID_MODELS),
            }));
        }
        if let Err(e) = client
            .execute(
                "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW())
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
                &[&"ai_uploader.model", model],
            )
            .await
        {
            app_log!(error, "Failed to update model: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Update failed"}));
        }
    }

    app_log!(
        info,
        provider = ?body.provider, model = ?body.model,
        "Admin updated AI model config"
    );
    HttpResponse::Ok().json(serde_json::json!({"success": true, "message": "Updated"}))
}

// Public read-only endpoint for internal services (ai-uploader, no auth).
pub async fn get_ai_config_public(store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    let (provider, model) = read_ai_config(&store).await;
    HttpResponse::Ok().json(serde_json::json!({
        "provider": provider,
        "model": model,
    }))
}
