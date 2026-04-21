use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;
// Handler for deleting an API group
pub async fn delete_api_group(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>,
) -> impl Responder {
    let (email, group_id) = path_params.into_inner();

    app_log!(info,
        email = %email,
        group_id = %group_id,
        "Received HTTP delete API group request"
    );

    match store.delete_user_api_group(&email, &group_id).await {
        Ok(deleted) => {
            if deleted {
                app_log!(info,
                    email = %email,
                    group_id = %group_id,
                    "Successfully deleted API group"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "API group and its endpoints successfully deleted"
                }))
            } else {
                app_log!(warn,
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
            app_log!(error,
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
