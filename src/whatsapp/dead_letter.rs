// src/whatsapp/dead_letter.rs
//
// Dead-letter queue for failed WhatsApp message processing.
//
// Bridge-internal (X-Internal-Secret):
//   POST /api/internal/whatsapp/failed-messages   — insert a failed message
//   GET  /api/internal/whatsapp/failed-messages/{tenant_id}?limit=50 — list recent failures

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
pub struct InsertFailedMessage {
    pub tenant_id: String,
    pub customer_phone: String,
    pub message_text: Option<String>,
    pub error_type: String,
    pub error_detail: String,
    pub payload: Option<serde_json::Value>,
}

// POST /api/internal/whatsapp/failed-messages
pub async fn insert_failed_message(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<InsertFailedMessage>,
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

    let msg_text = body.message_text.as_deref().unwrap_or("");
    let payload = body.payload.clone().unwrap_or(serde_json::json!(null));

    match client.execute(
        "INSERT INTO whatsapp_failed_messages
         (tenant_id, customer_phone, message_text, error_type, error_detail, payload)
         VALUES ($1, $2, $3, $4, $5, $6)",
        &[
            &body.tenant_id,
            &body.customer_phone,
            &msg_text,
            &body.error_type,
            &body.error_detail,
            &payload,
        ],
    ).await {
        Ok(_) => {
            app_log!(
                info,
                tenant_id = %body.tenant_id,
                error_type = %body.error_type,
                "Dead-lettered failed WA message"
            );
            HttpResponse::Ok().json(serde_json::json!({"success": true}))
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to insert dead letter");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": e.to_string()}))
        }
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub limit: Option<i64>,
}

// GET /api/internal/whatsapp/failed-messages/{tenant_id}
pub async fn list_failed_messages(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
    query: web::Query<ListQuery>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let tenant_id = path.into_inner();
    let limit = query.limit.unwrap_or(50).min(500);

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    match client.query(
        "SELECT id, customer_phone, message_text, error_type, error_detail, payload, created_at
         FROM whatsapp_failed_messages
         WHERE tenant_id = $1
         ORDER BY created_at DESC
         LIMIT $2",
        &[&tenant_id, &limit],
    ).await {
        Ok(rows) => {
            let messages: Vec<serde_json::Value> = rows.iter().map(|row| {
                let created_at: chrono::DateTime<chrono::Utc> = row.get(6);
                serde_json::json!({
                    "id": row.get::<_, i64>(0),
                    "customer_phone": row.get::<_, String>(1),
                    "message_text": row.get::<_, String>(2),
                    "error_type": row.get::<_, String>(3),
                    "error_detail": row.get::<_, String>(4),
                    "payload": row.get::<_, Option<serde_json::Value>>(5),
                    "created_at": created_at.to_rfc3339(),
                })
            }).collect();
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "messages": messages,
            }))
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to list dead letters");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": e.to_string()}))
        }
    }
}
