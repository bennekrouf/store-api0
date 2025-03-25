use crate::endpoint_store::{EndpointStore, StoreError, ApiGroupWithEndpoints};
use crate::endpoint_store::db_helpers::ResultExt;

/// Adds a single API group for a user
pub async fn add_user_api_group(
    store: &EndpointStore,
    email: &str,
    api_group: &ApiGroupWithEndpoints,
) -> Result<usize, StoreError> {
    let mut conn = store.get_conn()?;
    let tx = conn.transaction().to_store_error()?;
    // store.with_transaction(|tx| {
        let group = &api_group.group;
        let group_id = &group.id;

        tracing::info!(
            email = %email,
            group_id = %group_id,
            "Adding API group"
        );

        // Check if group exists
        let group_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM api_groups WHERE id = ?)",
            [group_id],
            |row| row.get(0),
        ).to_store_error()?;

        if !group_exists {
            // Insert new group
            tx.execute(
                "INSERT INTO api_groups (id, name, description, base, is_default) VALUES (?, ?, ?, ?, false)",
                &[
                    group_id,
                    &group.name,
                    &group.description,
                    &group.base,
                ],
            ).to_store_error()?;
        } else {
            // Check if it's a default group
            let is_default: bool = tx.query_row(
                "SELECT is_default FROM api_groups WHERE id = ?",
                [group_id],
                |row| row.get(0),
            ).to_store_error()?;

            if !is_default {
                // Update existing non-default group
                tx.execute(
                    "UPDATE api_groups SET name = ?, description = ?, base = ? WHERE id = ?",
                    &[&group.name, &group.description, &group.base, group_id],
                ).to_store_error()?;
            }
        }

        // Associate group with user
        tx.execute(
            "INSERT OR IGNORE INTO user_groups (email, group_id) VALUES (?, ?)",
            &[email, group_id],
        ).to_store_error()?;

        // Add endpoints
        let mut endpoint_count = 0;

        for endpoint in &api_group.endpoints {
            // Check if endpoint exists
            let endpoint_exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM endpoints WHERE id = ?)",
                [&endpoint.id],
                |row| row.get(0),
            ).to_store_error()?;

            if !endpoint_exists {
                // Insert new endpoint
                tx.execute(
                    "INSERT INTO endpoints (id, text, description, verb, base, path, group_id, is_default) 
                    VALUES (?, ?, ?, ?, ?, ?, ?, false)",
                    &[
                        &endpoint.id,
                        &endpoint.text,
                        &endpoint.description,
                        &endpoint.verb,
                        &endpoint.base,
                        &endpoint.path,
                        group_id,
                    ],
                );
            } else {
                // Check if it's a default endpoint
                let is_default: bool = tx.query_row(
                    "SELECT is_default FROM endpoints WHERE id = ?",
                    [&endpoint.id],
                    |row| row.get(0),
                ).to_store_error()?;

                if !is_default {
                    // Update existing non-default endpoint
                    tx.execute(
                        "UPDATE endpoints SET text = ?, description = ?, verb = ?, base = ?, path = ?, group_id = ? WHERE id = ?",
                        &[
                            &endpoint.text,
                            &endpoint.description,
                            &endpoint.verb,
                            &endpoint.base,
                            &endpoint.path,
                            group_id,
                            &endpoint.id,
                        ],
                    ).to_store_error()?;
                }
            }

            // Associate endpoint with user
            tx.execute(
                "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                &[email, &endpoint.id],
            ).to_store_error()?;

            // Handle parameters for non-default endpoints
            let is_default: bool = tx.query_row(
                "SELECT is_default FROM endpoints WHERE id = ?",
                [&endpoint.id],
                |row| row.get(0),
            ).to_store_error()?;

            if !is_default {
                // Clean up existing parameters
                tx.execute(
                    "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
                    [&endpoint.id],
                ).to_store_error()?;

                tx.execute(
                    "DELETE FROM parameters WHERE endpoint_id = ?",
                    [&endpoint.id],
                ).to_store_error()?;

                // Add parameters
                for param in &endpoint.parameters {
                    tx.execute(
                        "INSERT INTO parameters (endpoint_id, name, description, required) 
                        VALUES (?, ?, ?, ?)",
                        &[
                            &endpoint.id,
                            &param.name,
                            &param.description,
                            &param.required.to_string(),
                        ],
                    ).to_store_error()?;

                    // Add parameter alternatives
                    for alt in &param.alternatives {
                        tx.execute(
                            "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) 
                            VALUES (?, ?, ?)",
                            &[&endpoint.id, &param.name, alt],
                        );
                    }
                }
            }

            endpoint_count += 1;
        }

        tracing::info!(
            email = %email,
            group_id = %group_id,
            endpoint_count = endpoint_count,
            "API group successfully added"
        );
        tx.commit().to_store_error()?;
        Ok(endpoint_count)
    // })
}
