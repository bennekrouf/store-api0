use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

pub async fn verify_tenant_access(
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
) -> impl Responder {
    let (email, mut tenant_id) = path.into_inner();
    app_log!(info, requester_email = %email, target_tenant_id = %tenant_id, "Verifying tenant access");
    
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
            app_log!(info, requester_email = %email, tenant_id = %tenant_id, has_access = has_access, "Access verification result");
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
    let email = email.into_inner().to_lowercase();
    
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

#[derive(Debug, Deserialize)]
pub struct SetActiveTenantRequest {
    pub email: String,
    pub tenant_id: String,
}

/// PUT /api/user/tenant/active — switch the workspace the caller acts on.
pub async fn set_active_tenant(
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SetActiveTenantRequest>,
) -> impl Responder {
    let email = body.email.to_lowercase();
    let tenant_id = body.tenant_id.trim();

    match crate::endpoint_store::tenant_management::set_active_tenant(&store, &email, tenant_id).await {
        Ok(Some(tenant)) => {
            app_log!(info, email = %email, tenant_id = %tenant.id, "Switched active workspace");
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "tenant": tenant,
            }))
        }
        Ok(None) => {
            app_log!(warn, email = %email, tenant_id = %tenant_id, "Refused a switch into a workspace the caller is not a member of");
            HttpResponse::Forbidden().json(serde_json::json!({
                "success": false,
                "message": "You are not a member of that workspace",
            }))
        }
        Err(e) => {
            app_log!(error, email = %email, tenant_id = %tenant_id, "Failed to switch workspace: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": "Could not switch workspace",
            }))
        }
    }
}
