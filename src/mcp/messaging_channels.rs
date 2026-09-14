// src/mcp/messaging_channels.rs
//
// HTTP surface for messaging channels. All behind X-Internal-Secret.
//
//   PUT    /user/messaging-channels               — register (gateway-proxied)
//   GET    /user/messaging-channels?email=…       — the caller's channels
//   DELETE /user/messaging-channels/{channel}?email=…
//   GET    /internal/messaging-channels/{channel}/{channel_ref}  — for the bridge

use crate::app_log;
use crate::endpoint_store::messaging_channels::{
    channel_for_bridge, delete_channel, list_channels, register_channel, RegisterChannel,
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
            app_log!(error, error = %other, "Messaging channel operation failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": other.to_string()}))
        }
    }
}

#[derive(Deserialize)]
pub struct RegisterBody {
    pub email: String,
    pub channel: String,
    pub channel_ref: String,
    pub credential: String,
    #[serde(default)]
    pub display_ref: String,
    pub webhook_secret: String,
    #[serde(default)]
    pub system_prompt: String,
}

pub async fn register_channel_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<RegisterBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant = match get_default_tenant(&store, &body.email).await {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let r = RegisterChannel {
        channel: &body.channel,
        channel_ref: &body.channel_ref,
        credential: &body.credential,
        display_ref: &body.display_ref,
        webhook_secret: &body.webhook_secret,
        system_prompt: &body.system_prompt,
    };
    match register_channel(&store, &tenant.id, r).await {
        Ok(c) => HttpResponse::Ok().json(serde_json::json!({"success": true, "channel": c})),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
pub struct EmailQuery {
    pub email: String,
}

pub async fn list_channels_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant = match get_default_tenant(&store, &query.email).await {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    match list_channels(&store, &tenant.id).await {
        Ok(list) => HttpResponse::Ok().json(serde_json::json!({"success": true, "channels": list})),
        Err(e) => fail(e),
    }
}

pub async fn delete_channel_handler(
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
        Err(e) => return fail(e),
    };
    match delete_channel(&store, &tenant.id, &path.into_inner()).await {
        // The credential goes back to the gateway so it can deregister the
        // webhook with the platform. This is the last time it is ever read.
        Ok(Some(credential)) => {
            HttpResponse::Ok().json(serde_json::json!({"success": true, "credential": credential}))
        }
        Ok(None) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "No such channel"})),
        Err(e) => fail(e),
    }
}

pub async fn channel_for_bridge_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let (channel, channel_ref) = path.into_inner();
    match channel_for_bridge(&store, &channel, &channel_ref).await {
        Ok(Some(c)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "tenant_id": c.tenant_id,
            "credential": c.credential,
            "webhook_secret": c.webhook_secret,
            "system_prompt": c.system_prompt,
        })),
        Ok(None) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "Unknown channel"})),
        Err(e) => fail(e),
    }
}
