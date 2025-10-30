use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{
    generate_id_from_text, ApiGroupWithEndpoints, EndpointStore, StoreError,
};
/// Replaces all API groups and endpoints for a user
pub async fn replace_user_api_groups(
    store: &EndpointStore,
    email: &str,
    api_groups: Vec<ApiGroupWithEndpoints>,
) -> Result<usize, StoreError> {
    app_log!(info, email = %email, "Starting complete API group replacement");

    // Clean up existing user data
    match store.force_clean_user_data(email).await {
        Ok(_) => {
            app_log!(info, email = %email, "Successfully cleaned up user data");
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                email = %email,
                "Failed to clean up user data, will try fallback approach"
            );

            match store.fallback_clean_user_data(email).await {
                Ok(_) => app_log!(info, email = %email, "Fallback cleanup successful"),
                Err(e) => {
                    app_log!(error,
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
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

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

        if group_exists_row.is_none() {
            app_log!(debug, group_id = %group_id, "Creating new API group");
            tx.execute(
                "INSERT INTO api_groups (id, name, description, base) VALUES ($1, $2, $3, $4)",
                &[&group_id, &group.name, &group.description, &group.base],
            )
            .await
            .to_store_error()?;
        } else {
            app_log!(debug, group_id = %group_id, "Updating existing API group");
            tx.execute(
                "UPDATE api_groups SET name = $1, description = $2, base = $3 WHERE id = $4",
                &[&group.name, &group.description, &group.base, &group_id],
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
            let endpoint_id = if endpoint.id.is_empty() {
                generate_id_from_text(&endpoint.text)
            } else {
                endpoint.id.clone()
            };

            let endpoint_exists_row = tx
                .query_opt("SELECT 1 FROM endpoints WHERE id = $1", &[&endpoint_id])
                .await
                .to_store_error()?;

            if endpoint_exists_row.is_none() {
                app_log!(debug, endpoint_id = %endpoint_id, "Creating new endpoint");
                tx.execute(
                    "INSERT INTO endpoints (id, text, description, verb, base, path, group_id) 
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    &[
                        &endpoint_id,
                        &endpoint.text,
                        &endpoint.description,
                        &endpoint.verb,
                        &endpoint.base,
                        &endpoint.path,
                        &group_id,
                    ],
                )
                .await
                .to_store_error()?;
            } else {
                app_log!(debug, endpoint_id = %endpoint_id, "Updating existing endpoint");
                tx.execute(
                    "UPDATE endpoints SET text = $1, description = $2, verb = $3, base = $4, path = $5, group_id = $6 WHERE id = $7",
                    &[
                        &endpoint.text,
                        &endpoint.description,
                        &endpoint.verb,
                        &endpoint.base,
                        &endpoint.path,
                        &group_id,
                        &endpoint_id,
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
