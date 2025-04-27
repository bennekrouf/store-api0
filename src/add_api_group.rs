use crate::{
    endpoint_store::{generate_id_from_text, EndpointStore},
    models::AddApiGroupRequest,
};

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for adding a new API group
pub async fn add_api_group(
    store: web::Data<Arc<EndpointStore>>,
    add_data: web::Json<AddApiGroupRequest>,
) -> impl Responder {
    let email = &add_data.email;
    let mut api_group = add_data.api_group.clone();

    tracing::info!(
        email = %email,
        group_name = %api_group.group.name,
        "Received HTTP add API group request"
    );

    // Validate group data
    if api_group.group.name.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "API group name cannot be empty"
        }));
    }

    if api_group.group.base.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Base URL cannot be empty"
        }));
    }

    // Generate ID if not provided
    if api_group.group.id.trim().is_empty() {
        api_group.group.id = generate_id_from_text(&api_group.group.name);
    }

    // Set group_id on all endpoints
    for endpoint in &mut api_group.endpoints {
        // Generate endpoint ID if not provided
        if endpoint.id.trim().is_empty() {
            endpoint.id = generate_id_from_text(&endpoint.text);
        }
        endpoint.group_id = api_group.group.id.clone();
    }

    // Add the API group
    let groups = vec![api_group.clone()];
    match store.replace_user_api_groups(email, groups).await {
        Ok(endpoint_count) => {
            tracing::info!(
                email = %email,
                group_id = %api_group.group.id,
                endpoint_count = endpoint_count,
                "Successfully added API group"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "API group successfully added",
                "group_id": api_group.group.id,
                "endpoint_count": endpoint_count
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                group_id = %api_group.group.id,
                "Failed to add API group"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to add API group: {}", e)
            }))
        }
    }
}
