use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;
// use actix_web::{web, HttpResponse, Responder};

pub async fn get_api_key_usage(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    app_log!(info, email = %email, "Received HTTP get API key usage request");

    match store.get_api_key_usage(&email).await {
        Ok(usage) => {
            app_log!(info,
                email = %email,
                "Successfully retrieved API key usage"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "usage": usage,
            }))
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                email = %email,
                "Failed to retrieve API key usage"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error: {}", e),
            }))
        }
    }
}
