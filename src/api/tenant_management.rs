use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

pub async fn verify_tenant_access(
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
) -> impl Responder {
    let (email, tenant_id) = path.into_inner();
    
    match store.verify_tenant_access(&email, &tenant_id).await {
        Ok(has_access) => {
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "has_access": has_access,
            }))
        }
        Err(e) => {
            app_log!(error, email = %email, tenant_id = %tenant_id, "Failed to verify tenant access: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Internal error: {}", e),
            }))
        }
    }
}
