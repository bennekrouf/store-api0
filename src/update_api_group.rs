use crate::{
    check_is_default_group::check_is_default_group,
    endpoint_store::{generate_id_from_text, EndpointStore},
    models::UpdateApiGroupRequest,
};

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for updating an API group
pub async fn update_api_group(
    store: web::Data<Arc<EndpointStore>>,
    update_data: web::Json<UpdateApiGroupRequest>,
) -> impl Responder {
    let email = &update_data.email;
    let group_id = &update_data.group_id;
    let mut api_group = update_data.api_group.clone();

    tracing::info!(
        email = %email,
        group_id = %group_id,
        "Received HTTP update API group request"
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

    // Check if group is a default group
    let is_default_group = match check_is_default_group(&store, group_id).await {
        Ok(is_default) => is_default,
        Err(e) => {
            tracing::error!(
                error = %e,
                group_id = %group_id,
                "Failed to check if group is default"
            );
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to check group status: {}", e)
            }));
        }
    };

    if is_default_group {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Cannot update a default API group. Default groups are read-only."
        }));
    }

    // Ensure group ID is consistent
    api_group.group.id = group_id.clone();

    // Set group_id on all endpoints
    for endpoint in &mut api_group.endpoints {
        // Generate endpoint ID if not provided
        if endpoint.id.trim().is_empty() {
            endpoint.id = generate_id_from_text(&endpoint.text);
        }
        endpoint.group_id = group_id.clone();
    }

    // Update API group by first deleting and then adding
    match store.delete_user_api_group(email, group_id).await {
        Ok(_) => match store.add_user_api_group(email, &api_group).await {
            Ok(endpoint_count) => {
                tracing::info!(
                    email = %email,
                    group_id = %group_id,
                    endpoint_count = endpoint_count,
                    "Successfully updated API group"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "API group successfully updated",
                    "group_id": group_id,
                    "endpoint_count": endpoint_count
                }))
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    group_id = %group_id,
                    "Failed to add updated API group"
                );
                HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": format!("Failed to update API group: {}", e)
                }))
            }
        },
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                group_id = %group_id,
                "Failed to delete API group before update"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to update API group: {}", e)
            }))
        }
    }
}
