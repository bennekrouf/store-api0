// src/endpoint_store/mcp_tools_management.rs
//
// CRUD for the mcp_tools table.
// Called by mcp_tools_handler (HTTP) and the gateway via /api/mcp-tools/* endpoints.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{ApiGroupWithEndpoints, EndpointStore, StoreError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use slug::slugify;
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
    /// When Some("GET"|"POST"|…) the gateway does REST passthrough.
    /// When None the backend is expected to speak native MCP format.
    pub http_verb: Option<String>,
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
    /// REST verb for endpoint-imported tools. None = native MCP backend.
    pub http_verb: Option<String>,
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
    let http_verb = req.http_verb.as_deref().map(|v| v.to_uppercase());

    let row = client
        .query_one(
            "INSERT INTO mcp_tools
                (id, tenant_id, tool_name, backend_url, description,
                 input_schema, cost_credits, timeout_ms, http_verb, is_active,
                 created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, true, $10, $10)
             ON CONFLICT (tenant_id, tool_name) DO UPDATE SET
                backend_url  = EXCLUDED.backend_url,
                description  = EXCLUDED.description,
                input_schema = EXCLUDED.input_schema,
                cost_credits = EXCLUDED.cost_credits,
                timeout_ms   = EXCLUDED.timeout_ms,
                http_verb    = EXCLUDED.http_verb,
                is_active    = true,
                updated_at   = EXCLUDED.updated_at
             RETURNING id, tenant_id, tool_name, backend_url, description,
                       input_schema, cost_credits, timeout_ms, http_verb,
                       is_active, created_at, updated_at",
            &[
                &id, &tenant_id, &req.tool_name, &req.backend_url,
                &description, &input_schema, &cost_credits, &timeout_ms,
                &http_verb, &now,
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

    // 1. Fetch explicit tools from mcp_tools table
    let explicit_rows = client
        .query(
            "SELECT id, tenant_id, tool_name, backend_url, description,
                    input_schema, cost_credits, timeout_ms, http_verb,
                    is_active, created_at, updated_at
             FROM mcp_tools
             WHERE tenant_id = $1 AND is_active = true
             ORDER BY tool_name",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    let mut all_tools: Vec<McpTool> = explicit_rows.into_iter().map(row_to_tool).collect();

    // 2. Fetch all endpoints for this tenant to expose them as virtual tools
    // We join api_groups (which now has tenant_id) to endpoints.
    let endpoint_rows = client
        .query(
            "SELECT g.name, e.text, e.description, e.suggested_sentence, e.verb, e.base, g.base, e.path, e.id
             FROM api_groups g
             JOIN endpoints e ON g.id = e.group_id
             WHERE g.tenant_id = $1",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    app_log!(debug, tenant_id = %tenant_id, endpoint_count = endpoint_rows.len(), "Fetched endpoints for MCP tool mapping");

    for row in endpoint_rows {
        let group_name: String = row.get(0);
        let endpoint_text: String = row.get(1);
        let endpoint_desc: String = row.get(2);
        let suggested: String = row.get(3);
        let verb: String = row.get(4);
        let e_base: String = row.get(5);
        let g_base: String = row.get(6);
        let path: String = row.get(7);
        let endpoint_id: String = row.get(8);
        
        let raw_name = format!("{} {}", group_name, endpoint_text);
        let tool_name = slugify(&raw_name);
        if tool_name.is_empty() { continue; }

        // Skip if a tool with this name already exists (explicit tools take precedence)
        if all_tools.iter().any(|t| t.tool_name == tool_name) {
            continue;
        }

        let base = if e_base.is_empty() { &g_base } else { &e_base };
        let backend_url = format!("{}{}", base.trim_end_matches('/'), path);
        
        let description = [endpoint_desc.as_str(), suggested.as_str(), endpoint_text.as_str()]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("")
            .to_string();

        // Parameters: we need another query if we want full schemas.
        // For 'list_mcp_tools', we'll fetch them. (In a real high-load scenario we'd join, but this is cleaner).
        let param_rows = client.query(
            "SELECT name, description, required FROM parameters WHERE endpoint_id = $1",
            &[&endpoint_id]
        ).await.to_store_error()?;
        
        let mut params = Vec::new();
        for pr in param_rows {
            params.push(crate::endpoint_store::models::Parameter {
                name: pr.get(0),
                description: pr.get(1),
                required: pr.get::<_, bool>(2).to_string(),
                alternatives: vec![], // skipped for speed in list
            });
        }
        
        let input_schema = build_input_schema(&params);

        all_tools.push(McpTool {
            id: format!("virtual-{}", endpoint_id),
            tenant_id: tenant_id.to_string(),
            tool_name,
            backend_url,
            description,
            input_schema,
            cost_credits: 1,
            timeout_ms: 30000,
            http_verb: Some(verb.to_uppercase()),
            is_active: true,
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
        });
    }

    Ok(all_tools)
}

pub async fn get_mcp_tool(
    store: &EndpointStore,
    tenant_id: &str,
    tool_name: &str,
) -> Result<Option<McpTool>, StoreError> {
    let client = store.get_conn().await?;

    // 1. Check explicit tools
    let row = client
        .query_opt(
            "SELECT id, tenant_id, tool_name, backend_url, description,
                    input_schema, cost_credits, timeout_ms, http_verb,
                    is_active, created_at, updated_at
             FROM mcp_tools
             WHERE tenant_id = $1 AND tool_name = $2 AND is_active = true",
            &[&tenant_id, &tool_name],
        )
        .await
        .to_store_error()?;

    if let Some(r) = row {
        return Ok(Some(row_to_tool(r)));
    }

    // 2. Check virtual tools (endpoints)
    // We need to find an endpoint for this tenant whose slugified (Group + Name) matches tool_name.
    // Optimization: we could store the slug in the DB, but for now we'll fetch all groups for the tenant
    // and find the matching one. Since a tenant usually has < 100 endpoints, this is acceptable.
    let endpoint_rows = client
        .query(
            "SELECT g.name, e.text, e.description, e.suggested_sentence, e.verb, e.base, g.base, e.path, e.id
             FROM api_groups g
             JOIN endpoints e ON g.id = e.group_id
             WHERE g.tenant_id = $1",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    for row in endpoint_rows {
        let group_name: String = row.get(0);
        let endpoint_text: String = row.get(1);
        
        let raw_name = format!("{} {}", group_name, endpoint_text);
        if slugify(&raw_name) == tool_name {
            let endpoint_desc: String = row.get(2);
            let suggested: String = row.get(3);
            let verb: String = row.get(4);
            let e_base: String = row.get(5);
            let g_base: String = row.get(6);
            let path: String = row.get(7);
            let endpoint_id: String = row.get(8);

            let base = if e_base.is_empty() { &g_base } else { &e_base };
            let backend_url = format!("{}{}", base.trim_end_matches('/'), path);
            
            let description = [endpoint_desc.as_str(), suggested.as_str(), endpoint_text.as_str()]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or("")
                .to_string();

            let param_rows = client.query(
                "SELECT name, description, required FROM parameters WHERE endpoint_id = $1",
                &[&endpoint_id]
            ).await.to_store_error()?;
            
            let mut params = Vec::new();
            for pr in param_rows {
                params.push(crate::endpoint_store::models::Parameter {
                    name: pr.get(0),
                    description: pr.get(1),
                    required: pr.get::<_, bool>(2).to_string(),
                    alternatives: vec![],
                });
            }
            
            let input_schema = build_input_schema(&params);

            return Ok(Some(McpTool {
                id: format!("virtual-{}", endpoint_id),
                tenant_id: tenant_id.to_string(),
                tool_name: tool_name.to_string(),
                backend_url,
                description,
                input_schema,
                cost_credits: 1,
                timeout_ms: 30000,
                http_verb: Some(verb.to_uppercase()),
                is_active: true,
                created_at: Utc::now().to_rfc3339(),
                updated_at: Utc::now().to_rfc3339(),
            }));
        }
    }

    Ok(None)
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

// ── Sync: imported endpoints → mcp_tools ─────────────────────────────────────

/// Called after every successful endpoint import.
/// Each endpoint in every group is upserted into `mcp_tools` so it appears
/// automatically in `tools/list` without any extra configuration.
///
/// Mapping:
///   tool_name    = slug("{group_name} {endpoint_text}")
///   backend_url  = endpoint.base + endpoint.path
///   description  = endpoint.description || suggested_sentence || text
///   input_schema = JSON Schema built from endpoint.parameters
///   http_verb    = endpoint.verb  (GET/POST/… — gateway will do REST passthrough)
///   cost_credits = 1 (default, can be changed via the management API later)
pub async fn sync_endpoints_as_mcp_tools(
    store: &EndpointStore,
    tenant_id: &str,
    groups: &[ApiGroupWithEndpoints],
) -> Result<usize, StoreError> {
    let mut count = 0usize;

    for group in groups {
        for endpoint in &group.endpoints {
            // ── tool_name ─────────────────────────────────────────────────────
            let raw_name = format!("{} {}", group.group.name, endpoint.text);
            let tool_name = slugify(&raw_name);
            if tool_name.is_empty() {
                continue;
            }

            // ── backend_url ───────────────────────────────────────────────────
            let base = if endpoint.base.is_empty() {
                &group.group.base
            } else {
                &endpoint.base
            };
            let backend_url = format!("{}{}", base.trim_end_matches('/'), endpoint.path);
            if backend_url.trim_matches('/').is_empty() {
                continue;
            }

            // ── description ───────────────────────────────────────────────────
            let description = [
                endpoint.description.as_str(),
                endpoint.suggested_sentence.as_str(),
                endpoint.text.as_str(),
            ]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or("")
            .to_string();

            // ── input_schema (JSON Schema from parameters) ────────────────────
            let input_schema = build_input_schema(&endpoint.parameters);

            let req = UpsertMcpToolRequest {
                tool_name,
                backend_url,
                description: Some(description),
                input_schema: Some(input_schema),
                cost_credits: Some(1),
                timeout_ms: Some(30_000),
                http_verb: Some(endpoint.verb.to_uppercase()),
            };

            match upsert_mcp_tool(store, tenant_id, &req).await {
                Ok(_) => count += 1,
                Err(e) => {
                    // Non-fatal: log and continue with the remaining endpoints
                    app_log!(
                        warn,
                        tenant_id = %tenant_id,
                        tool_name = %req.tool_name,
                        error = %e,
                        "Failed to sync endpoint as MCP tool (skipping)"
                    );
                }
            }
        }
    }

    app_log!(
        info,
        tenant_id = %tenant_id,
        synced = count,
        "Synced imported endpoints to mcp_tools"
    );
    Ok(count)
}

/// Build a minimal JSON Schema from a list of endpoint parameters.
fn build_input_schema(params: &[crate::endpoint_store::models::Parameter]) -> String {
    if params.is_empty() {
        return r#"{"type":"object","properties":{}}"#.to_string();
    }

    let mut properties = serde_json::Map::new();
    let mut required: Vec<serde_json::Value> = Vec::new();

    for p in params {
        let mut prop = serde_json::Map::new();
        prop.insert("type".into(), serde_json::Value::String("string".into()));
        if !p.description.is_empty() {
            prop.insert(
                "description".into(),
                serde_json::Value::String(p.description.clone()),
            );
        }
        // Also list any known alternatives as an enum
        if !p.alternatives.is_empty() {
            prop.insert(
                "enum".into(),
                serde_json::Value::Array(
                    p.alternatives
                        .iter()
                        .map(|a| serde_json::Value::String(a.clone()))
                        .collect(),
                ),
            );
        }
        properties.insert(p.name.clone(), serde_json::Value::Object(prop));

        if p.required == "true" {
            required.push(serde_json::Value::String(p.name.clone()));
        }
    }

    let schema = if required.is_empty() {
        serde_json::json!({
            "type": "object",
            "properties": properties
        })
    } else {
        serde_json::json!({
            "type": "object",
            "properties": properties,
            "required": required
        })
    };

    serde_json::to_string(&schema).unwrap_or_else(|_| r#"{"type":"object","properties":{}}"#.to_string())
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
        http_verb:    row.get(8),
        is_active:    row.get(9),
        created_at:   row.get::<_, chrono::DateTime<Utc>>(10).to_rfc3339(),
        updated_at:   row.get::<_, chrono::DateTime<Utc>>(11).to_rfc3339(),
    }
}
