use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::{
    endpoint_store::EndpointStore,
    models::{ValidateKeyRequest, ValidateKeyResponse},
};

// Handler for validating an API key
pub async fn validate_api_key(
    store: web::Data<Arc<EndpointStore>>,
    key_data: web::Json<ValidateKeyRequest>,
) -> impl Responder {
    let api_key = &key_data.api_key;

    tracing::info!("Received HTTP validate API key request");

    match store.validate_api_key(api_key).await {
        Ok(Some((key_id, email))) => {
            // Record usage for this key
            if let Err(e) = store.record_api_key_usage(&key_id).await {
                tracing::warn!(
                    error = %e,
                    key_id = %key_id,
                    "Failed to record API key usage but proceeding with validation"
                );
            }

            tracing::info!(
                email = %email,
                key_id = %key_id,
                "Successfully validated API key"
            );
            HttpResponse::Ok().json(ValidateKeyResponse {
                valid: true,
                email: Some(email),
                key_id: Some(key_id),
                message: "API key is valid".to_string(),
            })
        }
        Ok(None) => {
            tracing::warn!("Invalid API key provided");
            HttpResponse::Ok().json(ValidateKeyResponse {
                valid: false,
                email: None,
                key_id: None,
                message: "API key is invalid".to_string(),
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "Failed to validate API key"
            );
            HttpResponse::InternalServerError().json(ValidateKeyResponse {
                valid: false,
                email: None,
                key_id: None,
                message: format!("Error validating API key: {}", e),
            })
        }
    }
}
