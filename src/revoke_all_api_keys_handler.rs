use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;
// Handler for revoking all API keys for a user
pub async fn revoke_all_api_keys_handler(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    app_log!(info, email = %email, "Received HTTP revoke all API keys request");

    match store.revoke_all_api_keys(&email).await {
        Ok(count) => {
            app_log!(info,
                email = %email,
                count = count,
                "Successfully revoked all API keys"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": format!("Successfully revoked {} API keys", count),
                "count": count,
            }))
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                email = %email,
                "Failed to revoke all API keys"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to revoke all API keys: {}", e),
            }))
        }
    }
}
