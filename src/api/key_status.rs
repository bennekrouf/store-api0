use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for getting API key status
pub async fn get_api_keys_status(
    store: web::Data<Arc<EndpointStore>>,
    tenant_id: web::Path<String>,
) -> impl Responder {
    let mut tenant_id = tenant_id.into_inner();
    app_log!(info, tenant_id = %tenant_id, "Received HTTP get API keys status request");

    // If tenant_id looks like an email, resolve it to the actual tenant ID
    if tenant_id.contains('@') {
        use crate::endpoint_store::tenant_management;
        match tenant_management::get_default_tenant(&store, &tenant_id).await {
            Ok(t) => {
                app_log!(info, email = %tenant_id, resolved_tenant_id = %t.id, "Resolved email to tenant ID");
                tenant_id = t.id;
            },
            Err(e) => {
                app_log!(error, email = %tenant_id, error = %e, "Failed to resolve tenant for key status lookup");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": "Account resolution failed"
                }));
            }
        }
    }

    match store.get_api_keys_status(&tenant_id).await {
        Ok(key_preference) => {
            app_log!(info,
                tenant_id = %tenant_id,
                has_keys = key_preference.has_keys,
                key_count = key_preference.active_key_count,
                "Successfully retrieved API keys status"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "keyPreference": key_preference,
            }))
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                tenant_id = %tenant_id,
                "Failed to retrieve API keys status"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error: {}", e),
            }))
        }
    }
}
