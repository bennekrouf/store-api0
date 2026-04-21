// src/generate_consumer_key_handler.rs
//
// POST /api/consumer-keys
//
// Providers (e.g. cvenom backend) call this to create an MCP consumer key for
// one of their end-users. The resulting key has provider_tenant_id set to the
// provider's tenant, so:
//   - tools/list returns the provider's tools
//   - credits are deducted from the consumer's balance
//
// Auth: X-Internal-Secret header (service-to-service, same secret as other
//       internal endpoints).
//
// Body:
//   {
//     "provider_email":  "admin@cvenom.com",   -- identifies the provider tenant
//     "consumer_email":  "alice@example.com",  -- the end-user who will use the key
//     "key_name":        "Alice's MCP key"     -- label shown in dashboard
//   }
//
// Response:
//   { "success": true, "api_key": "sk_live_...", "key_prefix": "sk_live_xx",
//     "key_id": "...", "consumer_email": "...", "provider_tenant_id": "..." }
//
// The plain-text key is returned only once — the caller must store it.

use crate::app_log;
use crate::endpoint_store::api_key_management::{
    extract_key_prefix, generate_secure_key, hash_api_key,
};
use crate::endpoint_store::tenant_management::get_default_tenant;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

fn check_internal_secret(req: &HttpRequest) -> bool {
    let expected = match std::env::var("API0_INTERNAL_SECRET") {
        Ok(s) if !s.is_empty() => s,
        _ => return false,
    };
    req.headers()
        .get("X-Internal-Secret")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == expected)
        .unwrap_or(false)
}

#[derive(Deserialize)]
pub struct GenerateConsumerKeyRequest {
    pub provider_email: String,
    pub consumer_email: String,
    pub key_name: Option<String>,
}

#[derive(Serialize)]
pub struct GenerateConsumerKeyResponse {
    pub success: bool,
    pub api_key: String,
    pub key_prefix: String,
    pub key_id: String,
    pub consumer_email: String,
    pub provider_tenant_id: String,
}

pub async fn generate_consumer_key_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<GenerateConsumerKeyRequest>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success":false,"error":"Unauthorized"}));
    }

    let provider_email = body.provider_email.trim().to_lowercase();
    let consumer_email = body.consumer_email.trim().to_lowercase();
    let key_name = body
        .key_name
        .as_deref()
        .unwrap_or("MCP Consumer Key")
        .to_string();

    // 1. Resolve provider tenant
    let provider_tenant = match get_default_tenant(&store, &provider_email).await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, provider_email = %provider_email, error = %e, "Provider tenant not found");
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": format!("Provider tenant not found for '{}'", provider_email)
            }));
        }
    };

    // 2. Ensure consumer user_preferences row exists (creates it if absent)
    //    Also ensure a personal tenant for the consumer so their credit balance can be tracked.
    let consumer_tenant = match get_default_tenant(&store, &consumer_email).await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, consumer_email = %consumer_email, error = %e, "Failed to resolve consumer tenant");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": "Failed to resolve consumer account"
            }));
        }
    };

    // 3. Generate the consumer key
    let new_key = generate_secure_key();
    let key_hash = hash_api_key(&new_key);
    let key_prefix = extract_key_prefix(&new_key);
    let key_id = Uuid::new_v4().to_string();
    let now = Utc::now();

    let mut client = match store.get_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "DB connection failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":"Database error"}));
        }
    };

    let tx = match client.transaction().await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, error = %e, "Transaction start failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":"Database error"}));
        }
    };

    let result = tx
        .execute(
            "INSERT INTO api_keys
                (id, email, key_hash, key_prefix, key_name,
                 generated_at, usage_count, is_active, tenant_id, provider_tenant_id)
             VALUES ($1, $2, $3, $4, $5, $6, 0, true, $7, $8)",
            &[
                &key_id,
                &consumer_email,
                &key_hash,
                &key_prefix,
                &key_name,
                &now,
                &consumer_tenant.id,
                &provider_tenant.id,
            ],
        )
        .await;

    match result {
        Ok(_) => {}
        Err(e) => {
            app_log!(error, error = %e, "Failed to insert consumer key");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":"Failed to create key"}));
        }
    }

    if let Err(e) = tx.commit().await {
        app_log!(error, error = %e, "Transaction commit failed");
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success":false,"error":"Database error"}));
    }

    app_log!(
        info,
        consumer_email = %consumer_email,
        provider_tenant_id = %provider_tenant.id,
        key_id = %key_id,
        "Consumer MCP key created"
    );

    HttpResponse::Ok().json(GenerateConsumerKeyResponse {
        success: true,
        api_key: new_key,
        key_prefix,
        key_id,
        consumer_email,
        provider_tenant_id: provider_tenant.id,
    })
}
