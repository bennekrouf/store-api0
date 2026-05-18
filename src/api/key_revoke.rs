use crate::app_log;
use crate::email::{send_async, EmailKind};
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

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
                app_log!(info, tenant_id = %tenant_id, "Successfully revoked API key");

                // Look up owner email for the notification.
                let store2 = store.as_ref().clone();
                let tid = tenant_id.clone();
                let kid = key_id.clone();
                tokio::spawn(async move {
                    if let Ok(client) = store2.get_admin_conn().await {
                        // Get owner email
                        if let Ok(row) = client.query_opt(
                            "SELECT email FROM tenant_users WHERE tenant_id = $1 AND role = 'owner' LIMIT 1",
                            &[&tid],
                        ).await {
                            if let Some(r) = row {
                                let email: &str = r.get(0);
                                // Get key prefix for display
                                let prefix = client
                                    .query_opt("SELECT key_prefix FROM api_keys WHERE id = $1", &[&kid])
                                    .await
                                    .ok()
                                    .flatten()
                                    .map(|r| r.get::<_, String>(0))
                                    .unwrap_or_else(|| kid[..8.min(kid.len())].to_string());
                                send_async(store2, email.to_string(), EmailKind::KeyRevoked { key_prefix: prefix });
                            }
                        }
                    }
                });

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
