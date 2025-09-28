use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{
    generate_id_from_text, ApiGroupWithEndpoints, EndpointStore, StoreError,
};
use rusqlite::ToSql;
/// Replaces all API groups and endpoints for a user
pub async fn replace_user_api_groups(
    store: &EndpointStore,
    email: &str,
    api_groups: Vec<ApiGroupWithEndpoints>,
) -> Result<usize, StoreError> {
    tracing::info!(email = %email, "Starting complete API group replacement");

    // Clean up existing user data
    match store.force_clean_user_data(email).await {
        Ok(_) => {
            tracing::info!(email = %email, "Successfully cleaned up user data");
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to clean up user data, will try fallback approach"
            );

            // Fallback approach
            match store.fallback_clean_user_data(email).await {
                Ok(_) => tracing::info!(email = %email, "Fallback cleanup successful"),
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        email = %email,
                        "Fallback cleanup also failed, proceeding with import anyway"
                    );
                }
            }
        }
    }

    // Add new groups and endpoints
    let mut imported_count = 0;
    let mut conn = store.get_conn().await?;
    let tx = conn.transaction().to_store_error()?;

    for group_with_endpoints in &api_groups {
        let group = &group_with_endpoints.group;

        // Generate ID if not provided
        let group_id = if group.id.is_empty() {
            generate_id_from_text(&group.name)
        } else {
            group.id.clone()
        };

        // Check if group exists
        let group_exists: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM api_groups WHERE id = ?)",
                [&group_id],
                |row| row.get(0),
            )
            .to_store_error()?;

        if !group_exists {
            // Create new group
            tracing::debug!(group_id = %group_id, "Creating new API group");
            tx.execute(
                "INSERT INTO api_groups (id, name, description, base) VALUES (?, ?, ?, ?)",
                &[&group_id, &group.name, &group.description, &group.base],
            )
            .to_store_error()?;
        } else {
            // Update existing non-default group
            tracing::debug!(group_id = %group_id, "Updating existing API group");
            tx.execute(
                "UPDATE api_groups SET name = ?, description = ?, base = ? WHERE id = ?",
                &[&group.name, &group.description, &group.base, &group_id],
            )
            .to_store_error()?;
        }

        // Link group to user
        tx.execute(
            "INSERT OR IGNORE INTO user_groups (email, group_id) VALUES (?, ?)",
            &[email, &group_id],
        )
        .to_store_error()?;

        // Process endpoints for this group
        for endpoint in &group_with_endpoints.endpoints {
            // Generate ID if not provided
            let endpoint_id = if endpoint.id.is_empty() {
                generate_id_from_text(&endpoint.text)
            } else {
                endpoint.id.clone()
            };

            // Check if endpoint exists
            let endpoint_exists: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM endpoints WHERE id = ?)",
                    [&endpoint_id],
                    |row| row.get(0),
                )
                .to_store_error()?;

            if !endpoint_exists {
                // Create new endpoint
                tracing::debug!(endpoint_id = %endpoint_id, "Creating new endpoint");

                let params: &[&dyn ToSql] = &[
                    &endpoint_id as &dyn ToSql,
                    &endpoint.text,
                    &endpoint.description,
                    &endpoint.verb,
                    &endpoint.base,
                    &endpoint.path,
                    &group_id,
                ];

                tx.execute(
                    "INSERT INTO endpoints (id, text, description, verb, base, path, group_id) 
                     VALUES (?, ?, ?, ?, ?, ?, ?)",
                    params,
                )
                .to_store_error()?;
            } else {
                // Check if it's a default endpoint
                tracing::debug!(endpoint_id = %endpoint_id, "Updating existing endpoint");
                tx.execute(
                    "UPDATE endpoints SET text = ?, description = ?, verb = ?, base = ?, path = ?, group_id = ? WHERE id = ?",
                    &[
                        &endpoint.text,
                        &endpoint.description,
                        &endpoint.verb,
                        &endpoint.base,
                        &endpoint.path,
                        &group_id,
                        &endpoint_id,
                    ],
                ).to_store_error()?;
            }

            // Link endpoint to user
            tx.execute(
                "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                &[email, &endpoint_id],
            )
            .to_store_error()?;

            // Clean up existing parameters first
            tx.execute(
                "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
                [&endpoint_id],
            )
            .to_store_error()?;

            tx.execute(
                "DELETE FROM parameters WHERE endpoint_id = ?",
                [&endpoint_id],
            )
            .to_store_error()?;

            // Add new parameters
            for param in &endpoint.parameters {
                tx.execute(
                    "INSERT INTO parameters (endpoint_id, name, description, required) 
                        VALUES (?, ?, ?, ?)",
                    &[
                        &endpoint_id,
                        &param.name,
                        &param.description,
                        &param.required,
                    ],
                )
                .to_store_error()?;

                // Add parameter alternatives
                for alt in &param.alternatives {
                    tx.execute(
                        "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) 
                            VALUES (?, ?, ?)",
                        &[&endpoint_id, &param.name, alt],
                    ).to_store_error()?;
                }
            }

            imported_count += 1;
        }
    }

    tracing::info!(
        email = %email,
        group_count = api_groups.len(),
        endpoint_count = imported_count,
        "Successfully imported API groups and endpoints"
    );

    tx.commit().to_store_error()?;
    Ok(imported_count)
}
