use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;
/// Handler for deleting a single endpoint
pub async fn delete_endpoint(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>,
) -> impl Responder {
    let (email, endpoint_id) = path_params.into_inner();

    app_log!(info,
        email = %email,
        endpoint_id = %endpoint_id,
        "Received HTTP delete endpoint request"
    );

    match store.delete_user_endpoint(&email, &endpoint_id).await {
        Ok(deleted) => {
            if deleted {
                app_log!(info,
                    email = %email,
                    endpoint_id = %endpoint_id,
                    "Successfully deleted endpoint"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "Endpoint successfully deleted"
                }))
            } else {
                app_log!(warn,
                    email = %email,
                    endpoint_id = %endpoint_id,
                    "Endpoint not found or not deletable"
                );
                HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "message": "Endpoint not found or is a default endpoint that cannot be deleted"
                }))
            }
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                email = %email,
                endpoint_id = %endpoint_id,
                "Failed to delete endpoint"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to delete endpoint: {}", e)
            }))
        }
    }
}
