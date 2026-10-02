// src/admin/assistant_config.rs
//
// The AI provider behind the messaging assistant (Telegram, WhatsApp): one
// platform-wide setting the super admin chooses, used for every workspace.
//
//   GET    /api/admin/config/assistant               provider, model, which keys are set
//   PUT    /api/admin/config/assistant               { provider?, model? }
//   PUT    /api/admin/config/assistant/keys/{prov}   { api_key }
//   DELETE /api/admin/config/assistant/keys/{prov}
//   GET    /api/internal/assistant-config            the active provider with its key — the bridge
//
// All require X-Internal-Secret; the gateway restricts the admin routes to the
// super admin. A key is sealed with secret_box on the way in and is never
// returned by an admin route — only "set" or "not set". The bridge's internal
// read is the one place a key leaves, decrypted, on the internal network.
//
// Every provider here speaks Anthropic's Messages API (DeepSeek through its
// Anthropic-compatible endpoint), so the bridge needs one client, not one each.

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::infra::secret_box::{self, SecretContext};
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

/// Sealed keys are bound to this pseudo-tenant and purpose: a key cannot be
/// moved into a tenant's credentials and still decrypt.
const KEY_TENANT: &str = "platform";
const KEY_PURPOSE: &str = "assistant_llm_key";

struct Provider {
    id: &'static str,
    label: &'static str,
    base_url: &'static str,
    models: &'static [&'static str],
}

const PROVIDERS: &[Provider] = &[
    Provider {
        id: "claude",
        label: "Anthropic Claude",
        base_url: "https://api.anthropic.com",
        models: &["claude-sonnet-4-6", "claude-haiku-4-5-20251001", "claude-opus-4-7"],
    },
    Provider {
        id: "deepseek",
        label: "DeepSeek",
        base_url: "https://api.deepseek.com/anthropic",
        models: &["deepseek-chat", "deepseek-reasoner"],
    },
];

fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// The current choice, or `None` when the super admin has not set one (the
/// bridge then keeps using the key in its own configuration).
async fn read_choice(store: &EndpointStore) -> Result<Option<(String, String)>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let rows = client
        .query("SELECT key, value FROM system_config WHERE key IN ('assistant.provider', 'assistant.model')", &[])
        .await
        .map_err(|e| e.to_string())?;
    let mut prov = None;
    let mut model = None;
    for row in rows {
        let (k, v): (&str, &str) = (row.get(0), row.get(1));
        match k {
            "assistant.provider" => prov = Some(v.to_string()),
            "assistant.model" => model = Some(v.to_string()),
            _ => {}
        }
    }
    Ok(prov.zip(model))
}

async fn providers_with_key(store: &EndpointStore) -> Result<Vec<String>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    Ok(client
        .query("SELECT provider FROM assistant_llm_keys ORDER BY provider", &[])
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .map(|r| r.get(0))
        .collect())
}

fn error(status: actix_web::http::StatusCode, msg: impl Into<String>) -> HttpResponse {
    HttpResponse::build(status).json(serde_json::json!({ "success": false, "error": msg.into() }))
}

// ── GET /api/admin/config/assistant ──────────────────────────────────────────

pub async fn get_assistant_config(req: HttpRequest, store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let (choice, keys) = match (read_choice(&store).await, providers_with_key(&store).await) {
        (Ok(c), Ok(k)) => (c, k),
        (Err(e), _) | (_, Err(e)) => {
            app_log!(error, error = %e, "Could not read the assistant configuration");
            return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Could not read the configuration");
        }
    };
    let providers: Vec<serde_json::Value> = PROVIDERS
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "label": p.label,
                "models": p.models,
                "key_set": keys.iter().any(|k| k == p.id),
            })
        })
        .collect();
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        // null: not configured here — the bridge uses its own built-in key.
        "provider": choice.as_ref().map(|c| &c.0),
        "model": choice.as_ref().map(|c| &c.1),
        "providers": providers,
    }))
}

// ── PUT /api/admin/config/assistant ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ChoiceRequest {
    pub provider: String,
    pub model: String,
}

pub async fn update_assistant_config(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<ChoiceRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let Some(p) = provider(&body.provider) else {
        return error(actix_web::http::StatusCode::BAD_REQUEST, format!("Unknown provider '{}'", body.provider));
    };
    if !p.models.contains(&body.model.as_str()) {
        return error(
            actix_web::http::StatusCode::BAD_REQUEST,
            format!("'{}' is not a {} model: {}", body.model, p.label, p.models.join(", ")),
        );
    }
    // Switching to a provider with no key would take every bot offline.
    match providers_with_key(&store).await {
        Ok(keys) if !keys.iter().any(|k| k == p.id) => {
            return error(
                actix_web::http::StatusCode::BAD_REQUEST,
                format!("Add a {} API key before choosing it", p.label),
            )
        }
        Err(e) => {
            app_log!(error, error = %e, "Could not read assistant keys");
            return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Could not read the configuration");
        }
        _ => {}
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Database unavailable"),
    };
    for (key, value) in [("assistant.provider", p.id), ("assistant.model", body.model.as_str())] {
        if let Err(e) = client
            .execute(
                "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW())
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
                &[&key, &value],
            )
            .await
        {
            app_log!(error, error = %e, "Could not save the assistant configuration");
            return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Could not save the configuration");
        }
    }
    app_log!(info, provider = %p.id, model = %body.model, "Messaging assistant provider changed");
    HttpResponse::Ok().json(serde_json::json!({ "success": true, "provider": p.id, "model": body.model }))
}

