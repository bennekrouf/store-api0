use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;
// use actix_web::{web, HttpResponse, Responder};

// Handler for revoking an API key
pub async fn revoke_api_key_handler(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>,
) -> impl Responder {
    let (mut tenant_id, key_id) = path_params.into_inner();
    app_log!(info, tenant_id = %tenant_id, key_id = %key_id, "Received HTTP revoke API key request");

    // If tenant_id looks like an email, resolve it to the actual tenant ID
    if tenant_id.contains('@') {
        use crate::endpoint_store::tenant_management;
        match tenant_management::get_default_tenant(&store, &tenant_id).await {
            Ok(t) => {
                app_log!(info, email = %tenant_id, resolved_tenant_id = %t.id, "Resolved email to tenant ID");
                tenant_id = t.id;
            },
            Err(e) => {
                app_log!(error, email = %tenant_id, error = %e, "Failed to resolve tenant for revoke lookup");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": "Account resolution failed"
                }));
            }
        }
    }

    match store.revoke_api_key(&tenant_id, &key_id).await {
        Ok(revoked) => {
            if revoked {
                app_log!(info,
                    tenant_id = %tenant_id,
                    "Successfully revoked API key"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "API key revoked successfully",
                }))
            } else {
                app_log!(warn,
                    tenant_id = %tenant_id,
                    "No API key found to revoke"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "No API key found to revoke",
                }))
            }
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                tenant_id = %tenant_id,
                "Failed to revoke API key"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to revoke API key: {}", e),
            }))
        }
    }
}
