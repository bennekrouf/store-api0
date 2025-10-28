use actix_web::{web, HttpRequest, HttpResponse, Responder};
use std::sync::Arc;

use crate::{
    endpoint_store::EndpointStore,
    models::{ValidateKeyRequest, ValidateKeyResponse},
};

// Handler for validating an API key
pub async fn validate_api_key(
    store: web::Data<Arc<EndpointStore>>,
    req: web::Json<ValidateKeyRequest>,
    http_req: HttpRequest,
) -> impl Responder {
    // Try to get API key from request body first, then from Authorization header
    let api_key = if !req.api_key.is_empty() {
        req.api_key.clone()
    } else if let Some(auth_header) = http_req.headers().get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                auth_str.strip_prefix("Bearer ").unwrap_or("").to_string()
            } else {
                auth_str.to_string()
            }
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    if api_key.is_empty() {
        tracing::warn!("No API key provided in request body or Authorization header");
        return HttpResponse::BadRequest().json(ValidateKeyResponse {
            valid: false,
            email: None,
            key_id: None,
            message: "No API key provided".to_string(),
        });
    }

    tracing::info!(api_key = %api_key, "Validating API key");

    match store.validate_api_key(&api_key).await {
        Ok(Some((email, key_id))) => {
            tracing::info!(
                email = %email,
                key_id = %key_id,
                "API key validation successful"
            );

            HttpResponse::Ok().json(ValidateKeyResponse {
                valid: true,
                email: Some(email),
                key_id: Some(key_id),
                message: "API key is valid".to_string(),
            })
        }
        Ok(None) => {
            tracing::warn!(
                api_key = %api_key,
                "Invalid API key provided"
            );

            HttpResponse::Ok().json(ValidateKeyResponse {
                valid: false,
                email: None,
                key_id: None,
                message: "Invalid API key".to_string(),
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                api_key = %api_key,
                "Database error during API key validation"
            );

            HttpResponse::InternalServerError().json(ValidateKeyResponse {
                valid: false,
                email: None,
                key_id: None,
                message: "Validation error".to_string(),
            })
        }
    }
}
