// src/mcp/instructions.rs
//
// HTTP surface for a tenant's MCP instructions. All behind X-Internal-Secret.
//
//   GET /mcp-instructions/{tenant_id}        — for the gateway's initialize
//   GET /user/mcp-instructions?email=…       — the caller's tenant (gateway-proxied)
//   PUT /user/mcp-instructions               — { email, instructions } (gateway-proxied)
//
// Both reads return the same context: the tenant's own text plus a summary of its
// tools. The gateway writes the prose, so the dashboard can show exactly what a
// model will receive.

use crate::app_log;
use crate::endpoint_store::mcp_instructions::{
    get_mcp_instructions_context, set_tenant_instructions, MAX_TENANT_INSTRUCTIONS_CHARS,
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
            app_log!(error, error = %other, "MCP instructions operation failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": other.to_string()}))
        }
    }
}

pub async fn get_mcp_instructions_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match get_mcp_instructions_context(&store, &path.into_inner()).await {
        Ok(ctx) => HttpResponse::Ok().json(serde_json::json!({"success": true, "context": ctx})),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
pub struct EmailQuery {
    pub email: String,
}

pub async fn get_my_mcp_instructions_handler(
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
    match get_mcp_instructions_context(&store, &tenant.id).await {
        Ok(ctx) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "context": ctx,
            "max_chars": MAX_TENANT_INSTRUCTIONS_CHARS
        })),
        Err(e) => fail(e),
    }
}

#[derive(Deserialize)]
pub struct SaveInstructionsBody {
    pub email: String,
    /// Absent, null or blank clears the tenant's text.
    #[serde(default)]
    pub instructions: Option<String>,
}

pub async fn save_my_mcp_instructions_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SaveInstructionsBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match set_tenant_instructions(&store, &body.email, body.instructions.as_deref()).await {
        Ok((tenant_id, stored)) => {
            app_log!(info, tenant_id = %tenant_id, cleared = stored.is_none(), "MCP instructions saved");
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "tenant_id": tenant_id,
                "instructions": stored
            }))
        }
        Err(e) => fail(e),
    }
}
