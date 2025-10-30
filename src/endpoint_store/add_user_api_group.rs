use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{ApiGroupWithEndpoints, EndpointStore, StoreError};
/// Adds a single API group for a user
pub async fn add_user_api_group(
    store: &EndpointStore,
    email: &str,
    api_group: &ApiGroupWithEndpoints,
) -> Result<usize, StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let group = &api_group.group;
    let group_id = &group.id;

    app_log!(info,
        email = %email,
        group_id = %group_id,
        "Adding API group"
    );

    // Check if group exists
    let group_exists_row = tx
        .query_opt("SELECT 1 FROM api_groups WHERE id = $1", &[group_id])
        .await
        .to_store_error()?;

    if group_exists_row.is_none() {
        tx.execute(
            "INSERT INTO api_groups (id, name, description, base) VALUES ($1, $2, $3, $4)",
            &[group_id, &group.name, &group.description, &group.base],
        )
        .await
        .to_store_error()?;
    } else {
        tx.execute(
            "UPDATE api_groups SET name = $1, description = $2, base = $3 WHERE id = $4",
            &[&group.name, &group.description, &group.base, group_id],
        )
        .await
        .to_store_error()?;
    }

    // Associate group with user
    tx.execute(
        "INSERT INTO user_groups (email, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        &[&email, group_id],
    )
    .await
    .to_store_error()?;

    let mut endpoint_count = 0;

    for endpoint in &api_group.endpoints {
        let endpoint_exists_row = tx
            .query_opt("SELECT 1 FROM endpoints WHERE id = $1", &[&endpoint.id])
            .await
            .to_store_error()?;

        if endpoint_exists_row.is_none() {
            tx.execute(
                "INSERT INTO endpoints (id, text, description, verb, base, path, group_id) 
                VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[
                    &endpoint.id,
                    &endpoint.text,
                    &endpoint.description,
                    &endpoint.verb,
                    &endpoint.base,
                    &endpoint.path,
                    group_id,
                ],
            )
            .await
            .to_store_error()?;
        } else {
            tx.execute(
                "UPDATE endpoints SET text = $1, description = $2, verb = $3, base = $4, path = $5, group_id = $6 WHERE id = $7",
                &[
                    &endpoint.text,
                    &endpoint.description,
                    &endpoint.verb,
                    &endpoint.base,
                    &endpoint.path,
                    group_id,
                    &endpoint.id,
                ],
            )
            .await
            .to_store_error()?;
        }

        tx.execute(
            "INSERT INTO user_endpoints (email, endpoint_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            &[&email, &endpoint.id],
        )
        .await
        .to_store_error()?;

        // Clean up existing parameters
        tx.execute(
            "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
            &[&endpoint.id],
        )
        .await
        .to_store_error()?;

        tx.execute(
            "DELETE FROM parameters WHERE endpoint_id = $1",
            &[&endpoint.id],
        )
        .await
        .to_store_error()?;

        // Add parameters
        for param in &endpoint.parameters {
            let required = param.required.parse::<bool>().unwrap_or(false);

            tx.execute(
                "INSERT INTO parameters (endpoint_id, name, description, required) 
                VALUES ($1, $2, $3, $4)",
                &[&endpoint.id, &param.name, &param.description, &required],
            )
            .await
            .to_store_error()?;

            for alt in &param.alternatives {
                tx.execute(
                    "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) 
                    VALUES ($1, $2, $3)",
                    &[&endpoint.id, &param.name, alt],
                )
                .await
                .to_store_error()?;
            }
        }

        endpoint_count += 1;
    }

    app_log!(info,
        email = %email,
        group_id = %group_id,
        endpoint_count = endpoint_count,
        "API group successfully added"
    );

    tx.commit().await.to_store_error()?;
    Ok(endpoint_count)
}
