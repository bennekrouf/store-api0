// src/admin/assistant_config.rs
//
// The AI provider behind the messaging assistant (Telegram, WhatsApp, Teams):
// one platform-wide setting the super admin chooses, used for every workspace
// that has not chosen a region of its own (tenant_region.rs).
//
// The provider API keys kept here are api0's, shared by every AI feature: the
// YAML import (model_config.rs) reads them too, so a key is entered once, in
// the dashboard, and never in a server's environment.
//
//   GET    /api/admin/config/assistant               provider, model, which keys are set
//   PUT    /api/admin/config/assistant               { provider?, model? }
//   PUT    /api/admin/config/assistant/keys/{prov}   { api_key }
//   DELETE /api/admin/config/assistant/keys/{prov}
//   GET    /api/internal/assistant-config            the active provider with its key — the bridge;
//                                                    ?tenant_id= for a workspace's region (tenant_region.rs)
//
// All require X-Internal-Secret; the gateway restricts the admin routes to the
// super admin. A key is sealed with secret_box on the way in and is never
// returned by an admin route — only "set" or "not set". The bridge's internal
// read is the one place a key leaves, decrypted, on the internal network.
//
// Every provider here speaks the OpenAI-style chat-completions API, tool calls
// included, at `{base_url}{chat_path}` — `/v1/chat/completions` for most, so the
// bridge needs one client, not one each. Providers with no `models` here (OpenAI,
// Google, Qwen, Z.ai) only back the regions the desktop apps offer, with the
// models listed in llm_models.rs; the bridge never runs on them. DeepSeek is the default; Mistral and Anthropic (Claude,
// through its OpenAI-compatible endpoint) the alternatives. The same keys back
// the regions the desktop apps offer (llm_regions.rs). Mistral and OpenAI also
// take OpenAI-style audio transcriptions, which a region's speech model runs on
// (`transcription_path`).

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

pub(crate) struct Provider {
    pub id: &'static str,
    pub label: &'static str,
    pub base_url: &'static str,
    /// Where chat completions are, after `base_url`.
    pub chat_path: &'static str,
    /// The models the messaging assistant may choose; none for a provider
    /// that only backs regions.
    pub models: &'static [&'static str],
    /// Where OpenAI-style audio transcriptions are, after `base_url`, for a
    /// provider a region's speech model can run on (llm_regions).
    pub transcription_path: Option<&'static str>,
}

impl Provider {
    pub fn chat_url(&self) -> String {
        format!("{}{}", self.base_url, self.chat_path)
    }

    pub fn transcription_url(&self) -> Option<String> {
        self.transcription_path.map(|p| format!("{}{}", self.base_url, p))
    }
}

const CHAT_PATH: &str = "/v1/chat/completions";
const TRANSCRIPTION_PATH: &str = "/v1/audio/transcriptions";

pub(crate) const PROVIDERS: &[Provider] = &[
    Provider {
        id: "deepseek",
        label: "DeepSeek",
        base_url: "https://api.deepseek.com",
        chat_path: CHAT_PATH,
        models: &["deepseek-chat", "deepseek-reasoner"],
        transcription_path: None,
    },
    Provider {
        id: "mistral",
        label: "Mistral",
        base_url: "https://api.mistral.ai",
        chat_path: CHAT_PATH,
        models: &["mistral-medium-latest", "mistral-small-latest", "codestral-latest"],
        transcription_path: Some(TRANSCRIPTION_PATH),
    },
    // Not "claude": schema.sql moves a deployment still on the old Claude
    // provider, dropped before, back to DeepSeek.
    Provider {
        id: "anthropic",
        label: "Anthropic (Claude)",
        base_url: "https://api.anthropic.com",
        chat_path: CHAT_PATH,
        models: &["claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"],
        transcription_path: None,
    },
    Provider {
        id: "openai",
        label: "OpenAI",
        base_url: "https://api.openai.com",
        chat_path: CHAT_PATH,
        models: &[],
        transcription_path: Some(TRANSCRIPTION_PATH),
    },
    Provider {
        id: "google",
        label: "Google (Gemini)",
        base_url: "https://generativelanguage.googleapis.com",
        chat_path: "/v1beta/openai/chat/completions",
        models: &[],
        transcription_path: None,
    },
    Provider {
        id: "qwen",
        label: "Alibaba (Qwen)",
        base_url: "https://dashscope-intl.aliyuncs.com",
        chat_path: "/compatible-mode/v1/chat/completions",
        models: &[],
        transcription_path: None,
    },
    Provider {
        id: "zai",
        label: "Z.ai (GLM)",
        base_url: "https://api.z.ai",
        chat_path: "/api/paas/v4/chat/completions",
        models: &[],
        transcription_path: None,
    },
];

