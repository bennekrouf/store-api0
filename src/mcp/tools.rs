// src/mcp_tools_handler.rs
//
// HTTP handlers for the MCP tool registry.
//
// Every route here requires X-Internal-Secret. The reads too: a tool record
// carries its backend_url and static_headers, and those hold credentials. The
// gateway is the only caller, and it verifies the user before proxying.
//
// Routes (all under /api):
//   POST   /mcp-tools                            — upsert a tool
//   GET    /mcp-tools/{tenant_id}                — list tools for a tenant
//   GET    /mcp-tools/{tenant_id}/{tool_name}    — lookup single tool (used by gateway)
//   DELETE /mcp-tools/{tenant_id}/{tool_name}    — soft-delete a tool
//
// And the tenant-facing half, keyed by the caller's email instead of a tenant id
// so a tenant can register its own tools without the internal secret. The
// gateway authenticates the caller and binds the email before proxying here:
//
//   GET    /user/mcp-tools?email=...             — list the caller's tools
//   PUT    /user/mcp-tools                       — upsert one of the caller's tools
//   DELETE /user/mcp-tools/{tool_name}?email=... — soft-delete one

use crate::app_log;
use crate::endpoint_store::mcp_tools_management::UpsertMcpToolRequest;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

use crate::middleware::internal_secret::require_internal_secret;

// ── POST /api/mcp-tools ───────────────────────────────────────────────────────
// Body: { tenant_id, tool_name, backend_url, description?, input_schema?,
//          cost_credits?, timeout_ms?, http_verb?,
//          content_type?, body_template?, static_headers?, forward_identity? }

#[derive(Deserialize)]
pub struct UpsertWithTenantRequest {
    pub tenant_id: String,
    #[serde(flatten)]
    pub tool: UpsertMcpToolRequest,
}

pub async fn upsert_mcp_tool_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<UpsertWithTenantRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    match store.upsert_mcp_tool(&body.tenant_id, &body.tool).await {
        Ok(tool) => {
            app_log!(info, tenant_id = %body.tenant_id, tool_name = %tool.tool_name, "MCP tool upserted");
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "tool": tool }))
        }
        // A malformed body_template is the caller's mistake, not the store's.
        Err(crate::endpoint_store::StoreError::InvalidInput(msg)) => {
            app_log!(warn, tenant_id = %body.tenant_id, error = %msg, "Rejected MCP tool");
            HttpResponse::BadRequest().json(serde_json::json!({"success":false,"error":msg}))
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to upsert MCP tool");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":e.to_string()}))
        }
    }
}

#[derive(Deserialize)]
pub struct McpQuery {
    pub email: Option<String>,
}

// ── GET /api/mcp-tools/{tenant_id} ───────────────────────────────────────────

pub async fn list_mcp_tools_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
    query: web::Query<McpQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let tenant_id = path.into_inner();

    match store.list_mcp_tools(&tenant_id, query.email.as_deref()).await {
        Ok(tools) => HttpResponse::Ok().json(serde_json::json!({ "tools": tools })),
        Err(e) => {
            app_log!(error, tenant_id = %tenant_id, error = %e, "Failed to list MCP tools");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":e.to_string()}))
        }
    }
}

// ── GET /api/mcp-tools/{tenant_id}/{tool_name} ───────────────────────────────

pub async fn get_mcp_tool_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
    query: web::Query<McpQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let (tenant_id, tool_name) = path.into_inner();

    match store.get_mcp_tool(&tenant_id, &tool_name, query.email.as_deref()).await {
        Ok(Some(tool)) => HttpResponse::Ok().json(tool),
        Ok(None) => HttpResponse::NotFound()
            .json(serde_json::json!({"success":false,"error":"Tool not found"})),
        Err(e) => {
            app_log!(error, error = %e, "Failed to get MCP tool");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":e.to_string()}))
        }
    }
}

// ── DELETE /api/mcp-tools/{tenant_id}/{tool_name} ────────────────────────────

pub async fn delete_mcp_tool_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let (tenant_id, tool_name) = path.into_inner();

    match store.delete_mcp_tool(&tenant_id, &tool_name).await {
        Ok(true) => HttpResponse::Ok().json(serde_json::json!({"success":true})),
        Ok(false) => HttpResponse::NotFound()
            .json(serde_json::json!({"success":false,"error":"Tool not found"})),
        Err(e) => {
            app_log!(error, error = %e, "Failed to delete MCP tool");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":e.to_string()}))
        }
    }
}

// ── Tenant-facing: /api/user/mcp-tools ───────────────────────────────────────
//
// Same registry, reached as "my tools". The email is trusted because only the
// gateway can reach the store, and it binds the email to the verified caller.

use crate::endpoint_store::tenant_management::get_default_tenant;

#[derive(Deserialize)]
pub struct EmailQuery {
    pub email: String,
}

#[derive(Deserialize)]
pub struct UpsertForCallerRequest {
    pub email: String,
    #[serde(flatten)]
    pub tool: UpsertMcpToolRequest,
}

async fn caller_tenant(
    store: &Arc<EndpointStore>,
    email: &str,
) -> Result<crate::endpoint_store::models::Tenant, HttpResponse> {
    get_default_tenant(store, email).await.map_err(|e| {
        app_log!(error, email = %email, error = %e, "mcp-tools: tenant lookup failed");
        HttpResponse::InternalServerError()
            .json(serde_json::json!({"success":false,"error":"Tenant not found"}))
    })
}

pub async fn list_my_mcp_tools_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let tenant = match caller_tenant(&store, &query.email).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    match store.list_mcp_tools(&tenant.id, Some(&query.email)).await {
        Ok(tools) => HttpResponse::Ok()
            .json(serde_json::json!({"success": true, "tenant_id": tenant.id, "tools": tools})),
        Err(e) => {
            app_log!(error, error = %e, "Failed to list the caller's MCP tools");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":e.to_string()}))
        }
    }
}

pub async fn upsert_my_mcp_tool_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<UpsertForCallerRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let tenant = match caller_tenant(&store, &body.email).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };

    match store.upsert_mcp_tool(&tenant.id, &body.tool).await {
        Ok(tool) => {
            app_log!(info, tenant_id = %tenant.id, tool_name = %tool.tool_name, "Tenant upserted its MCP tool");
            HttpResponse::Ok().json(serde_json::json!({"success": true, "tool": tool}))
        }
        Err(crate::endpoint_store::StoreError::InvalidInput(msg)) => {
            app_log!(warn, tenant_id = %tenant.id, error = %msg, "Rejected MCP tool");
            HttpResponse::BadRequest().json(serde_json::json!({"success":false,"error":msg}))
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to upsert the caller's MCP tool");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":e.to_string()}))
        }
    }
}

pub async fn delete_my_mcp_tool_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let tenant = match caller_tenant(&store, &query.email).await {
        Ok(t) => t,
        Err(resp) => return resp,
    };
    let tool_name = path.into_inner();

    match store.delete_mcp_tool(&tenant.id, &tool_name).await {
        Ok(true) => HttpResponse::Ok().json(serde_json::json!({"success": true})),
        Ok(false) => HttpResponse::NotFound()
            .json(serde_json::json!({"success":false,"error":"Tool not found"})),
        Err(e) => {
            app_log!(error, error = %e, "Failed to delete the caller's MCP tool");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":e.to_string()}))
        }
    }
}
