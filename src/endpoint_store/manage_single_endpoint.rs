// src/endpoint_store/manage_single_endpoint.rs
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{Endpoint, EndpointStore, StoreError};

/// Manages (adds or updates) a single endpoint
pub async fn manage_single_endpoint(
    store: &EndpointStore,
    email: &str,
    endpoint: &Endpoint,
) -> Result<String, StoreError> {
    let mut conn = store.get_conn().await?;
    let tx = conn.transaction().to_store_error()?;

    let endpoint_id = &endpoint.id;
    let group_id = &endpoint.group_id;

    tracing::info!(
        email = %email,
        endpoint_id = %endpoint_id,
        group_id = %group_id,
        "Managing single endpoint"
    );

    // Check if user has access to this group
    let user_has_group: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM user_groups WHERE email = ? AND group_id = ?)",
            [email, group_id],
            |row| row.get(0),
        )
        .to_store_error()?;

    if !user_has_group {
        return Err(StoreError::Database(
            "User does not have access to this API group".to_string(),
        ));
    }

    // Check if endpoint exists
    let endpoint_exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM endpoints WHERE id = ?)",
            [endpoint_id],
            |row| row.get(0),
        )
        .to_store_error()?;

    let operation_type = if endpoint_exists {
        // Update existing endpoint
        tx.execute(
            "UPDATE endpoints SET text = ?, description = ?, verb = ?, base = ?, path = ?, group_id = ? WHERE id = ?",
            &[
                &endpoint.text,
                &endpoint.description,
                &endpoint.verb,
                &endpoint.base,
                &endpoint.path,
                group_id,
                endpoint_id,
            ],
        ).to_store_error()?;

        // Ensure user-endpoint association exists
        tx.execute(
            "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
            [email, endpoint_id],
        )
        .to_store_error()?;

        "updated"
    } else {
        // Create new endpoint
        tx.execute(
            "INSERT INTO endpoints (id, text, description, verb, base, path, group_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
            &[
                endpoint_id,
                &endpoint.text,
                &endpoint.description,
                &endpoint.verb,
                &endpoint.base,
                &endpoint.path,
                group_id,
            ],
        ).to_store_error()?;

        // Associate endpoint with user
        tx.execute(
            "INSERT INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
            [email, endpoint_id],
        )
        .to_store_error()?;

        "created"
    };

    // Clean up existing parameters
    tx.execute(
        "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
        [endpoint_id],
    )
    .to_store_error()?;

    tx.execute(
        "DELETE FROM parameters WHERE endpoint_id = ?",
        [endpoint_id],
    )
    .to_store_error()?;

    // Add parameters
    for param in &endpoint.parameters {
        tx.execute(
            "INSERT INTO parameters (endpoint_id, name, description, required) VALUES (?, ?, ?, ?)",
            &[
                endpoint_id,
                &param.name,
                &param.description,
                &param.required,
            ],
        )
        .to_store_error()?;

        // Add parameter alternatives
        for alt in &param.alternatives {
            tx.execute(
                "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) VALUES (?, ?, ?)",
                &[endpoint_id, &param.name, alt],
            ).to_store_error()?;
        }
    }

    tracing::info!(
        email = %email,
        endpoint_id = %endpoint_id,
        operation = %operation_type,
        "Successfully managed endpoint"
    );

    tx.commit().to_store_error()?;
    Ok(operation_type.to_string())
}
