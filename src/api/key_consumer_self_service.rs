// src/api/key_consumer_self_service.rs
//
// Self-service consumer key API — authenticated with Firebase JWT.
//
// POST /api/consumer-keys/me
//   Body:    { "provider_tenant_id": "...", "key_name": "optional label" }
//   Creates a consumer key for the authenticated user linked to the given provider.
//   Returns the plaintext key (shown once only) plus connection instructions.
//
// GET  /api/consumer-keys/me
//   Returns all active consumer keys for the authenticated user, grouped by
//   provider, WITHOUT exposing the plaintext key (it was shown once at creation).
//
// Auth: Firebase JWT via `Authorization: Bearer <id_token>` (X-Firebase-Auth
//       header is also accepted for dashboard compatibility).
//
// The endpoint reuses the existing `generate_consumer_key_handler` logic but
// authenticates via Firebase JWT rather than X-Internal-Secret.

use crate::app_log;
use crate::endpoint_store::api_key_management::{
    extract_key_prefix, generate_secure_key, hash_api_key,
};
use crate::endpoint_store::tenant_management::get_default_tenant;
use crate::endpoint_store::EndpointStore;
use crate::infra::auth::FirebaseUser;
use actix_web::{web, HttpResponse, Responder};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

// ── Request bodies ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct GenerateSelfServiceKeyRequest {
    pub provider_tenant_id: String,
    pub key_name: Option<String>,
}

// ── POST /api/consumer-keys/me ────────────────────────────────────────────────

pub async fn generate_self_service_key(
    user: FirebaseUser,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<GenerateSelfServiceKeyRequest>,
) -> impl Responder {
    let consumer_email = user.email.trim().to_lowercase();
    let provider_tenant_id = body.provider_tenant_id.trim().to_string();
    let key_name = body
        .key_name
        .as_deref()
        .unwrap_or("My Claude MCP Key")
        .to_string();

    if provider_tenant_id.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "provider_tenant_id is required"
        }));
    }

    // 1. Verify the provider tenant exists
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "DB connection failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":"Database error"}));
        }
    };

    let provider_exists = client
        .query_opt(
            "SELECT id FROM tenants WHERE id = $1",
            &[&provider_tenant_id],
        )
        .await
        .ok()
        .flatten()
        .is_some();

    if !provider_exists {
        return HttpResponse::NotFound().json(serde_json::json!({
            "success": false,
            "error": format!("Provider '{}' not found", provider_tenant_id)
        }));
    }

    drop(client); // release before the tenant resolution calls below

    // 2. Ensure the consumer has a personal tenant row
    let consumer_tenant = match get_default_tenant(&store, &consumer_email).await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, consumer_email = %consumer_email, error = %e,
                     "Failed to resolve consumer tenant");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": "Failed to resolve consumer account"
            }));
        }
    };

    // 3. Generate the consumer key
    let new_key    = generate_secure_key();
    let key_hash   = hash_api_key(&new_key);
    let key_prefix = extract_key_prefix(&new_key);
    let key_id     = Uuid::new_v4().to_string();
    let now        = Utc::now();

    let mut client = match store.get_conn(Some(&consumer_tenant.id)).await {
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
                &key_id as &(dyn tokio_postgres::types::ToSql + Sync),
                &consumer_email as &(dyn tokio_postgres::types::ToSql + Sync),
                &key_hash as &(dyn tokio_postgres::types::ToSql + Sync),
                &key_prefix as &(dyn tokio_postgres::types::ToSql + Sync),
                &key_name as &(dyn tokio_postgres::types::ToSql + Sync),
                &now as &(dyn tokio_postgres::types::ToSql + Sync),
                &consumer_tenant.id as &(dyn tokio_postgres::types::ToSql + Sync),
                &provider_tenant_id as &(dyn tokio_postgres::types::ToSql + Sync),
            ],
        )
        .await;

    if let Err(e) = result {
        app_log!(error, error = %e, "Failed to insert consumer key");
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success":false,"error":"Failed to create key"}));
    }

    if let Err(e) = tx.commit().await {
        app_log!(error, error = %e, "Transaction commit failed");
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success":false,"error":"Database error"}));
    }

    app_log!(
        info,
        consumer_email = %consumer_email,
        provider_tenant_id = %provider_tenant_id,
        key_id = %key_id,
        "Self-service consumer key created"
    );

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "api_key": new_key,      // shown once — user must copy it now
        "key_prefix": key_prefix,
        "key_id": key_id,
        "provider_tenant_id": provider_tenant_id,
        "message": "Key created. Copy it now — it will not be shown again.",
        "mcp_url": "https://api.api0.ai/mcp"
    }))
}

// ── GET /api/consumer-keys/me ─────────────────────────────────────────────────

pub async fn list_self_service_keys(
    user: FirebaseUser,
    store: web::Data<Arc<EndpointStore>>,
) -> impl Responder {
    let consumer_email = user.email.trim().to_lowercase();
    let consumer_tenant = match get_default_tenant(&store, &consumer_email).await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, consumer_email = %consumer_email, error = %e, "list_keys: failed to resolve tenant");
            return HttpResponse::InternalServerError().json(serde_json::json!({"success":false,"error":"Failed to resolve account"}));
        }
    };

    let client = match store.get_conn(Some(&consumer_tenant.id)).await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "DB connection failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":"Database error"}));
        }
    };

    let rows = match client
        .query(
            "SELECT k.id, k.key_prefix, k.key_name, k.generated_at,
                    k.provider_tenant_id, t.name AS provider_name,
                    k.is_active
             FROM api_keys k
             LEFT JOIN tenants t ON t.id = k.provider_tenant_id
             WHERE k.email = $1
               AND k.provider_tenant_id IS NOT NULL
             ORDER BY k.generated_at DESC",
            &[&consumer_email],
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, consumer_email = %consumer_email,
                     "list_self_service_keys query failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success":false,"error":"Query failed"}));
        }
    };

    let keys: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            let ts: chrono::DateTime<chrono::Utc> = row.get(3);
            serde_json::json!({
                "id":                 row.get::<_, String>(0),
                "key_prefix":         row.get::<_, String>(1),
                "key_name":           row.get::<_, String>(2),
                "generated_at":       ts.to_rfc3339(),
                "provider_tenant_id": row.get::<_, Option<String>>(4),
                "provider_name":      row.get::<_, Option<String>>(5),
                "is_active":          row.get::<_, bool>(6),
            })
        })
        .collect();

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "keys": keys,
        "mcp_url": "https://api.api0.ai/mcp"
    }))
}
