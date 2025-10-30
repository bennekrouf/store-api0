// src/log_api_usage.rs
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::app_log;
use crate::endpoint_store::{EndpointStore, LogApiUsageRequest, LogApiUsageResponse};
/// Handler for logging detailed API usage with token information
pub async fn log_api_usage(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<LogApiUsageRequest>,
) -> impl Responder {
    let log_request = request.into_inner();

    app_log!(info,
        key_id = %log_request.key_id,
        email = %log_request.email,
        endpoint = %log_request.endpoint_path,
        method = %log_request.method,
        has_token_usage = log_request.usage.is_some(),
        total_tokens = log_request.usage.as_ref().map(|u| u.total_tokens),
        model = log_request.usage.as_ref().map(|u| u.model.as_str()).unwrap_or("none"),
        "Received HTTP log API usage request with token data"
    );

    match store.log_api_usage(&log_request).await {
        Ok(log_id) => {
            app_log!(info,
                key_id = %log_request.key_id,
                log_id = %log_id,
                total_tokens = log_request.usage.as_ref().map(|u| u.total_tokens),
                "Successfully logged API usage with token data"
            );
            HttpResponse::Ok().json(LogApiUsageResponse {
                success: true,
                message: "API usage logged successfully".to_string(),
                log_id: Some(log_id),
            })
        }
        Err(e) => {
            app_log!(error,
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