// ── PUT / DELETE /api/admin/config/assistant/keys/{provider} ─────────────────

#[derive(Debug, Deserialize)]
pub struct KeyRequest {
    pub api_key: String,
}

pub async fn set_assistant_key(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
    body: web::Json<KeyRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let id = path.into_inner();
    let Some(p) = provider(&id) else {
        return error(actix_web::http::StatusCode::BAD_REQUEST, format!("Unknown provider '{}'", id));
    };
    let key = body.api_key.trim();
    if key.len() < 16 {
        return error(actix_web::http::StatusCode::BAD_REQUEST, "That does not look like an API key");
    }
    let sealed = match secret_box::seal(key, &SecretContext { tenant_id: KEY_TENANT, purpose: KEY_PURPOSE }) {
        Ok(s) => s,
        Err(e) => {
            // Refuse rather than store a key in the clear.
            app_log!(error, error = %e, "Could not seal an assistant API key");
            return error(
                actix_web::http::StatusCode::INTERNAL_SERVER_ERROR,
                "Could not encrypt the key — is API0_ENCRYPTION_KEY set?",
            );
        }
    };
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Database unavailable"),
    };
    if let Err(e) = client
        .execute(
            "INSERT INTO assistant_llm_keys (provider, key_enc, updated_at) VALUES ($1, $2, NOW())
             ON CONFLICT (provider) DO UPDATE SET key_enc = EXCLUDED.key_enc, updated_at = NOW()",
            &[&p.id, &sealed],
        )
        .await
    {
        app_log!(error, error = %e, "Could not save an assistant API key");
        return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Could not save the key");
    }
    app_log!(info, provider = %p.id, "Messaging assistant API key set");
    HttpResponse::Ok().json(serde_json::json!({ "success": true, "provider": p.id, "key_set": true }))
}

pub async fn delete_assistant_key(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let id = path.into_inner();
    // The active provider's key cannot be removed: that would stop every bot.
    if let Ok(Some((active, _))) = read_choice(&store).await {
        if active == id {
            return error(
                actix_web::http::StatusCode::BAD_REQUEST,
                "This provider is in use — choose another one before removing its key",
            );
        }
    }
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Database unavailable"),
    };
    match client.execute("DELETE FROM assistant_llm_keys WHERE provider = $1", &[&id]).await {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({ "success": true, "provider": id, "key_set": false })),
        Err(e) => {
            app_log!(error, error = %e, "Could not remove an assistant API key");
            error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Could not remove the key")
        }
    }
}

// ── GET /api/internal/assistant-config ── the bridge ─────────────────────────

pub async fn internal_assistant_config(req: HttpRequest, store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let choice = match read_choice(&store).await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "Could not read the assistant configuration");
            return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Could not read the configuration");
        }
    };
    let Some((id, model)) = choice else {
        return HttpResponse::Ok().json(serde_json::json!({ "success": true, "configured": false }));
    };
    let Some(p) = provider(&id) else {
        return HttpResponse::Ok().json(serde_json::json!({ "success": true, "configured": false }));
    };
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Database unavailable"),
    };
    let sealed: Option<Vec<u8>> = match client
        .query_opt("SELECT key_enc FROM assistant_llm_keys WHERE provider = $1", &[&p.id])
        .await
    {
        Ok(row) => row.map(|r| r.get(0)),
        Err(e) => {
            app_log!(error, error = %e, "Could not read an assistant API key");
            return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Could not read the key");
        }
    };
    let Some(sealed) = sealed else {
        return HttpResponse::Ok().json(serde_json::json!({ "success": true, "configured": false }));
    };
    let api_key = match secret_box::open(&sealed, &SecretContext { tenant_id: KEY_TENANT, purpose: KEY_PURPOSE }) {
        Ok(k) => k,
        Err(e) => {
            app_log!(error, error = %e, "Could not decrypt the assistant API key");
            return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Could not decrypt the key");
        }
    };
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "configured": true,
        "provider": p.id,
        "base_url": p.base_url,
        "model": model,
        "api_key": api_key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_has_models_and_an_anthropic_style_endpoint() {
        for p in PROVIDERS {
            assert!(!p.models.is_empty(), "{} has no models", p.id);
            assert!(p.base_url.starts_with("https://"), "{} base_url", p.id);
        }
        assert!(provider("deepseek").is_some());
        assert!(provider("openai").is_none());
    }
}
