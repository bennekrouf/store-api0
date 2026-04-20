// src/endpoint_store/mcp_tools_management.rs
//
// CRUD for the mcp_tools table.
// Called by mcp_tools_handler (HTTP) and the gateway via /api/mcp-tools/* endpoints.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub id: String,
    pub tenant_id: String,
    pub tool_name: String,
    pub backend_url: String,
    pub description: String,
    pub input_schema: String, // JSON Schema as text
    pub cost_credits: i64,
    pub timeout_ms: i32,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct UpsertMcpToolRequest {
    pub tool_name: String,
    pub backend_url: String,
    pub description: Option<String>,
    pub input_schema: Option<String>,
    pub cost_credits: Option<i64>,
    pub timeout_ms: Option<i32>,
}

pub async fn upsert_mcp_tool(
    store: &EndpointStore,
    tenant_id: &str,
    req: &UpsertMcpToolRequest,
) -> Result<McpTool, StoreError> {
    let client = store.get_conn().await?;
    let now = Utc::now();
    let id = Uuid::new_v4().to_string();

    let description = req.description.as_deref().unwrap_or("");
    let input_schema = req
        .input_schema
        .as_deref()
        .unwrap_or(r#"{"type":"object","properties":{}}"#);
    let cost_credits = req.cost_credits.unwrap_or(1);
    let timeout_ms = req.timeout_ms.unwrap_or(30000);

    // INSERT ... ON CONFLICT(tenant_id, tool_name) DO UPDATE
    let row = client
        .query_one(
            "INSERT INTO mcp_tools
                (id, tenant_id, tool_name, backend_url, description,
                 input_schema, cost_credits, timeout_ms, is_active, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, true, $9, $9)
             ON CONFLICT (tenant_id, tool_name) DO UPDATE SET
                backend_url  = EXCLUDED.backend_url,
                description  = EXCLUDED.description,
                input_schema = EXCLUDED.input_schema,
                cost_credits = EXCLUDED.cost_credits,
                timeout_ms   = EXCLUDED.timeout_ms,
                is_active    = true,
                updated_at   = EXCLUDED.updated_at
             RETURNING id, tenant_id, tool_name, backend_url, description,
                       input_schema, cost_credits, timeout_ms, is_active,
                       created_at, updated_at",
            &[
                &id, &tenant_id, &req.tool_name, &req.backend_url,
                &description, &input_schema, &cost_credits, &timeout_ms, &now,
            ],
        )
        .await
        .to_store_error()?;

    app_log!(
        info,
        tenant_id = %tenant_id,
        tool_name = %req.tool_name,
        "Upserted MCP tool"
    );

    Ok(row_to_tool(row))
}

pub async fn list_mcp_tools(
    store: &EndpointStore,
    tenant_id: &str,
) -> Result<Vec<McpTool>, StoreError> {
    let client = store.get_conn().await?;

    let rows = client
        .query(
            "SELECT id, tenant_id, tool_name, backend_url, description,
                    input_schema, cost_credits, timeout_ms, is_active,
                    created_at, updated_at
             FROM mcp_tools
             WHERE tenant_id = $1 AND is_active = true
             ORDER BY tool_name",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    Ok(rows.into_iter().map(row_to_tool).collect())
}

pub async fn get_mcp_tool(
    store: &EndpointStore,
    tenant_id: &str,
    tool_name: &str,
) -> Result<Option<McpTool>, StoreError> {
    let client = store.get_conn().await?;

    let row = client
        .query_opt(
            "SELECT id, tenant_id, tool_name, backend_url, description,
                    input_schema, cost_credits, timeout_ms, is_active,
                    created_at, updated_at
             FROM mcp_tools
             WHERE tenant_id = $1 AND tool_name = $2 AND is_active = true",
            &[&tenant_id, &tool_name],
        )
        .await
        .to_store_error()?;

    Ok(row.map(row_to_tool))
}

pub async fn delete_mcp_tool(
    store: &EndpointStore,
    tenant_id: &str,
    tool_name: &str,
) -> Result<bool, StoreError> {
    let client = store.get_conn().await?;

    let n = client
        .execute(
            "UPDATE mcp_tools SET is_active = false, updated_at = NOW()
             WHERE tenant_id = $1 AND tool_name = $2",
            &[&tenant_id, &tool_name],
        )
        .await
        .to_store_error()?;

    Ok(n > 0)
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn row_to_tool(row: tokio_postgres::Row) -> McpTool {
    McpTool {
        id:           row.get(0),
        tenant_id:    row.get(1),
        tool_name:    row.get(2),
        backend_url:  row.get(3),
        description:  row.get(4),
        input_schema: row.get(5),
        cost_credits: row.get(6),
        timeout_ms:   row.get(7),
        is_active:    row.get(8),
        created_at:   row.get::<_, chrono::DateTime<Utc>>(9).to_rfc3339(),
        updated_at:   row.get::<_, chrono::DateTime<Utc>>(10).to_rfc3339(),
    }
}
