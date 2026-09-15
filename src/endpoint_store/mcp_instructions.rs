// src/endpoint_store/mcp_instructions.rs
//
// What the gateway needs to write a tenant's MCP `instructions`: a summary of the
// tenant's tools, grouped the way the tenant grouped them, and the text the
// tenant wrote itself.
//
// The gateway turns this into prose. Nothing here is wording — only facts about
// which tools exist and which can be called without arguments, so a model can be
// pointed at the tools that answer "which projects / teams / sprints are there"
// before it guesses a name.

use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::tenant_management::get_default_tenant;
use crate::endpoint_store::{EndpointStore, StoreError};
use serde::Serialize;
use slug::slugify;

/// Longest text a tenant may store. It is sent to the model on every
/// connection, next to api0's own guidance, so it has to stay short.
pub const MAX_TENANT_INSTRUCTIONS_CHARS: usize = 2000;

#[derive(Debug, Clone, Serialize)]
pub struct ToolSummary {
    pub name: String,
    pub description: String,
    /// `None` for a native MCP backend, whose verb api0 does not know.
    pub http_verb: Option<String>,
    pub required_params: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolGroupSummary {
    pub name: String,
    pub description: String,
    pub tools: Vec<ToolSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpInstructionsContext {
    pub tenant_id: String,
    /// The tenant's own text. `None` when it has not written any.
    pub custom: Option<String>,
    pub groups: Vec<ToolGroupSummary>,
    /// Tools registered directly rather than imported with an API group.
    pub ungrouped_tools: Vec<ToolSummary>,
}

/// Everything the gateway needs for one tenant's `initialize`.
pub async fn get_mcp_instructions_context(
    store: &EndpointStore,
    tenant_id: &str,
) -> Result<McpInstructionsContext, StoreError> {
    let client = store.get_conn(Some(tenant_id)).await?;

    // One query for every endpoint and how many of its parameters are required,
    // rather than one parameters query per endpoint as `list_mcp_tools` does:
    // this runs on every connection.
    let endpoint_rows = client
        .query(
            "SELECT g.id, g.name, g.description, e.text, e.description, e.suggested_sentence,
                    e.verb,
                    (SELECT COUNT(*) FROM parameters p
                      WHERE p.endpoint_id = e.id AND p.required = true)
             FROM api_groups g
             JOIN endpoints e ON g.id = e.group_id
             WHERE g.tenant_id = $1
             ORDER BY g.name, e.text",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    let mut groups: Vec<(String, ToolGroupSummary)> = Vec::new();
    for row in endpoint_rows {
        let group_id: String = row.get(0);
        let group_name: String = row.get(1);
        let endpoint_text: String = row.get(3);

        // Same naming as `list_mcp_tools`, so the names here are the ones the
        // model sees in tools/list.
        let tool_name = slugify(format!("{} {}", group_name, endpoint_text));
        if tool_name.is_empty() {
            continue;
        }

        let endpoint_desc: String = row.get(4);
        let suggested: String = row.get(5);
        let description = [endpoint_desc, suggested, endpoint_text]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        let required: i64 = row.get(7);

        let tool = ToolSummary {
            name: tool_name,
            description,
            http_verb: Some(row.get::<_, String>(6).to_uppercase()),
            required_params: required.max(0) as usize,
        };

        match groups.iter_mut().find(|(id, _)| *id == group_id) {
            Some((_, group)) => group.tools.push(tool),
            None => groups.push((
                group_id,
                ToolGroupSummary {
                    name: group_name,
                    description: row.get(2),
                    tools: vec![tool],
                },
            )),
        }
    }
    let groups: Vec<ToolGroupSummary> = groups.into_iter().map(|(_, g)| g).collect();

    // Imported endpoints are also synced into mcp_tools under the same names, so
    // only a row with no group behind it is ungrouped.
    let explicit_rows = client
        .query(
            "SELECT tool_name, description, input_schema, http_verb
             FROM mcp_tools
             WHERE tenant_id = $1 AND is_active = true
             ORDER BY tool_name",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    let ungrouped_tools = explicit_rows
        .into_iter()
        .filter_map(|row| {
            let name: String = row.get(0);
            if groups.iter().any(|g| g.tools.iter().any(|t| t.name == name)) {
                return None;
            }
            let schema: String = row.get(2);
            Some(ToolSummary {
                name,
                description: row.get(1),
                http_verb: row.get::<_, Option<String>>(3).map(|v| v.to_uppercase()),
                required_params: required_count(&schema),
            })
        })
        .collect();

    let custom = get_tenant_instructions(store, tenant_id).await?;

    Ok(McpInstructionsContext {
        tenant_id: tenant_id.to_string(),
        custom,
        groups,
        ungrouped_tools,
    })
}

/// The tenant's own text, if it wrote any.
pub async fn get_tenant_instructions(
    store: &EndpointStore,
    tenant_id: &str,
) -> Result<Option<String>, StoreError> {
    let client = store.get_admin_conn().await?;
    let row = client
        .query_opt("SELECT mcp_instructions FROM tenants WHERE id = $1", &[&tenant_id])
        .await
        .to_store_error()?;
    Ok(row
        .and_then(|r| r.get::<_, Option<String>>(0))
        .filter(|s| !s.trim().is_empty()))
}

/// Replace the caller's tenant's text. Blank clears it. Returns the tenant id
/// and what is now stored.
pub async fn set_tenant_instructions(
    store: &EndpointStore,
    email: &str,
    text: Option<&str>,
) -> Result<(String, Option<String>), StoreError> {
    let text = text.map(str::trim).filter(|s| !s.is_empty());
    if let Some(t) = text {
        let chars = t.chars().count();
        if chars > MAX_TENANT_INSTRUCTIONS_CHARS {
            return Err(StoreError::InvalidInput(format!(
                "Instructions are {} characters; the limit is {}",
                chars, MAX_TENANT_INSTRUCTIONS_CHARS
            )));
        }
    }

    let tenant = get_default_tenant(store, email).await?;
    let client = store.get_admin_conn().await?;
    client
        .execute(
            "UPDATE tenants SET mcp_instructions = $1 WHERE id = $2",
            &[&text, &tenant.id],
        )
        .await
        .to_store_error()?;

    Ok((tenant.id, text.map(str::to_string)))
}

/// How many properties a stored JSON Schema marks as required. A schema that
/// does not parse counts as zero, the same as the gateway treats it.
fn required_count(schema: &str) -> usize {
    serde_json::from_str::<serde_json::Value>(schema)
        .ok()
        .and_then(|v| v.get("required").and_then(|r| r.as_array()).map(Vec::len))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::required_count;

    #[test]
    fn counts_required_properties() {
        assert_eq!(
            required_count(r#"{"type":"object","properties":{"a":{},"b":{}},"required":["a"]}"#),
            1
        );
    }

    #[test]
    fn a_schema_without_required_or_unparseable_counts_zero() {
        assert_eq!(required_count(r#"{"type":"object","properties":{}}"#), 0);
        assert_eq!(required_count("not json"), 0);
    }
}
