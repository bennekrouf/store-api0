use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{
    generate_id_from_text, ApiGroupWithEndpoints, EndpointStore, StoreError,
};
/// Imports API groups into a workspace, replacing its groups of the same name.
///
/// `tenant_id` names the tenant the groups belong to. `None` means the caller's
/// default tenant, which is what every caller did before this was a choice. It
/// decides MCP visibility: `list_mcp_tools` matches on `api_groups.tenant_id`,
/// so importing into the wrong tenant leaves the tools invisible to the
/// connector the user actually opens.
pub async fn replace_user_api_groups(
    store: &EndpointStore,
    email: &str,
    api_groups: Vec<ApiGroupWithEndpoints>,
    tenant_id: Option<&str>,
) -> Result<usize, StoreError> {
    app_log!(info, email = %email, "Starting complete API group replacement");

    // Settle the target tenant before touching anything: a refused import must
    // not have already deleted the caller's existing endpoints.
    let tenant_id = match tenant_id {
        // A named tenant must be one the caller actually belongs to, or an
        // import becomes a way to write into someone else's namespace.
        Some(requested) => {
            let client = store.get_admin_conn().await?;
            let allowed = crate::endpoint_store::tenant_management::verify_tenant_access_with_conn(
                &client, email, requested,
            )
            .await?;
            if !allowed {
                app_log!(warn, email = %email, tenant_id = %requested, "Rejected import into a tenant the caller does not belong to");
                return Err(StoreError::InvalidInput(format!(
                    "You do not belong to tenant '{}'",
                    requested
                )));
            }
            requested.to_string()
        }
        None => {
            crate::endpoint_store::tenant_management::get_default_tenant(store, email)
                .await?
                .id
        }
    };

    // Add new groups and endpoints

    let mut imported_count = 0;
    let mut client = store.get_admin_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    // An upload replaces the groups of *this workspace* that it brings again,
    // matched by name (ids get a fresh suffix on every upload), in the same
    // transaction as the insert. Groups the file does not mention stay: a
    // workspace mixes APIs from several files and from the setup wizard, and
    // importing one must not wipe the others. A group is removed with its
    // delete button.
    let names: Vec<String> = api_groups
        .iter()
        .map(|g| g.group.name.trim().to_lowercase())
        .collect();
    let old_groups: Vec<String> = tx
        .query(
            "SELECT id FROM api_groups WHERE tenant_id = $1 AND lower(trim(name)) = ANY($2)",
            &[&tenant_id, &names],
        )
        .await
        .to_store_error()?
        .iter()
        .map(|r| r.get(0))
        .collect();
    let old_endpoints: Vec<String> = tx
        .query("SELECT id FROM endpoints WHERE group_id = ANY($1)", &[&old_groups])
        .await
        .to_store_error()?
        .iter()
        .map(|r| r.get(0))
        .collect();
    for sql in [
        "DELETE FROM parameter_alternatives WHERE endpoint_id = ANY($1)",
        "DELETE FROM parameters WHERE endpoint_id = ANY($1)",
        "DELETE FROM user_endpoints WHERE endpoint_id = ANY($1)",
        "DELETE FROM endpoints WHERE id = ANY($1)",
    ] {
        tx.execute(sql, &[&old_endpoints]).await.to_store_error()?;
    }
    for sql in [
        "DELETE FROM user_groups WHERE group_id = ANY($1)",
        "DELETE FROM api_groups WHERE id = ANY($1)",
    ] {
        tx.execute(sql, &[&old_groups]).await.to_store_error()?;
    }
    app_log!(info, tenant_id = %tenant_id, groups = old_groups.len(), endpoints = old_endpoints.len(),
        "Replaced the workspace's previous API groups");

    for group_with_endpoints in &api_groups {
        let group = &group_with_endpoints.group;

        // Generate ID if not provided
        let group_id = if group.id.is_empty() {
            generate_id_from_text(&group.name)
        } else {
            group.id.clone()
        };

        // Check if group exists
        let group_exists_row = tx
            .query_opt("SELECT 1 FROM api_groups WHERE id = $1", &[&group_id])
            .await
            .to_store_error()?;

        if group_exists_row.is_some() {
            return Err(StoreError::InvalidInput(format!(
                "API group id '{}' is used by another workspace; remove the explicit id or choose another",
                group_id
            )));
        }
        if group_exists_row.is_none() {
            app_log!(debug, group_id = %group_id, tenant_id = %tenant_id, "Creating new API group");
            tx.execute(
                "INSERT INTO api_groups (id, name, description, base, tenant_id) VALUES ($1, $2, $3, $4, $5)",
                &[&group_id, &group.name, &group.description, &group.base, &tenant_id],
            )
            .await
            .to_store_error()?;
        } else {
            app_log!(debug, group_id = %group_id, tenant_id = %tenant_id, "Updating existing API group");
            tx.execute(
                "UPDATE api_groups SET name = $1, description = $2, base = $3, tenant_id = $4 WHERE id = $5",
                &[&group.name, &group.description, &group.base, &tenant_id, &group_id],
            )
            .await
            .to_store_error()?;
        }

        // Link group to user
        tx.execute(
            "INSERT INTO user_groups (email, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            &[&email, &group_id],
        )
        .await
        .to_store_error()?;

        // Process endpoints for this group
        for endpoint in &group_with_endpoints.endpoints {
            let response_fields = crate::endpoint_store::mcp_tools_management::clean_response_fields(
                endpoint.response_fields.as_deref(),
            );
            let endpoint_id = if endpoint.id.is_empty() {
                generate_id_from_text(&endpoint.text)
            } else {
                endpoint.id.clone()
            };

            let endpoint_exists_row = tx
                .query_opt("SELECT 1 FROM endpoints WHERE id = $1", &[&endpoint_id])
                .await
                .to_store_error()?;

            if endpoint_exists_row.is_some() {
                return Err(StoreError::InvalidInput(format!(
                    "Endpoint id '{}' is used by another workspace; remove the explicit id or choose another",
                    endpoint_id
                )));
            }
            if endpoint_exists_row.is_none() {
                app_log!(debug, endpoint_id = %endpoint_id, "Creating new endpoint");
                tx.execute(
                    "INSERT INTO endpoints (id, text, description, verb, base, path, group_id, suggested_sentence, content_type, body_template, forward_identity, response_fields) 
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
                    &[
                        &endpoint_id,
                        &endpoint.text,
                        &endpoint.description,
                        &endpoint.verb,
                        &endpoint.base,
                        &endpoint.path,
                        &group_id,
                        &endpoint.suggested_sentence,
                        &endpoint.content_type,
                        &endpoint.body_template,
                        &endpoint.forward_identity,
                        &response_fields,
                    ],
                )
                .await
                .to_store_error()?;
            } else {
                app_log!(debug, endpoint_id = %endpoint_id, "Updating existing endpoint");
                tx.execute(
                    "UPDATE endpoints SET text = $1, description = $2, verb = $3, base = $4, path = $5, group_id = $6, suggested_sentence = $7, response_fields = $9 WHERE id = $8",
                    &[
                        &endpoint.text,
                        &endpoint.description,
                        &endpoint.verb,
                        &endpoint.base,
                        &endpoint.path,
                        &group_id,
                        &endpoint.suggested_sentence,
                        &endpoint_id,
                        &response_fields,
                    ],
                )
                .await
                .to_store_error()?;
            }

            // Link endpoint to user
            tx.execute(
                "INSERT INTO user_endpoints (email, endpoint_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                &[&email, &endpoint_id],
            )
            .await
            .to_store_error()?;

            // Clean up existing parameters
            tx.execute(
                "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
                &[&endpoint_id],
            )
            .await
            .to_store_error()?;

            tx.execute(
                "DELETE FROM parameters WHERE endpoint_id = $1",
                &[&endpoint_id],
            )
            .await
            .to_store_error()?;

            // Add new parameters
            for param in &endpoint.parameters {
                let required = param.required.parse::<bool>().unwrap_or(false);

                tx.execute(
                    "INSERT INTO parameters (endpoint_id, name, description, required) 
                        VALUES ($1, $2, $3, $4)",
                    &[&endpoint_id, &param.name, &param.description, &required],
                )
                .await
                .to_store_error()?;

                for alt in &param.alternatives {
                    tx.execute(
                        "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) 
                            VALUES ($1, $2, $3)",
                        &[&endpoint_id, &param.name, alt],
                    )
                    .await
                    .to_store_error()?;
                }
            }

            imported_count += 1;
        }
    }

    app_log!(info,
        email = %email,
        group_count = api_groups.len(),
        endpoint_count = imported_count,
        "Successfully imported API groups and endpoints"
    );

    tx.commit().await.to_store_error()?;
    Ok(imported_count)
}

