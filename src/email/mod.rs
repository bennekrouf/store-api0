// src/email/mod.rs
//
// Email sending for all api0-family services.
//
// SMTP config is stored in the system_config table:
//   email.smtp_host / email.smtp_port / email.smtp_user / email.smtp_password / email.from_addr
//
// Endpoints (all require X-Internal-Secret):
//   POST /api/internal/email/send          — send an email (called by cvenom, other services)
//   GET  /api/admin/smtp-config            — read current config (password masked)
//   PUT  /api/admin/smtp-config            — save config

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

// ── Auth ──────────────────────────────────────────────────────────────────────

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

// ── Config helpers ────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct SmtpConfig {
    host:      String,
    port:      u16,
    user:      String,
    password:  String,
    from_addr: String,
}

async fn load_smtp_config(store: &EndpointStore) -> Option<SmtpConfig> {
    let client = store.get_admin_conn().await.ok()?;
    let rows = client
        .query(
            "SELECT key, value FROM system_config WHERE key LIKE 'email.%'",
            &[],
        )
        .await
        .ok()?;

    let mut map = std::collections::HashMap::new();
    for row in &rows {
        let key: &str = row.get(0);
        let val: &str = row.get(1);
        map.insert(key.to_string(), val.to_string());
    }

    // Fall back to env vars when DB has no config.
    let host = map
        .get("email.smtp_host")
        .cloned()
        .or_else(|| std::env::var("SMTP_HOST").ok())?;
    let user = map
        .get("email.smtp_user")
        .cloned()
        .or_else(|| std::env::var("SMTP_USER").ok())?;
    let password = map
        .get("email.smtp_password")
        .cloned()
        .or_else(|| std::env::var("SMTP_PASSWORD").ok())?;
    let port: u16 = map
        .get("email.smtp_port")
        .and_then(|v| v.parse().ok())
        .or_else(|| std::env::var("SMTP_PORT").ok().and_then(|v| v.parse().ok()))
        .unwrap_or(587);
    let from_addr = map
        .get("email.from_addr")
        .cloned()
        .or_else(|| std::env::var("EMAIL_FROM").ok())
        .unwrap_or_else(|| user.clone());

    Some(SmtpConfig { host, port, user, password, from_addr })
}

async fn save_config_key(store: &EndpointStore, key: &str, value: &str) -> anyhow::Result<()> {
    let client = store.get_admin_conn().await?;
    client
        .execute(
            "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW())
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
            &[&key, &value],
        )
        .await?;
    Ok(())
}

// ── POST /api/internal/email/send ─────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SendEmailRequest {
    pub to:        String,
    pub subject:   String,
    pub html_body: String,
}

pub async fn send_email_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SendEmailRequest>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let cfg = match load_smtp_config(&store).await {
        Some(c) => c,
        None => {
            app_log!(error, "SMTP not configured — set email.smtp_* in system_config or SMTP_* env vars");
            return HttpResponse::ServiceUnavailable()
                .json(serde_json::json!({"success": false, "error": "SMTP not configured"}));
        }
    };

    let email = match Message::builder()
        .from(format!("api0 <{}>", cfg.from_addr).parse().unwrap())
        .to(body.to.parse().unwrap())
        .subject(&body.subject)
        .header(ContentType::TEXT_HTML)
        .body(body.html_body.clone())
    {
        Ok(m) => m,
        Err(e) => {
            app_log!(error, "Failed to build email to {}: {}", body.to, e);
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"success": false, "error": format!("{e}")}));
        }
    };

    let creds = Credentials::new(cfg.user.clone(), cfg.password.clone());
    let transport = match AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host) {
        Ok(b) => b.credentials(creds).port(cfg.port).build(),
        Err(e) => {
            app_log!(error, "SMTP transport init failed: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": format!("{e}")}));
        }
    };

    match transport.send(email).await {
        Ok(_) => {
            app_log!(info, to = %body.to, subject = %body.subject, "Email sent via api0");
            HttpResponse::Ok().json(serde_json::json!({"success": true}))
        }
        Err(e) => {
            app_log!(error, to = %body.to, "Email send failed: {}", e);
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": format!("{e}")}))
        }
    }
}

// ── GET /api/admin/smtp-config ────────────────────────────────────────────────

#[derive(Serialize)]
struct SmtpConfigResponse {
    success:      bool,
    smtp_host:    Option<String>,
    smtp_port:    Option<u16>,
    smtp_user:    Option<String>,
    email_from:   Option<String>,
    has_password: bool,
}

pub async fn get_smtp_config_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": format!("{e}")}))
        }
    };

    let rows = client
        .query("SELECT key, value FROM system_config WHERE key LIKE 'email.%'", &[])
        .await
        .unwrap_or_default();

    let mut map = std::collections::HashMap::new();
    for row in &rows {
        let k: &str = row.get(0);
        let v: &str = row.get(1);
        map.insert(k, v.to_string());
    }

    HttpResponse::Ok().json(SmtpConfigResponse {
        success:      true,
        smtp_host:    map.get("email.smtp_host").cloned(),
        smtp_port:    map.get("email.smtp_port").and_then(|v| v.parse().ok()),
        smtp_user:    map.get("email.smtp_user").cloned(),
        email_from:   map.get("email.from_addr").cloned(),
        has_password: map.contains_key("email.smtp_password"),
    })
}

// ── PUT /api/admin/smtp-config ────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct UpdateSmtpConfigRequest {
    pub smtp_host:     Option<String>,
    pub smtp_port:     Option<u16>,
    pub smtp_user:     Option<String>,
    pub smtp_password: Option<String>,
    pub email_from:    Option<String>,
}

pub async fn update_smtp_config_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<UpdateSmtpConfigRequest>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let mut saved = Vec::new();
    if let Some(v) = &body.smtp_host {
        if let Err(e) = save_config_key(&store, "email.smtp_host", v).await {
            app_log!(error, "Failed to save email.smtp_host: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": format!("{e}")}));
        }
        saved.push("smtp_host");
    }
    if let Some(v) = body.smtp_port {
        if let Err(e) = save_config_key(&store, "email.smtp_port", &v.to_string()).await {
            app_log!(error, "Failed to save email.smtp_port: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": format!("{e}")}));
        }
        saved.push("smtp_port");
    }
    if let Some(v) = &body.smtp_user {
        if let Err(e) = save_config_key(&store, "email.smtp_user", v).await {
            app_log!(error, "Failed to save email.smtp_user: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": format!("{e}")}));
        }
        saved.push("smtp_user");
    }
    if let Some(v) = &body.smtp_password {
        if let Err(e) = save_config_key(&store, "email.smtp_password", v).await {
            app_log!(error, "Failed to save email.smtp_password: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": format!("{e}")}));
        }
        saved.push("smtp_password");
    }
    if let Some(v) = &body.email_from {
        if let Err(e) = save_config_key(&store, "email.from_addr", v).await {
            app_log!(error, "Failed to save email.from_addr: {}", e);
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": format!("{e}")}));
        }
        saved.push("email_from");
    }

    app_log!(info, saved = ?saved, "Admin updated SMTP config");
    HttpResponse::Ok().json(serde_json::json!({"success": true, "updated": saved}))
}
