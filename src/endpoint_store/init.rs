use crate::endpoint_store::{EndpointStore, StoreError, ApiGroupWithEndpoints, generate_id_from_text};
// use duckdb::Connection;
use std::path::Path;
use r2d2_duckdb::DuckDBConnectionManager;
use r2d2::Pool;

use crate::endpoint_store::db_helpers::ResultExt;
/// Creates a new EndpointStore instance
pub(crate) fn new_endpoint_store<P: AsRef<Path>>(db_path: P) -> Result<EndpointStore, StoreError> {
    tracing::info!(
        "Initializing EndpointStore with path: {:?}",
        db_path.as_ref()
    );

    // Create the connection manager
    let manager = DuckDBConnectionManager::file(db_path.as_ref());
    
    // Build the connection pool
    let pool = Pool::builder()
        .max_size(10)
        .build(manager)
        .map_err(|e| StoreError::Pool(e.to_string()))?;

     let conn = pool.get()
        .map_err(|e| StoreError::Pool(e.to_string()))?;
    
    tracing::debug!("DuckDB connection established");
    
    // Create tables with the schema
    conn.execute_batch(include_str!("../../sql/schema.sql"))
        .to_store_error()?;
    
    Ok(EndpointStore { pool })
}

/// Initializes the database with default API groups if it's empty
pub(crate) fn initialize_if_empty(
    store: &mut EndpointStore,
    default_api_groups: &[ApiGroupWithEndpoints],
) -> Result<(), StoreError> {
    let mut conn = store.get_conn()?;
    let tx = conn.transaction().to_store_error()?;

    // Check if we already have default endpoints
    let count: i64 = tx.query_row(
        "SELECT COUNT(*) FROM api_groups WHERE is_default = true",
        [],
        |row| row.get(0),
    ).to_store_error()?;

    if count > 0 {
        // Default API groups already exist, no need to create them
        tracing::info!("Default API groups already exist. Skipping initialization.");
        return Ok(());
    }

    tracing::info!("Initializing database with default API groups and endpoints");

    // Create default API groups and endpoints
    for group_with_endpoints in default_api_groups {
        let group = &group_with_endpoints.group;

        // Insert the API group
        tx.execute(
            "INSERT INTO api_groups (id, name, description, base, is_default) VALUES (?, ?, ?, ?, true)",
            &[
                &group.id,
                &group.name,
                &group.description,
                &group.base,
            ],
        ).to_store_error()?;

        tracing::debug!(
            group_id = %group.id,
            group_name = %group.name,
            "Inserted API group"
        );

        // Insert endpoints for this group
        for endpoint in &group_with_endpoints.endpoints {
            // Generate ID if not provided
            let endpoint_id = if endpoint.id.is_empty() {
                generate_id_from_text(&endpoint.text)
            } else {
                endpoint.id.clone()
            };

            // Insert endpoint
            tx.execute(
                "INSERT INTO endpoints (id, text, description, verb, base, path, group_id, is_default) 
                VALUES (?, ?, ?, ?, ?, ?, ?, true)",
                &[
                    &endpoint_id,
                    &endpoint.text,
                    &endpoint.description,
                    &endpoint.verb,
                    &endpoint.base,
                    &endpoint.path,
                    &group.id,
                ],
            ).to_store_error()?;

            tracing::debug!(
                endpoint_id = %endpoint_id,
                endpoint_text = %endpoint.text,
                "Inserted endpoint"
            );

            // Insert parameters
            for param in &endpoint.parameters {
                tx.execute(
                    "INSERT INTO parameters (endpoint_id, name, description, required) 
                    VALUES (?, ?, ?, ?)",
                    &[
                        &endpoint_id,
                        &param.name,
                        &param.description,
                        &param.required.to_string(),
                    ],
                ).to_store_error()?;

                // Insert parameter alternatives
                for alt in &param.alternatives {
                    tx.execute(
                        "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) 
                        VALUES (?, ?, ?)",
                        &[&endpoint_id, &param.name, alt],
                    ).to_store_error()?;
                }
            }
        }
    }

    // Create a default user for testing if none exists
    // This is optional but helpful during development
    let default_email = "default@example.com";

    // Associate default groups with the default user
    for group_with_endpoints in default_api_groups {
        // Associate group with default user
        tx.execute(
            "INSERT OR IGNORE INTO user_groups (email, group_id) VALUES (?, ?)",
            &[default_email, &group_with_endpoints.group.id],
        ).to_store_error()?;

        // Associate endpoints with default user
        for endpoint in &group_with_endpoints.endpoints {
            tx.execute(
                "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                &[default_email, &endpoint.id],
            ).to_store_error()?;
        }
    }

    tracing::info!(
        group_count = default_api_groups.len(),
        "Successfully initialized database with default API groups and endpoints"
    );

    tx.commit().to_store_error()?;
    Ok(())
}
