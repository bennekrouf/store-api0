use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::{
    endpoint_store::EndpointStore,
    models::{RecordUsageRequest, RecordUsageResponse},
};
// use actix_web::{web, HttpResponse, Responder};

// Modify the record_api_key_usage handler in http_server.rs to accept a key_id parameter
pub async fn record_api_key_usage(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<RecordUsageRequest>,
) -> impl Responder {
    let key_id = &request.key_id;

    tracing::info!(key_id = %key_id, "Received HTTP record API key usage request");

    match store.record_api_key_usage(key_id).await {
        Ok(_) => {
            tracing::info!(
                key_id = %key_id,
                "Successfully recorded API key usage"
            );
            HttpResponse::Ok().json(RecordUsageResponse {
                success: true,
                message: "API key usage recorded successfully".to_string(),
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                key_id = %key_id,
                "Failed to record API key usage"
            );
            HttpResponse::InternalServerError().json(RecordUsageResponse {
                success: false,
                message: format!("Failed to record API key usage: {}", e),
            })
        }
    }
}
