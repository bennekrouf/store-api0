use crate::{check_is_default_group::check_is_default_group, endpoint_store::EndpointStore};
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for deleting an API group
pub async fn delete_api_group(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>,
) -> impl Responder {
    let (email, group_id) = path_params.into_inner();

    tracing::info!(
        email = %email,
        group_id = %group_id,
        "Received HTTP delete API group request"
    );

    // Check if group is a default group
    let is_default_group = match check_is_default_group(&store, &group_id).await {
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
            "message": "Cannot delete a default API group. Default groups are read-only."
        }));
    }

    match store.delete_user_api_group(&email, &group_id).await {
        Ok(deleted) => {
            if deleted {
                tracing::info!(
                    email = %email,
                    group_id = %group_id,
                    "Successfully deleted API group"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "API group and its endpoints successfully deleted"
                }))
            } else {
                tracing::warn!(
                    email = %email,
                    group_id = %group_id,
                    "API group not found or not deletable"
                );
                HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "message": "API group not found or is a default group that cannot be deleted"
                }))
            }
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                group_id = %group_id,
                "Failed to delete API group"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to delete API group: {}", e)
            }))
        }
    }
}