/// Against a real database. Run with a throwaway Postgres:
///   TEST_DATABASE_URL=postgres://app:app@localhost:55432/app \
///     cargo test --bin store -- --ignored an_import_keeps_the_groups_it_does_not_mention
#[cfg(test)]
mod db_tests {
    use crate::endpoint_store::mcp_tools_management::resync_tenant_tools;
    use crate::endpoint_store::tenant_management::get_default_tenant;
    use crate::endpoint_store::{ApiStorage, EndpointStore};

    /// An import document in the shape the setup wizard sends (JSON, required
    /// as a string) — parsed the way the upload route parses it.
    fn doc(groups: &[(&str, &str)]) -> ApiStorage {
        let groups: Vec<_> = groups
            .iter()
            .map(|(name, text)| {
                serde_json::json!({
                    "name": name,
                    "base": "https://dev.azure.com/acme",
                    "endpoints": [{
                        "text": text,
                        "verb": "GET",
                        "path": format!("/{{project}}/{}", text.replace(' ', "-")),
                        "forward_identity": false,
                        "parameters": [{ "name": "project", "description": "Project.", "required": "true" }]
                    }]
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({ "api_groups": groups })).expect("import document")
    }

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL"]
    async fn an_import_keeps_the_groups_it_does_not_mention() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = EndpointStore::new(&url).await.expect("store");
        let email = format!("import-{}@example.com", uuid::Uuid::new_v4().simple());
        let tenant = get_default_tenant(&store, &email).await.unwrap().id;

        // A file with two APIs, then the wizard importing its own.
        store
            .replace_user_api_groups(&email, doc(&[("Billing", "list invoices"), ("Azure DevOps", "list projects")]).api_groups, Some(&tenant))
            .await
            .unwrap();
        resync_tenant_tools(&store, &tenant).await;

        // Reconfigure imports "Azure DevOps" again, with different operations
        // (and the name spelt differently): it replaces that group only.
        store
            .replace_user_api_groups(&email, doc(&[(" azure devops", "list teams")]).api_groups, Some(&tenant))
            .await
            .unwrap();
        let synced = resync_tenant_tools(&store, &tenant).await;
        assert_eq!(synced, 2);

        let c = store.get_admin_conn().await.unwrap();
        let mut groups: Vec<String> = c
            .query("SELECT name FROM api_groups WHERE tenant_id = $1", &[&tenant])
            .await.unwrap().iter().map(|r| r.get(0)).collect();
        groups.sort();
        assert_eq!(groups, vec![" azure devops".to_string(), "Billing".to_string()]);

        let mut endpoints: Vec<String> = c
            .query(
                "SELECT e.text FROM endpoints e JOIN api_groups g ON g.id = e.group_id WHERE g.tenant_id = $1",
                &[&tenant],
            )
            .await.unwrap().iter().map(|r| r.get(0)).collect();
        endpoints.sort();
        assert_eq!(endpoints, vec!["list invoices".to_string(), "list teams".to_string()]);

        let mut active: Vec<String> = c
            .query("SELECT tool_name FROM mcp_tools WHERE tenant_id = $1 AND is_active", &[&tenant])
            .await.unwrap().iter().map(|r| r.get(0)).collect();
        active.sort();
        assert_eq!(active, vec!["azure-devops-list-teams".to_string(), "billing-list-invoices".to_string()]);

        let gone: bool = c
            .query_one(
                "SELECT is_active FROM mcp_tools WHERE tenant_id = $1 AND tool_name = 'azure-devops-list-projects'",
                &[&tenant],
            )
            .await.unwrap().get(0);
        assert!(!gone, "the replaced operation's tool is switched off");

        let identity: bool = c
            .query_one(
                "SELECT forward_identity FROM mcp_tools WHERE tenant_id = $1 AND tool_name = 'azure-devops-list-teams'",
                &[&tenant],
            )
            .await.unwrap().get(0);
        assert!(!identity, "no api0 identity headers to a third party");
    }

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL"]
    async fn response_fields_reach_the_tool_and_come_back_on_export() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = EndpointStore::new(&url).await.expect("store");
        let email = format!("fields-{}@example.com", uuid::Uuid::new_v4().simple());
        let tenant = get_default_tenant(&store, &email).await.unwrap().id;

        let with_fields: ApiStorage = serde_json::from_value(serde_json::json!({ "api_groups": [{
            "name": "Azure DevOps",
            "base": "https://dev.azure.com/acme",
            "endpoints": [
                { "text": "list work item types", "verb": "GET", "path": "/{project}/_apis/wit/workitemtypes",
                  "response_fields": ["count", " value", "name", "name", ""],
                  "parameters": [{ "name": "project", "description": "Project.", "required": "true" }] },
                { "text": "list projects", "verb": "GET", "path": "/_apis/projects", "parameters": [] }
            ]
        }]}))
        .expect("import document");
        store.replace_user_api_groups(&email, with_fields.api_groups, Some(&tenant)).await.unwrap();
        resync_tenant_tools(&store, &tenant).await;

        let tool = crate::endpoint_store::mcp_tools_management::get_mcp_tool(
            &store, &tenant, "azure-devops-list-work-item-types", None,
        )
        .await
        .unwrap()
        .expect("tool");
        assert_eq!(
            tool.response_fields,
            Some(vec!["count".to_string(), "value".to_string(), "name".to_string()]),
            "trimmed, without blanks or repeats"
        );
        let plain = crate::endpoint_store::mcp_tools_management::get_mcp_tool(&store, &tenant, "azure-devops-list-projects", None)
            .await
            .unwrap()
            .expect("tool");
        assert_eq!(plain.response_fields, None, "no list: the whole answer");
        // What the gateway reads is the tool's JSON.
        assert_eq!(serde_json::to_value(&tool).unwrap()["response_fields"][2], "name");

        // Exported and imported again (what an edit in the dashboard does), the list survives.
        let exported = crate::endpoint_store::get_api_groups_by_email::get_api_groups_by_tenant(&store, &tenant)
            .await
            .unwrap();
        let types = exported
            .iter()
            .flat_map(|g| g.endpoints.iter())
            .find(|e| e.text == "list work item types")
            .expect("exported");
        assert_eq!(types.response_fields.as_deref().map(|f| f.len()), Some(3));
    }
}
