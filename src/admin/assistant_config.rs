// src/admin/assistant_config.rs
//
// api0's AI provider API keys, shared by every AI feature: the regions the
// desktop apps and the messaging assistant answer from (llm_regions.rs,
// tenant_region.rs) and the YAML import (model_config.rs). A key is entered
// once, in the dashboard (Admin → AI models), and never in a server's
// environment. Which provider and model answer is chosen per region there.
//
//   PUT    /api/admin/config/assistant/keys/{prov}   { api_key }
//   DELETE /api/admin/config/assistant/keys/{prov}
//   GET    /api/internal/assistant-config            the provider of the region the messaging
//                                                    assistant answers from, with its key — the bridge;
//                                                    ?tenant_id= for a workspace (tenant_region.rs)
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

use crate::admin::tenant_region::AssistantRegion;
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
    // A key in use cannot be removed: that would stop a region, or every
    // AI-assisted import.
    if let Some(region) = crate::admin::llm_regions::uses_provider(&store, &id).await {
        return error(
            actix_web::http::StatusCode::BAD_REQUEST,
            format!("The {} region uses this provider — put another one behind it before removing its key", region),
        );
    }
    if crate::admin::model_config::read_ai_config(&store).await.0 == id {
        return error(
            actix_web::http::StatusCode::BAD_REQUEST,
            "The YAML import uses this provider — choose another model for it in Admin → AI models before removing its key",
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
// The provider of the region the workspace (`?tenant_id=`) chose, else of the
// default region (tenant_region.rs). A region that cannot answer comes back as
// `region_unavailable`, never as another provider. A region that cannot be read
// is an error for the same reason.

#[derive(Debug, Deserialize)]
pub struct InternalConfigQuery {
    #[serde(default)]
    pub tenant_id: Option<String>,
}

fn not_configured(region: Option<&str>) -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "configured": false,
        "region_unavailable": region,
    }))
}

pub async fn internal_assistant_config(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<InternalConfigQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant = query.tenant_id.as_deref().filter(|t| !t.is_empty());
    let (region, choice) = match crate::admin::tenant_region::resolve(&store, tenant).await {
        Ok(AssistantRegion::Live { region, provider, model }) => (Some(region.id), Some((provider, model))),
        Ok(AssistantRegion::Unavailable(r)) => return not_configured(Some(r.id)),
        Err(e) => {
            app_log!(error, error = %e, tenant_id = ?tenant, "Could not read the assistant's region");
            return error(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, "Could not read the region");
        }
    };
    let Some((id, model)) = choice else {
        return not_configured(region);
    };
    let Some(p) = provider(&id) else {
        return not_configured(region);
    };
    let api_key = match api_key(&store, p.id).await {
        Ok(Some(k)) => k,
        Ok(None) => return not_configured(region),
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
