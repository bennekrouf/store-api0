// src/mcp_tools_handler.rs
//
// HTTP handlers for the MCP tool registry.
//
// All write operations require X-Internal-Secret (service-to-service calls from
// provider backends like cvenom). Read/lookup endpoints are open — the gateway
// calls them after it has already verified the caller's API key.
//
// Routes (all under /api):
//   POST   /mcp-tools                            — upsert a tool
//   GET    /mcp-tools/{tenant_id}                — list tools for a tenant
//   GET    /mcp-tools/{tenant_id}/{tool_name}    — lookup single tool (used by gateway)
//   DELETE /mcp-tools/{tenant_id}/{tool_name}    — soft-delete a tool

use crate::app_log;
use crate::endpoint_store::mcp_tools_management::UpsertMcpToolRequest;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

fn check_internal_secret(req: &HttpRequest) -> bool {
    let expected = match std::env::var("API0_INTERNAL_SECRET") {
        Ok(s) if !s.is_empty() => s,
        _ => return false,
    };
    req.headers()
        .get("X-Internal-Secret")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == expected)
        .unwrap_or(false)
}

// ── POST /api/mcp-tools ───────────────────────────────────────────────────────
// Body: { tenant_id, tool_name, backend_url, description?, input_schema?,
//          cost_credits?, timeout_ms? }

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
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success":false,"error":"Unauthorized"}));
    }

    match store.upsert_mcp_tool(&body.tenant_id, &body.tool).await {
        Ok(tool) => {
            app_log!(info, tenant_id = %body.tenant_id, tool_name = %tool.tool_name, "MCP tool upserted");
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "tool": tool }))
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to upsert MCP tool");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":e.to_string()}))
        }
    }
}

// ── GET /api/mcp-tools/{tenant_id} ───────────────────────────────────────────

pub async fn list_mcp_tools_handler(
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    let tenant_id = path.into_inner();

    match store.list_mcp_tools(&tenant_id).await {
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
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
) -> impl Responder {
    let (tenant_id, tool_name) = path.into_inner();

    match store.get_mcp_tool(&tenant_id, &tool_name).await {
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
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success":false,"error":"Unauthorized"}));
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
