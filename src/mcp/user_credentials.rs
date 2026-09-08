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
    delete_credential, get_secret, list_credentials, save_credential, SaveCredentialRequest,
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
}

#[derive(Deserialize)]
pub struct SaveForCallerRequest {
    pub email: String,
    #[serde(flatten)]
    pub credential: SaveCredentialRequest,
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

    match list_credentials(&store, &tenant.id, &query.email).await {
        Ok(credentials) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "tenant_id": tenant.id,
            "credentials": credentials
        })),
        Err(e) => store_error_response(e),
    }
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

    let tenant = match get_default_tenant(&store, &body.email).await {
        Ok(t) => t,
        Err(e) => return store_error_response(e),
    };

    match save_credential(&store, &tenant.id, &body.email, &body.credential).await {
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

    let tenant = match get_default_tenant(&store, &query.email).await {
        Ok(t) => t,
        Err(e) => return store_error_response(e),
    };
    let kind = path.into_inner();

    match delete_credential(&store, &tenant.id, &query.email, &kind).await {
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
