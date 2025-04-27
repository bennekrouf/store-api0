use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;

// Handler for getting API key status
pub async fn get_api_keys_status(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP get API keys status request");

    match store.get_api_keys_status(&email).await {
        Ok(key_preference) => {
            tracing::info!(
                email = %email,
                has_keys = key_preference.has_keys,
                key_count = key_preference.active_key_count,
                "Successfully retrieved API keys status"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "keyPreference": key_preference,
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to retrieve API keys status"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error: {}", e),
            }))
        }
    }
}
