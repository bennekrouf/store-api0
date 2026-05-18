use crate::email::{send_async, EmailKind};
use crate::endpoint_store::api_key_management::generate_api_key_with_provider;
use crate::endpoint_store::EndpointStore;
use crate::endpoint_store::GenerateKeyRequest;

use crate::app_log;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;
// Handler for generating a new API key
pub async fn generate_api_key(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<GenerateKeyRequest>,
) -> impl Responder {
    let email = &request.email;
    let key_name = &request.key_name;
    let provider_tenant_id = request.provider_tenant_id.as_deref();
    let explicit_tenant_id = request.tenant_id.as_deref();

    app_log!(info,
        email = %email,
        key_name = %key_name,
        tenant_id = ?explicit_tenant_id,
        provider_tenant_id = ?provider_tenant_id,
        "Received HTTP generate API key request"
    );

    match generate_api_key_with_provider(&store, email, key_name, explicit_tenant_id, provider_tenant_id).await {
        Ok((key, key_prefix, _)) => {
            app_log!(info, email = %email, key_prefix = %key_prefix, "Successfully generated API key");

            send_async(store.as_ref().clone(), email.clone(), EmailKind::KeyCreated {
                key_prefix: key_prefix.clone(),
                key_name: key_name.clone(),
            });

            // Send Welcome email on first key if not already sent.
            let store2 = store.as_ref().clone();
            let email2 = email.clone();
            let kp = key_prefix.clone();
            tokio::spawn(async move {
                if let Ok(client) = store2.get_admin_conn().await {
                    let sent: bool = client
                        .query_one("SELECT COALESCE(welcome_sent, false) FROM user_preferences WHERE email = $1", &[&email2])
                        .await
                        .map(|r| r.get(0))
                        .unwrap_or(true); // if row missing, skip
                    if !sent {
                        let _ = client.execute(
                            "UPDATE user_preferences SET welcome_sent = true WHERE email = $1",
                            &[&email2],
                        ).await;
                        let credits = client
                            .query_one("SELECT COALESCE(t.credit_balance,0) FROM user_preferences up JOIN tenants t ON up.default_tenant_id = t.id WHERE up.email = $1", &[&email2])
                            .await
                            .map(|r| r.get::<_, i64>(0))
                            .unwrap_or(0);
                        send_async(store2.clone(), email2.clone(), EmailKind::Welcome {
                            name: email2.split('@').next().unwrap_or("there").to_string(),
                            key_prefix: kp,
                            credits,
                        });
                    }
                }
            });

            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "API key generated successfully",
                "key": key,
                "keyPrefix": key_prefix,
            }))
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                email = %email,
                "Failed to generate API key"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to generate API key: {}", e),
            }))
        }
    }
}
