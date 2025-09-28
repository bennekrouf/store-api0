// src/get_api_usage_logs.rs
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;

/// Handler for getting detailed API usage logs with token information
pub async fn get_api_usage_logs(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>, // (email, key_id)
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let (email, key_id) = path_params.into_inner();

    // Extract limit from query parameters if provided
    let limit = query.get("limit").and_then(|l| l.parse::<i64>().ok());

    tracing::info!(
        email = %email,
        key_id = %key_id,
        limit = limit,
        "Received HTTP get API usage logs request"
    );

    match store.get_api_usage_logs(&key_id, limit).await {
        Ok(logs) => {
            tracing::info!(
                email = %email,
                key_id = %key_id,
                log_count = logs.len(),
                "Successfully retrieved API usage logs with token data"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "logs": logs,
                "count": logs.len(),
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                key_id = %key_id,
                "Failed to retrieve API usage logs"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to retrieve API usage logs: {}", e),
            }))
        }
    }
}
