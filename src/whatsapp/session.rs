// src/whatsapp/session.rs
//
// Conversation session storage — one row per (tenant, customer_phone).
// History is a JSONB array of Claude message objects.
//
// Bridge-internal (X-Internal-Secret):
//   GET /api/internal/whatsapp/session/{tenant_id}/{customer_phone}
//   PUT /api/internal/whatsapp/session/{tenant_id}/{customer_phone}

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

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
pub struct SessionPath {
    pub tenant_id: String,
    pub customer_phone: String,
}

#[derive(Deserialize)]
pub struct UpdateSessionRequest {
    pub history: serde_json::Value, // JSON array of Claude messages
}

// GET /api/internal/whatsapp/session/{tenant_id}/{customer_phone}
pub async fn get_session(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<SessionPath>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    match client.query_opt(
        "SELECT history FROM whatsapp_sessions WHERE tenant_id = $1 AND customer_phone = $2",
        &[&path.tenant_id, &path.customer_phone],
    ).await {
        Ok(Some(row)) => {
            let history: serde_json::Value = row.get(0);
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "history": history,
            }))
        }
        // No session yet — return empty history, not an error
        Ok(None) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "history": [],
        })),
        Err(e) => {
            app_log!(error, error = %e, "Failed to get WA session");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": e.to_string()}))
        }
    }
}

// PUT /api/internal/whatsapp/session/{tenant_id}/{customer_phone}
pub async fn update_session(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<SessionPath>,
    body: web::Json<UpdateSessionRequest>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    // Keep only the last 40 messages (20 turns) to bound context size
    let history = trim_history(&body.history, 40);

    match client.execute(
        "INSERT INTO whatsapp_sessions (tenant_id, customer_phone, history, last_active)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (tenant_id, customer_phone)
         DO UPDATE SET history = EXCLUDED.history, last_active = NOW()",
        &[&path.tenant_id, &path.customer_phone, &history],
    ).await {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({"success": true})),
        Err(e) => {
            app_log!(error, error = %e, "Failed to update WA session");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": e.to_string()}))
        }
    }
}

// DELETE /api/internal/whatsapp/sessions/stale?days=30
#[derive(Deserialize)]
pub struct StaleQuery {
    pub days: Option<i64>,
}

pub async fn cleanup_stale_sessions(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<StaleQuery>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let days = query.days.unwrap_or(30).max(1); // minimum 1 day

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    match client.execute(
        "DELETE FROM whatsapp_sessions WHERE last_active < NOW() - ($1 || ' days')::interval",
        &[&days.to_string()],
    ).await {
        Ok(count) => {
            app_log!(info, days = %days, deleted = %count, "Cleaned up stale WA sessions");
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "deleted": count,
            }))
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to cleanup stale WA sessions");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": e.to_string()}))
        }
    }
}

fn trim_history(history: &serde_json::Value, max: usize) -> serde_json::Value {
    if let Some(arr) = history.as_array() {
        if arr.len() > max {
            serde_json::Value::Array(arr[arr.len() - max..].to_vec())
        } else {
            history.clone()
        }
    } else {
        serde_json::json!([])
    }
}
