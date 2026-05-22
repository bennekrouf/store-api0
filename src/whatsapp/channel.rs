// src/whatsapp/channel.rs
//
// WhatsApp channel management — one channel per tenant.
//
// Gateway-proxied (no auth on store side — gateway has already verified Firebase JWT):
//   POST   /api/whatsapp/channel          body: { email, phone_number_id, wa_token, verify_token, system_prompt? }
//   GET    /api/whatsapp/channel/{email}
//   DELETE /api/whatsapp/channel/{email}
//
// Bridge-internal (X-Internal-Secret):
//   GET    /api/internal/whatsapp/channel/{phone_number_id}
//   GET    /api/internal/whatsapp/channel/by-tenant/{tenant_id}

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::endpoint_store::tenant_management::get_default_tenant;
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
pub struct RegisterChannelRequest {
    pub email: String,
    pub phone_number_id: String,
    pub wa_token: String,
    pub verify_token: String,
    pub system_prompt: Option<String>,
}

// POST /api/whatsapp/channel
pub async fn register_channel(
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<RegisterChannelRequest>,
) -> impl Responder {
    let tenant = match get_default_tenant(&store, &body.email).await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, email = %body.email, error = %e, "Tenant resolution failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Tenant resolution failed"}));
        }
    };

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    let system_prompt = body.system_prompt.as_deref().unwrap_or("");

    match client.execute(
        "INSERT INTO whatsapp_channels (phone_number_id, tenant_id, wa_token, verify_token, system_prompt)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (tenant_id) DO UPDATE
           SET phone_number_id = EXCLUDED.phone_number_id,
               wa_token        = EXCLUDED.wa_token,
               verify_token    = EXCLUDED.verify_token,
               system_prompt   = EXCLUDED.system_prompt",
        &[&body.phone_number_id, &tenant.id, &body.wa_token, &body.verify_token, &system_prompt],
    ).await {
        Ok(_) => {
            app_log!(info, tenant_id = %tenant.id, "WA channel registered");
            HttpResponse::Ok().json(serde_json::json!({"success": true}))
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": e.to_string()})),
    }
}

// GET /api/whatsapp/channel/{email}
pub async fn get_channel(
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    let email = path.into_inner();
    let tenant = match get_default_tenant(&store, &email).await {
        Ok(t) => t,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "Tenant error"})),
    };

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    match client.query_opt(
        "SELECT phone_number_id, system_prompt, created_at FROM whatsapp_channels WHERE tenant_id = $1",
        &[&tenant.id],
    ).await {
        Ok(Some(row)) => {
            let created_at: chrono::DateTime<chrono::Utc> = row.get(2);
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "channel": {
                    "phone_number_id": row.get::<_, &str>(0),
                    "system_prompt":   row.get::<_, &str>(1),
                    "created_at":      created_at.to_rfc3339(),
                }
            }))
        }
        Ok(None) => HttpResponse::Ok()
            .json(serde_json::json!({"success": true, "channel": null})),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": e.to_string()})),
    }
}

// DELETE /api/whatsapp/channel/{email}
pub async fn delete_channel(
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    let email = path.into_inner();
    let tenant = match get_default_tenant(&store, &email).await {
        Ok(t) => t,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "Tenant error"})),
    };

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    match client.execute(
        "DELETE FROM whatsapp_channels WHERE tenant_id = $1",
        &[&tenant.id],
    ).await {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({"success": true})),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": e.to_string()})),
    }
}

// GET /api/internal/whatsapp/channel/{phone_number_id}
pub async fn lookup_channel_internal(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }
    let phone_number_id = path.into_inner();
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };
    match client.query_opt(
        "SELECT tenant_id, wa_token, verify_token, system_prompt
         FROM whatsapp_channels WHERE phone_number_id = $1",
        &[&phone_number_id],
    ).await {
        Ok(Some(row)) => HttpResponse::Ok().json(serde_json::json!({
            "success":       true,
            "tenant_id":     row.get::<_, &str>(0),
            "wa_token":      row.get::<_, &str>(1),
            "verify_token":  row.get::<_, &str>(2),
            "system_prompt": row.get::<_, &str>(3),
        })),
        Ok(None) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "Channel not found"})),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": e.to_string()})),
    }
}

// GET /api/internal/whatsapp/channel/by-tenant/{tenant_id}
pub async fn lookup_channel_by_tenant_internal(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }
    let tenant_id = path.into_inner();
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };
    match client.query_opt(
        "SELECT phone_number_id, wa_token, verify_token, system_prompt
         FROM whatsapp_channels WHERE tenant_id = $1",
        &[&tenant_id],
    ).await {
        Ok(Some(row)) => HttpResponse::Ok().json(serde_json::json!({
            "success":        true,
            "phone_number_id": row.get::<_, &str>(0),
            "wa_token":       row.get::<_, &str>(1),
            "verify_token":   row.get::<_, &str>(2),
            "system_prompt":  row.get::<_, &str>(3),
        })),
        Ok(None) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "Channel not found"})),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": e.to_string()})),
    }
}
