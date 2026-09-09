// src/mcp/user_credentials.rs
//
// HTTP handlers for per-user downstream credentials.
//
// Every route requires X-Internal-Secret: the gateway is the only caller, and it
// binds the email to a verified user before proxying. The one route that returns
// a decrypted secret is deliberately separate and named for what it does.
//
//   GET    /user/downstream-credentials?email=…        — metadata, never secrets
//   PUT    /user/downstream-credentials                — store or replace one
//   DELETE /user/downstream-credentials/{kind}?email=… — remove one
//   GET    /internal/downstream-credential/{tenant_id}/{kind}?email=…
//                                                      — the secret, for the gateway

use crate::app_log;
use crate::endpoint_store::tenant_management::get_default_tenant;
use crate::endpoint_store::user_credentials::{
    count_credentials, credential_slots, delete_credential, get_secret, require_tenant_access,
    save_credential, SaveCredentialRequest,
};
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::infra::secret_box;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct EmailQuery {
    pub email: String,
    /// Which workspace the credential belongs to. Omitted → the caller's own.
    ///
    /// This is the whole point of the parameter: a consumer's credential must
    /// live in the *provider's* tenant, because that is where the gateway looks
    /// for it when their tools run. Defaulting to the caller's own tenant is
    /// right only for someone whose own tenant owns the tools.
    pub tenant_id: Option<String>,
}

#[derive(Deserialize)]
pub struct SaveForCallerRequest {
    pub email: String,
    /// See EmailQuery::tenant_id.
    pub tenant_id: Option<String>,
    #[serde(flatten)]
    pub credential: SaveCredentialRequest,
}

/// The tenant a credential operation targets: the one asked for, once the
/// caller is shown to have access to it, or their own by default.
async fn target_tenant(
    store: &Arc<EndpointStore>,
    email: &str,
    requested: Option<&str>,
) -> Result<String, HttpResponse> {
    match requested.map(str::trim).filter(|t| !t.is_empty()) {
        Some(tenant_id) => match require_tenant_access(store, email, tenant_id).await {
            Ok(()) => Ok(tenant_id.to_string()),
            Err(e) => Err(store_error_response(e)),
        },
        None => match get_default_tenant(store, email).await {
            Ok(t) => Ok(t.id),
            Err(e) => Err(store_error_response(e)),
        },
    }
}

fn store_error_response(e: StoreError) -> HttpResponse {
    match e {
        StoreError::InvalidInput(msg) => {
            HttpResponse::BadRequest().json(serde_json::json!({"success": false, "error": msg}))
        }
        other => {
            app_log!(error, error = %other, "User credential operation failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": other.to_string()}))
        }
    }
}

pub async fn list_credentials_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let tenant = match get_default_tenant(&store, &query.email).await {
        Ok(t) => t,
        Err(e) => return store_error_response(e),
    };

    // One row per workspace the caller can reach, so somebody who belongs to a
    // provider's tenant can see — and fill — the slot that actually matters.
    let slots = match credential_slots(&store, &query.email).await {
        Ok(s) => s,
        Err(e) => return store_error_response(e),
    };

    // Kept for callers that only want their own credentials, unchanged in shape.
    let credentials: Vec<_> = slots
        .iter()
        .filter(|s| s.tenant_id == tenant.id)
        .filter_map(|s| s.credential.clone())
        .collect();

    // How many people are set up, so an owner can tell whether the workspace is
    // ready without being shown who is and is not.
    let people_configured = count_credentials(&store, &tenant.id).await.unwrap_or(0);

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tenant_id": tenant.id,
        "tenant_name": tenant.name,
        "credentials": credentials,
        "workspaces": slots,
        "people_configured": people_configured
    }))
}

pub async fn save_credential_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SaveForCallerRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    // Refuse before touching the database rather than writing something we
    // cannot protect. A deployment without a key should say so, once, clearly.
    if !secret_box::is_configured() {
        app_log!(error, "Refusing to store a credential: API0_ENCRYPTION_KEY is not set");
        return HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "success": false,
            "error": "Credential storage is not configured on this deployment"
        }));
    }

    let tenant_id = match target_tenant(&store, &body.email, body.tenant_id.as_deref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };

    match save_credential(&store, &tenant_id, &body.email, &body.credential).await {
        Ok(summary) => HttpResponse::Ok()
            .json(serde_json::json!({"success": true, "credential": summary})),
        Err(e) => store_error_response(e),
    }
}

pub async fn delete_credential_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let tenant_id = match target_tenant(&store, &query.email, query.tenant_id.as_deref()).await {
        Ok(id) => id,
        Err(resp) => return resp,
    };
    let kind = path.into_inner();

    match delete_credential(&store, &tenant_id, &query.email, &kind).await {
        Ok(true) => HttpResponse::Ok().json(serde_json::json!({"success": true})),
        Ok(false) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "No such credential"})),
        Err(e) => store_error_response(e),
    }
}

/// The decrypted secret, for the gateway to put in a downstream request.
///
/// Keyed by tenant id rather than resolved from the email, because the gateway
/// already knows which tenant's tools it is serving — for a consumer key that is
/// the provider's tenant, not the caller's own.
pub async fn get_credential_secret_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let (tenant_id, kind) = path.into_inner();

    match get_secret(&store, &tenant_id, &query.email, &kind).await {
        Ok(Some(secret)) => {
            HttpResponse::Ok().json(serde_json::json!({"success": true, "secret": secret}))
        }
        Ok(None) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "No credential for this user"})),
        Err(e) => store_error_response(e),
    }
}

// ── POST /api/internal/encrypt-legacy-secrets ────────────────────────────────
//
// The one-shot backfill. Not wired into startup on purpose: it rewrites live
// credentials, so it runs when a human decides it should, after a backup.

pub async fn encrypt_legacy_secrets_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    app_log!(warn, "Starting the legacy secret encryption backfill");

    match crate::endpoint_store::encrypt_legacy::encrypt_legacy_secrets(&store).await {
        Ok(report) => HttpResponse::Ok().json(serde_json::json!({
            "success": report.failures.is_empty(),
            "report": report
        })),
        Err(e) => store_error_response(e),
    }
}