pub(crate) fn provider(id: &str) -> Option<&'static Provider> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// The current choice, or `None` when the super admin has not set one (the
/// bridge then keeps using the key in its own configuration).
pub(crate) async fn read_choice(store: &EndpointStore) -> Result<Option<(String, String)>, String> {
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

pub(crate) async fn providers_with_key(store: &EndpointStore) -> Result<Vec<String>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    Ok(client
        .query("SELECT provider FROM assistant_llm_keys ORDER BY provider", &[])
        .await
        .map_err(|e| e.to_string())?
        .iter()
        .map(|r| r.get(0))
        .collect())
}

/// A provider's API key, decrypted, or `None` when the super admin has not set
/// one. Only ever handed to an internal service — never to an admin route.
pub(crate) async fn api_key(store: &EndpointStore, provider: &str) -> Result<Option<String>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sealed: Option<Vec<u8>> = client
        .query_opt("SELECT key_enc FROM assistant_llm_keys WHERE provider = $1", &[&provider])
        .await
        .map_err(|e| e.to_string())?
        .map(|r| r.get(0));
    sealed
        .map(|s| secret_box::open(&s, &SecretContext { tenant_id: KEY_TENANT, purpose: KEY_PURPOSE }))
        .transpose()
        .map_err(|e| format!("could not decrypt the {} key: {}", provider, e))
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
    if p.models.is_empty() {
        return error(
            actix_web::http::StatusCode::BAD_REQUEST,
            format!("{} only backs the apps' regions, not the messaging assistant", p.label),
        );
    }
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
    // A key in use cannot be removed: that would stop every bot, or every
    // AI-assisted import.
    if let Ok(Some((active, _))) = read_choice(&store).await {
        if active == id {
            return error(
                actix_web::http::StatusCode::BAD_REQUEST,
                "The messaging assistant uses this provider — choose another one there before removing its key",
            );
        }
    }
    if let Some(region) = crate::admin::llm_regions::uses_provider(&store, &id).await {
        return error(
            actix_web::http::StatusCode::BAD_REQUEST,
            format!("The {} region uses this provider — put another one behind it before removing its key", region),
        );
    }
    if crate::admin::model_config::read_ai_config(&store).await.0 == id {
        return error(
            actix_web::http::StatusCode::BAD_REQUEST,
            "The YAML import uses this provider — choose another one there before removing its key",
        );
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
//
// With `?tenant_id=`, the provider that workspace's assistant answers with: its
// region's (tenant_region.rs) when it chose one on offer, else the platform's.

#[derive(Debug, Deserialize)]
pub struct InternalConfigQuery {
    #[serde(default)]
    pub tenant_id: Option<String>,
}

pub async fn internal_assistant_config(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<InternalConfigQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let by_region = match query.tenant_id.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => match crate::admin::tenant_region::resolve(&store, t).await {
            Ok(r) => r,
            Err(e) => {
                // The platform's provider still answers rather than none.
                app_log!(warn, error = %e, tenant_id = %t, "Could not read the workspace's region");
                None
            }
        },
        None => None,
    };
    let (region, choice) = match by_region {
        Some((r, p, m)) => (Some(r.id), Some((p, m))),
        None => match read_choice(&store).await {
            Ok(c) => (None, c),
            Err(e) => {
                app_log!(error, error = %e, "Could not read the assistant configuration");
                return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Could not read the configuration");
            }
        },
    };
    let Some((id, model)) = choice else {
        return HttpResponse::Ok().json(serde_json::json!({ "success": true, "configured": false }));
    };
    let Some(p) = provider(&id) else {
        return HttpResponse::Ok().json(serde_json::json!({ "success": true, "configured": false }));
    };
    let api_key = match api_key(&store, p.id).await {
        Ok(Some(k)) => k,
        Ok(None) => return HttpResponse::Ok().json(serde_json::json!({ "success": true, "configured": false })),
        Err(e) => {
            app_log!(error, error = %e, "Could not read the assistant API key");
            return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Could not read the key");
        }
    };
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "configured": true,
        "provider": p.id,
        "base_url": p.base_url,
        "chat_path": p.chat_path,
        "model": model,
        "region": region,
        "api_key": api_key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_provider_speaks_chat_completions_and_three_serve_the_assistant() {
        for p in PROVIDERS {
            assert!(p.base_url.starts_with("https://"), "{} base_url", p.id);
            assert!(p.chat_path.ends_with("/chat/completions"), "{} chat_path", p.id);
        }
        for id in ["deepseek", "mistral", "anthropic"] {
            assert!(!provider(id).unwrap().models.is_empty(), "{} has no assistant models", id);
        }
        assert_eq!(provider("google").unwrap().chat_url(), "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions");
        assert_eq!(PROVIDERS[0].id, "deepseek", "the first provider is the default");
        assert!(provider("mistral").is_some());
        assert!(provider("anthropic").is_some());
        assert!(provider("claude").is_none(), "the old id is migrated away in schema.sql");
        assert!(provider("openai").unwrap().models.is_empty(), "regions only");
    }
}
