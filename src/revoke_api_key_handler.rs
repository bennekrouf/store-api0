use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;
// use actix_web::{web, HttpResponse, Responder};

// Handler for revoking an API key
pub async fn revoke_api_key_handler(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>,
) -> impl Responder {
    let (email, key_id) = path_params.into_inner();
    tracing::info!(email = %email, "Received HTTP revoke API key request");

    match store.revoke_api_key(&email, &key_id).await {
        Ok(revoked) => {
            if revoked {
                tracing::info!(
                    email = %email,
                    "Successfully revoked API key"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "API key revoked successfully",
                }))
            } else {
                tracing::warn!(
                    email = %email,
                    "No API key found to revoke"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "No API key found to revoke",
                }))
            }
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to revoke API key"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to revoke API key: {}", e),
            }))
        }
    }
}
