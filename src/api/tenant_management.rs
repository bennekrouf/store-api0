use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

pub async fn verify_tenant_access(
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
) -> impl Responder {
    let (email, mut tenant_id) = path.into_inner();
    
    // If tenant_id looks like an email, resolve it to the actual tenant ID
    if tenant_id.contains('@') {
        use crate::endpoint_store::tenant_management as tm;
        match tm::get_default_tenant(&store, &tenant_id).await {
            Ok(t) => {
                app_log!(info, email = %tenant_id, resolved_tenant_id = %t.id, "Resolved email to tenant ID for access verification");
                tenant_id = t.id;
            },
            Err(e) => {
                app_log!(error, email = %tenant_id, error = %e, "Failed to resolve tenant for access verification");
                // If we can't resolve the email to a tenant, then access is denied (or it doesn't exist)
                return HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "has_access": false,
                }));
            }
        }
    }

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

pub async fn list_user_tenants(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    
    match store.list_user_tenants(&email).await {
        Ok(tenants) => {
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "tenants": tenants,
            }))
        }
        Err(e) => {
            app_log!(error, email = %email, "Failed to list user tenants: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Internal error: {}", e),
            }))
        }
    }
}
