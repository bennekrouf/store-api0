use crate::endpoint_store::{generate_id_from_text, Endpoint, EndpointStore};
use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
pub struct ManageEndpointRequest {
    pub email: String,
    pub group_id: String,
    pub endpoint: Endpoint,
}

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
