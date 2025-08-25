// src/log_api_usage.rs
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::{
    endpoint_store::{EndpointStore, LogApiUsageRequest, LogApiUsageResponse},
};

/// Handler for logging detailed API usage
pub async fn log_api_usage(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<LogApiUsageRequest>,
) -> impl Responder {
    let log_request = request.into_inner();

    tracing::info!(
        key_id = %log_request.key_id,
        email = %log_request.email,
        endpoint = %log_request.endpoint_path,
        method = %log_request.method,
        "Received HTTP log API usage request"
    );

    match store.log_api_usage(&log_request).await {
        Ok(log_id) => {
            tracing::info!(
                key_id = %log_request.key_id,
                log_id = %log_id,
                "Successfully logged API usage"
            );
            HttpResponse::Ok().json(LogApiUsageResponse {
                success: true,
                message: "API usage logged successfully".to_string(),
                log_id: Some(log_id),
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                key_id = %log_request.key_id,
                "Failed to log API usage"
            );
            HttpResponse::InternalServerError().json(LogApiUsageResponse {
                success: false,
                message: format!("Failed to log API usage: {}", e),
                log_id: None,
            })
        }
    }
}
