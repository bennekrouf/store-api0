use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

#[derive(serde::Serialize)]
struct HealthResponse {
    status: String,
    database: String,
    timestamp: String,
}

pub async fn health_check(store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    let timestamp = chrono::Utc::now().to_rfc3339();

    let database_status = match store.health_check().await {
        Ok(_) => "healthy".to_string(),
        Err(e) => {
            app_log!(error, error = %e, "Database health check failed");
            format!("unhealthy: {}", e)
        }
    };

    let overall_status = if database_status == "healthy" {
        "healthy"
    } else {
        "unhealthy"
    };

    let response = HealthResponse {
        status: overall_status.to_string(),
        database: database_status,
        timestamp,
    };

    if overall_status == "healthy" {
        HttpResponse::Ok().json(response)
    } else {
        HttpResponse::ServiceUnavailable().json(response)
    }
}

