use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{Endpoint, EndpointStore, StoreError};
/// Manages (adds or updates) a single endpoint
pub async fn manage_single_endpoint(
    store: &EndpointStore,
    email: &str,
    endpoint: &Endpoint,
) -> Result<String, StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let endpoint_id = &endpoint.id;
    let group_id = &endpoint.group_id;

    app_log!(info,
        email = %email,
        endpoint_id = %endpoint_id,
        group_id = %group_id,
        "Managing single endpoint"
    );

    // Check if user has access to this group
    let user_has_group_row = tx
        .query_opt(
            "SELECT 1 FROM user_groups WHERE email = $1 AND group_id = $2",
            &[&email, group_id],
        )
        .await
        .to_store_error()?;

    if user_has_group_row.is_none() {
        return Err(StoreError::Database(
            "User does not have access to this API group".to_string(),
        ));
    }

    // Check if endpoint exists
    let endpoint_exists_row = tx
        .query_opt("SELECT 1 FROM endpoints WHERE id = $1", &[endpoint_id])
        .await
        .to_store_error()?;

    let operation_type = if endpoint_exists_row.is_some() {
        // Update existing endpoint
        tx.execute(
            "UPDATE endpoints SET text = $1, description = $2, verb = $3, base = $4, path = $5, group_id = $6 WHERE id = $7",
            &[
                &endpoint.text,
                &endpoint.description,
                &endpoint.verb,
                &endpoint.base,
                &endpoint.path,
                group_id,
                endpoint_id,
            ],
        )
        .await
        .to_store_error()?;

        // Ensure user-endpoint association exists
        tx.execute(
            "INSERT INTO user_endpoints (email, endpoint_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            &[&email, endpoint_id],
        )
        .await
        .to_store_error()?;

        "updated"
    } else {
        // Create new endpoint
        tx.execute(
            "INSERT INTO endpoints (id, text, description, verb, base, path, group_id) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                endpoint_id,
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

        // Associate endpoint with user
        tx.execute(
            "INSERT INTO user_endpoints (email, endpoint_id) VALUES ($1, $2)",
            &[&email, endpoint_id],
        )
        .await
        .to_store_error()?;

        "created"
    };

    // Clean up existing parameters
    tx.execute(
        "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
        &[endpoint_id],
    )
    .await
    .to_store_error()?;

    tx.execute(
        "DELETE FROM parameters WHERE endpoint_id = $1",
        &[endpoint_id],
    )
    .await
    .to_store_error()?;

    // Add parameters
    for param in &endpoint.parameters {
        let required = param.required.parse::<bool>().unwrap_or(false);

        tx.execute(
            "INSERT INTO parameters (endpoint_id, name, description, required) VALUES ($1, $2, $3, $4)",
            &[endpoint_id, &param.name, &param.description, &required],
        )
        .await
        .to_store_error()?;

        // Add parameter alternatives
        for alt in &param.alternatives {
            tx.execute(
                "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) VALUES ($1, $2, $3)",
                &[endpoint_id, &param.name, alt],
            )
            .await
            .to_store_error()?;
        }
    }

    tx.commit().await.to_store_error()?;

    app_log!(info,
        email = %email,
        endpoint_id = %endpoint.id,
        operation = %operation_type,
        "Successfully managed endpoint"
    );

    Ok(operation_type.to_string())
}
