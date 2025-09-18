// src/manage_endpoint.rs
use crate::endpoint_store::db_helpers::ResultExt;
use crate::{
    endpoint_store::generate_id_from_text,
    // models::ManageEndpointRequest,
};
use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

// Handler for adding or updating a single endpoint
pub async fn manage_endpoint(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<ManageEndpointRequest>,
) -> impl Responder {
    let email = &request.email;
    let mut endpoint = request.endpoint.clone();
    let group_id = &request.group_id;

    tracing::info!(
        email = %email,
        group_id = %group_id,
        endpoint_text = %endpoint.text,
        "Received HTTP manage endpoint request"
    );

    // Validate endpoint data
    if endpoint.text.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Endpoint text cannot be empty"
        }));
    }

    // Generate ID if not provided
    if endpoint.id.trim().is_empty() {
        endpoint.id = generate_id_from_text(&endpoint.text);
    }

    // Set group_id
    endpoint.group_id = group_id.clone();

    // If endpoint base is empty, inherit from group
    if endpoint.base.trim().is_empty() {
        // Get the group's base URL
        match store.get_group_base_url(group_id).await {
            Ok(group_base) => {
                if group_base.trim().is_empty() {
                    endpoint.base = "https://api.example.com".to_string();
                } else {
                    endpoint.base = group_base;
                }
            }
            Err(_) => {
                endpoint.base = "https://api.example.com".to_string();
            }
        }
    }

    match store.manage_single_endpoint(email, &endpoint).await {
        Ok(operation_type) => {
            tracing::info!(
                email = %email,
                endpoint_id = %endpoint.id,
                operation = %operation_type,
                "Successfully managed endpoint"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": format!("Endpoint successfully {}", operation_type),
                "endpoint_id": endpoint.id,
                "operation": operation_type
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                endpoint_id = %endpoint.id,
                "Failed to manage endpoint"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to manage endpoint: {}", e)
            }))
        }
    }
}

// src/models.rs - Add this to your existing models
#[derive(Debug, Clone, Deserialize)]
pub struct ManageEndpointRequest {
    pub email: String,
    pub group_id: String,
    pub endpoint: Endpoint, // Using the existing Endpoint struct
}

// src/endpoint_store/manage_single_endpoint.rs
// use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{Endpoint, EndpointStore, StoreError};

/// Manages (adds or updates) a single endpoint
pub async fn manage_single_endpoint(
    store: &EndpointStore,
    email: &str,
    endpoint: &Endpoint,
) -> Result<String, StoreError> {
    let mut conn = store.get_conn().await?;

    // Use a scoped transaction to ensure proper cleanup
    let operation_type = {
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
            ).to_store_error()?;

            // Add parameter alternatives
            for alt in &param.alternatives {
                tx.execute(
                    "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) VALUES (?, ?, ?)",
                    &[endpoint_id, &param.name, alt],
                ).to_store_error()?;
            }
        }

        // Commit the transaction before returning
        tx.commit().to_store_error()?;
        operation_type
    }; // Transaction is dropped here

    tracing::info!(
        email = %email,
        endpoint_id = %endpoint.id,
        operation = %operation_type,
        "Successfully managed endpoint"
    );

    Ok(operation_type.to_string())
}
