// src/mcp/idp.rs
//
// Internal HTTP surface for tenant identity providers. Every route requires
// X-Internal-Secret: one returns a decrypted client secret, and the others
// hold the state of in-flight sign-ins.
//
//   GET  /internal/tenant-idp/{mcp_client_id}   — issuer, client id, secret
//   POST /internal/idp-auth-request             — remember a PKCE verifier
//   POST /internal/idp-auth-request/consume     — take it, once
//   PUT  /user/tenant-idp                       — configure (gateway-proxied)

use crate::app_log;
use crate::endpoint_store::idp_management::{
    consume_auth_request, get_idp_by_mcp_client_id, remember_auth_request, save_idp, SaveIdpRequest,
};
use crate::endpoint_store::tenant_management::get_default_tenant;
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

fn fail(e: StoreError) -> HttpResponse {
    match e {
        StoreError::InvalidInput(msg) => {
            HttpResponse::BadRequest().json(serde_json::json!({"success": false, "error": msg}))
        }
        other => {
            app_log!(error, error = %other, "Tenant IdP operation failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": other.to_string()}))
        }
    }
}

pub async fn get_tenant_idp_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    match get_idp_by_mcp_client_id(&store, &path.into_inner()).await {
        Ok(Some(idp)) => HttpResponse::Ok().json(serde_json::json!({"success": true, "idp": idp})),
        Ok(None) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "No IdP for that client"})),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
pub struct RememberRequest {
    pub state_nonce: String,
    pub tenant_id: String,
    pub verifier: String,
}

pub async fn remember_auth_request_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<RememberRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    match remember_auth_request(&store, &body.state_nonce, &body.tenant_id, &body.verifier).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({"success": true})),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
pub struct ConsumeRequest {
    pub state_nonce: String,
}

pub async fn consume_auth_request_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<ConsumeRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    match consume_auth_request(&store, &body.state_nonce).await {
        Ok(Some(verifier)) => {
            HttpResponse::Ok().json(serde_json::json!({"success": true, "verifier": verifier}))
        }
        // Expired, replayed, or never issued — all the same to the caller.
        Ok(None) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "Unknown or expired sign-in"})),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
pub struct SaveIdpForCaller {
    pub email: String,
    #[serde(flatten)]
    pub idp: SaveIdpRequest,
}

pub async fn save_tenant_idp_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SaveIdpForCaller>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let tenant = match get_default_tenant(&store, &body.email).await {
        Ok(t) => t,
        Err(e) => return fail(e),
    };

    match save_idp(&store, &tenant.id, &body.idp).await {
        Ok(()) => HttpResponse::Ok().json(serde_json::json!({"success": true})),
        Err(e) => fail(e),
    }
}
