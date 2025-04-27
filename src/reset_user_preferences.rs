use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;

// Handler for resetting user preferences
pub async fn reset_user_preferences(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP reset user preferences request");

    match store.reset_user_preferences(&email).await {
        Ok(_) => {
            tracing::info!(
                email = %email,
                "Successfully reset user preferences"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "User preferences successfully reset",
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to reset user preferences"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to reset user preferences: {}", e),
            }))
        }
    }
}
