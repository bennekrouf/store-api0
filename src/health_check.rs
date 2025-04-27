use actix_web::{HttpResponse, Responder};

// Health check endpoint
pub async fn health_check() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "service": "api-store-http",
        "version": env!("CARGO_PKG_VERSION")
    }))
}
