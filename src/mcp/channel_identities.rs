// src/mcp/channel_identities.rs
//
// HTTP surface for messaging-channel identities. All behind X-Internal-Secret.
//
// Person-facing (gateway binds the email from a verified session):
//   POST   /user/channel-link-code                          — mint a code
//   GET    /user/channel-identities?email=…                 — what is linked
//   DELETE /user/channel-identities/{channel}/{external_id}?email=…
//
// Bridge-facing:
//   POST   /internal/channel-identities/redeem   {channel, external_id, tenant_id, code}
//   GET    /internal/channel-identities/resolve?channel=…&external_id=…&tenant_id=…
//                                                → user_email + the api0 key

use crate::app_log;
use crate::endpoint_store::channel_identities::{
    create_link_code, list_identities, redeem_link_code, resolve_identity, unlink_identity,
};
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
            app_log!(error, error = %other, "Channel identity operation failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": other.to_string()}))
        }
    }
}

#[derive(Deserialize)]
pub struct EmailBody {
    pub email: String,
}

pub async fn create_link_code_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<EmailBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match create_link_code(&store, &body.email).await {
        Ok((code, expires_at)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true, "code": code, "expires_at": expires_at
        })),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
pub struct EmailQuery {
    pub email: String,
}

pub async fn list_identities_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match list_identities(&store, &query.email).await {
        Ok(list) => HttpResponse::Ok().json(serde_json::json!({"success": true, "identities": list})),
        Err(e) => fail(e),
    }
}

pub async fn unlink_identity_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let (channel, external_id) = path.into_inner();
    match unlink_identity(&store, &query.email, &channel, &external_id).await {
        Ok(true) => HttpResponse::Ok().json(serde_json::json!({"success": true})),
        Ok(false) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "Nothing linked there"})),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
pub struct RedeemBody {
    pub channel: String,
    pub external_id: String,
    pub tenant_id: String,
    pub code: String,
}

pub async fn redeem_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<RedeemBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match redeem_link_code(&store, &body.channel, &body.external_id, &body.tenant_id, &body.code).await {
        Ok(identity) => HttpResponse::Ok().json(serde_json::json!({"success": true, "identity": identity})),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
pub struct ResolveQuery {
    pub channel: String,
    pub external_id: String,
    pub tenant_id: String,
}

pub async fn resolve_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<ResolveQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match resolve_identity(&store, &query.channel, &query.external_id, &query.tenant_id).await {
        Ok(Some(id)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true, "user_email": id.user_email, "api_key": id.api_key
        })),
        Ok(None) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "Not linked"})),
        Err(e) => fail(e),
    }
}
