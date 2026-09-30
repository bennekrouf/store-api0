// src/mcp/downstream_oauth.rs
//
// HTTP surface for three-legged downstream OAuth. All behind X-Internal-Secret:
// only the gateway drives this flow, and it does so on behalf of a person it
// has already authenticated.
//
//   POST /internal/downstream-oauth/start   {tenant_id, email}
//                                           → {state, code_verifier}
//   POST /internal/downstream-oauth/redeem  {state}
//                                           → {tenant_id, user_email, code_verifier}
//   PUT  /internal/downstream-credential/{tenant_id}/{kind}?email=…
//                                           {secret, expires_at?, label?}
//
// The save route exists separately from the tenant-facing one because the
// callback knows the tenant id outright — it came from the redeemed state —
// whereas the tenant-facing route derives it from the signed-in user's own
// workspace. For a consumer those are different tenants, and deriving it here
// would store the token against the wrong one.

use crate::app_log;
use crate::endpoint_store::downstream_oauth::{redeem_authorization, start_authorization};
use crate::endpoint_store::user_credentials::{save_credential, SaveCredentialRequest};
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
            app_log!(error, error = %other, "Downstream OAuth operation failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": other.to_string()}))
        }
    }
}

#[derive(Deserialize)]
pub struct StartBody {
    pub tenant_id: String,
    pub email: String,
}

pub async fn start_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<StartBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match start_authorization(&store, &body.tenant_id, &body.email).await {
        Ok((state, code_verifier)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true, "state": state, "code_verifier": code_verifier
        })),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
pub struct RedeemBody {
    pub state: String,
}

pub async fn redeem_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<RedeemBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match redeem_authorization(&store, &body.state).await {
        Ok(Some(p)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "tenant_id": p.tenant_id,
            "user_email": p.user_email,
            "code_verifier": p.code_verifier,
        })),
        // Expired, already used, or never existed — all the same to the caller,
        // and deliberately not distinguished: telling an attacker which of those
        // it was is free information about somebody else's flow.
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "success": false,
            "error": "That authorization is no longer valid. Start again."
        })),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
pub struct SaveSecretBody {
    pub email: String,
    pub secret: String,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

pub async fn save_secret_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
    body: web::Json<SaveSecretBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let (tenant_id, kind) = path.into_inner();
    let r = SaveCredentialRequest {
        kind: Some(kind.clone()),
        secret: body.secret.clone(),
        label: body.label.clone(),
        expires_at: body.expires_at.clone(),
        // Only meaningful for a hand-pasted PAT, which the tenant can ask us to
        // verify against a "who is this token?" endpoint. An OAuth token's
        // identity is settled by the authorization itself.
        verified_identity: None,
    };
    match save_credential(&store, &tenant_id, &body.email, &r).await {
        Ok(summary) => {
            app_log!(info, tenant_id = %tenant_id, kind = %kind, "Stored a downstream OAuth token");
            HttpResponse::Ok().json(serde_json::json!({"success": true, "credential": summary}))
        }
        Err(e) => fail(e),
    }
}
