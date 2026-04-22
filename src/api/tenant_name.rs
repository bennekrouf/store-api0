use crate::endpoint_store::tenant_management::update_tenant_name;
use crate::endpoint_store::EndpointStore;
use crate::app_log;
use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct UpdateTenantNameRequest {
    pub email: String,
    pub name: String,
}

pub async fn update_tenant_name_handler(
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<UpdateTenantNameRequest>,
) -> impl Responder {
    let email = &body.email;
    let new_name = &body.name;

    app_log!(info, email = %email, new_name = %new_name, "Received HTTP update tenant name request");

    if new_name.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Tenant name cannot be empty"
        }));
    }

    match update_tenant_name(&store, email, new_name).await {
        Ok(_) => {
            app_log!(info, email = %email, "Successfully updated tenant name");
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "Tenant name updated successfully"
            }))
        }
        Err(e) => {
            app_log!(error, error = %e, email = %email, "Failed to update tenant name");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to update tenant name: {}", e)
            }))
        }
    }
}
