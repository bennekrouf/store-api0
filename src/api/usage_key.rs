use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;
// use actix_web::{web, HttpResponse, Responder};

pub async fn get_api_key_usage(
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
) -> impl Responder {
    let (tenant_id, key_id) = path.into_inner();
    app_log!(info, tenant_id = %tenant_id, key_id = %key_id, "Received HTTP get API key usage request");

    match store.get_api_key_usage(&key_id, &tenant_id).await {
        Ok(usage) => {
            app_log!(info,
                tenant_id = %tenant_id,
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
                tenant_id = %tenant_id,
                "Failed to retrieve API key usage"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error: {}", e),
            }))
        }
    }
}
