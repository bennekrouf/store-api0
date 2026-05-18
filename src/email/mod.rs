// src/email/mod.rs
//
// Centralised email system for api0.
// SMTP config lives in system_config (email.smtp_*) or falls back to SMTP_* env vars.
//
// Public surface:
//   send_async(store, to, kind)   — fire-and-forget, call from any handler
//   EmailKind                     — all email variants (Tier 1-3)
//
// Internal endpoints (X-Internal-Secret):
//   POST /api/internal/email/send
//   GET  /api/admin/smtp-config
//   PUT  /api/admin/smtp-config

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

// ── EmailKind ─────────────────────────────────────────────────────────────────

pub enum EmailKind {
    // ── Tier 1 — transactional ───────────────────────────────────────────────
    Welcome { name: String, key_prefix: String, credits: i64 },
    PaymentReceipt { amount_dollars: f64, credits_added: i64, new_balance: i64 },
    LowCredits { balance: i64 },
    KeyCreated { key_prefix: String, key_name: String },
    KeyRevoked { key_prefix: String },
    AccountDeleted,
    // ── Tier 2 — informational ───────────────────────────────────────────────
    CreditAdjustment { amount: i64, reason: String, new_balance: i64 },
    FirstCallMilestone { endpoint: String },
    MonthlyDigest { month: String, total_calls: i64, credits_spent: i64, top_endpoints: Vec<String> },
    ProviderConnected { provider: String },
    // ── Tier 3 — engagement ──────────────────────────────────────────────────
    Nudge { name: String, credits: i64 },
    WinBack { name: String },
    WhatsNew { feature_title: String, description: String },
}

impl EmailKind {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Welcome { .. }           => "welcome",
            Self::PaymentReceipt { .. }    => "payment_receipt",
            Self::LowCredits { .. }        => "low_credits",
            Self::KeyCreated { .. }        => "key_created",
            Self::KeyRevoked { .. }        => "key_revoked",
            Self::AccountDeleted           => "account_deleted",
            Self::CreditAdjustment { .. }  => "credit_adjustment",
            Self::FirstCallMilestone { .. }=> "first_call_milestone",
            Self::MonthlyDigest { .. }     => "monthly_digest",
            Self::ProviderConnected { .. } => "provider_connected",
            Self::Nudge { .. }             => "nudge",
            Self::WinBack { .. }           => "win_back",
            Self::WhatsNew { .. }          => "whats_new",
        }
    }

    pub fn subject(&self) -> String {
        match self {
            Self::Welcome { .. }                              => "Welcome to api0! 🎉".into(),
            Self::PaymentReceipt { .. }                      => "api0 — Payment Confirmed".into(),
            Self::LowCredits { balance }                     => format!("Low balance: {} credits remaining", balance),
            Self::KeyCreated { key_name, .. }                => format!("New API key created: {}", key_name),
            Self::KeyRevoked { key_prefix }                  => format!("API key {} revoked", key_prefix),
            Self::AccountDeleted                             => "Your api0 account has been deleted".into(),
            Self::CreditAdjustment { amount, .. }            => {
                if *amount >= 0 { format!("You received {} credits", amount) }
                else            { format!("Credit adjustment: {} credits", amount) }
            }
            Self::FirstCallMilestone { .. }                  => "Your first API call — milestone reached!".into(),
            Self::MonthlyDigest { month, .. }                => format!("Your api0 usage summary — {}", month),
            Self::ProviderConnected { provider }             => format!("{} connected to api0", provider),
            Self::Nudge { .. }                               => "You have credits waiting — try api0 today".into(),
            Self::WinBack { .. }                             => "We miss you — here's what's new on api0".into(),
            Self::WhatsNew { feature_title, .. }             => format!("New on api0: {}", feature_title),
        }
    }

    pub fn html_body(&self) -> String {
        let content = match self {
            // ── Tier 1 ───────────────────────────────────────────────────────
            Self::Welcome { name, key_prefix, credits } => format!(
                r#"<h1>Welcome to api0, {name}!</h1>
<p>Your account is live. Here's what you need to get started.</p>
<table style="border-collapse:collapse;margin:16px 0;background:#F8FAFC;border-radius:6px;overflow:hidden">
  <tr><td style="padding:8px 16px;font-weight:bold;color:#475569">Your first API key</td><td style="padding:8px 16px;font-family:monospace;color:#6366F1">{key_prefix}…</td></tr>
  <tr><td style="padding:8px 16px;font-weight:bold;color:#475569">Starting credits</td><td style="padding:8px 16px">{credits}</td></tr>
</table>
<h2>Quick start</h2>
<pre style="background:#0F172A;color:#E2E8F0;padding:16px;border-radius:6px;overflow-x:auto;font-size:13px">curl https://gateway.api0.ai/api/sentence \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -d '{{"sentence":"hello world"}}'</pre>
<p><a href="https://app.api0.ai" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">Open Dashboard</a></p>"#
            ),

            Self::PaymentReceipt { amount_dollars, credits_added, new_balance } => format!(
                r#"<h1>Payment Confirmed</h1>
<p>Thank you — your credits have been added.</p>
<table style="border-collapse:collapse;margin:16px 0">
  <tr><td style="padding:4px 12px;font-weight:bold">Amount charged</td><td style="padding:4px 12px">${amount_dollars:.2}</td></tr>
  <tr><td style="padding:4px 12px;font-weight:bold">Credits added</td><td style="padding:4px 12px">{credits_added}</td></tr>
  <tr><td style="padding:4px 12px;font-weight:bold">New balance</td><td style="padding:4px 12px">{new_balance}</td></tr>
</table>
<p><a href="https://app.api0.ai/?view=settings" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">View Balance</a></p>"#
            ),

            Self::LowCredits { balance } => format!(
                r#"<h1>Low Credit Balance</h1>
<p>Your api0 balance has dropped to <strong>{balance} credits</strong>.</p>
<p>Top up now to keep your integrations running without interruption.</p>
<p><a href="https://app.api0.ai/?view=settings" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">Buy Credits</a></p>"#
            ),

            Self::KeyCreated { key_prefix, key_name } => format!(
                r#"<h1>New API Key Created</h1>
<p>A new API key has been generated for your account.</p>
<table style="border-collapse:collapse;margin:16px 0;background:#F8FAFC;border-radius:6px;overflow:hidden">
  <tr><td style="padding:8px 16px;font-weight:bold;color:#475569">Name</td><td style="padding:8px 16px">{key_name}</td></tr>
  <tr><td style="padding:8px 16px;font-weight:bold;color:#475569">Prefix</td><td style="padding:8px 16px;font-family:monospace;color:#6366F1">{key_prefix}…</td></tr>
</table>
<p style="color:#64748B;font-size:13px">If you didn't create this key, revoke it immediately from the dashboard.</p>
<p><a href="https://app.api0.ai" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">Manage Keys</a></p>"#
            ),

            Self::KeyRevoked { key_prefix } => format!(
                r#"<h1>API Key Revoked</h1>
<p>The key <code style="background:#F1F5F9;padding:2px 6px;border-radius:4px">{key_prefix}…</code> has been permanently revoked.</p>
<p>Any applications still using this key will receive 401 errors.</p>
<p>If you didn't revoke this key, contact support immediately.</p>
<p><a href="https://app.api0.ai" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">Manage Keys</a></p>"#
            ),

            Self::AccountDeleted => r#"<h1>Account Deleted</h1>
<p>Your api0 account and all associated data have been permanently removed.</p>
<p>This includes your API keys, usage logs, and credit balance.</p>
<p>If this was a mistake, you can sign up again at any time — but your previous data cannot be recovered.</p>"#.into(),

            // ── Tier 2 ───────────────────────────────────────────────────────
            Self::CreditAdjustment { amount, reason, new_balance } => {
                let (verb, abs) = if *amount >= 0 { ("added to", *amount) } else { ("removed from", -amount) };
                format!(
                    r#"<h1>Credit Adjustment</h1>
<p><strong>{abs} credits</strong> have been {verb} your account.</p>
<table style="border-collapse:collapse;margin:16px 0">
  <tr><td style="padding:4px 12px;font-weight:bold">Reason</td><td style="padding:4px 12px">{reason}</td></tr>
  <tr><td style="padding:4px 12px;font-weight:bold">New balance</td><td style="padding:4px 12px">{new_balance} credits</td></tr>
</table>"#
                )
            }

            Self::FirstCallMilestone { endpoint } => format!(
                r#"<h1>First API Call — You're Live! 🎉</h1>
<p>You just made your first successful API call to <code style="background:#F1F5F9;padding:2px 6px;border-radius:4px">{endpoint}</code>.</p>
<p>Your integration is working. Here's what you can do next:</p>
<ul>
  <li>Explore other endpoints in the dashboard</li>
  <li>Check your usage logs to monitor traffic</li>
  <li>Set up alerts for high usage or low credits</li>
</ul>
<p><a href="https://app.api0.ai" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">View Dashboard</a></p>"#
            ),

            Self::MonthlyDigest { month, total_calls, credits_spent, top_endpoints } => {
                let endpoint_list = if top_endpoints.is_empty() {
                    "<li style=\"color:#94A3B8\">No endpoints used this month</li>".to_string()
                } else {
                    top_endpoints.iter()
                        .map(|e| format!("<li><code style=\"background:#F1F5F9;padding:2px 4px;border-radius:3px;font-size:12px\">{e}</code></li>"))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                format!(
                    r#"<h1>Your api0 Summary — {month}</h1>
<table style="border-collapse:collapse;margin:16px 0">
  <tr><td style="padding:4px 12px;font-weight:bold">Total API calls</td><td style="padding:4px 12px">{total_calls}</td></tr>
  <tr><td style="padding:4px 12px;font-weight:bold">Credits spent</td><td style="padding:4px 12px">{credits_spent}</td></tr>
</table>
<h2>Top endpoints</h2>
<ul style="padding-left:20px">{endpoint_list}</ul>
<p><a href="https://app.api0.ai/?view=stats" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">Full Stats</a></p>"#
                )
            }

            Self::ProviderConnected { provider } => format!(
                r#"<h1>{provider} Connected</h1>
<p>Your <strong>{provider}</strong> account is now linked to api0.</p>
<p>Your API keys can now access {provider}-powered endpoints.</p>
<p><a href="https://app.api0.ai" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">View Dashboard</a></p>"#
            ),

            // ── Tier 3 ───────────────────────────────────────────────────────
            Self::Nudge { name, credits } => format!(
                r#"<h1>Your API keys are ready, {name}</h1>
<p>You signed up for api0 but haven't made your first API call yet.</p>
<p>You have <strong>{credits} credits</strong> waiting — enough for thousands of requests.</p>
<h2>Make your first call in 30 seconds:</h2>
<pre style="background:#0F172A;color:#E2E8F0;padding:16px;border-radius:6px;overflow-x:auto;font-size:13px">curl https://gateway.api0.ai/api/sentence \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -d '{{"sentence":"hello api0"}}'</pre>
<p><a href="https://app.api0.ai" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">Get Your API Key</a></p>"#
            ),

            Self::WinBack { name } => format!(
                r#"<h1>We miss you, {name}</h1>
<p>It's been a while since your last API call. Your account and credits are still here.</p>
<h2>What's new:</h2>
<ul>
  <li>Improved gateway performance</li>
  <li>New provider integrations</li>
  <li>Better usage analytics</li>
  <li>MCP tool registry for AI agents</li>
</ul>
<p><a href="https://app.api0.ai" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">Come Back to api0</a></p>"#
            ),

            Self::WhatsNew { feature_title, description } => format!(
                r#"<h1>New on api0: {feature_title}</h1>
<p>{description}</p>
<p><a href="https://app.api0.ai" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">Try It Now</a></p>"#
            ),
        };

        wrap_layout(&content)
    }
}

fn wrap_layout(content: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head>
<body style="margin:0;padding:0;background:#F8FAFC;font-family:Arial,Helvetica,sans-serif">
<div style="max-width:600px;margin:24px auto;background:#fff;border-radius:8px;overflow:hidden;box-shadow:0 1px 3px rgba(0,0,0,0.1)">
  <div style="background:#0F172A;padding:20px 32px">
    <span style="color:white;font-size:20px;font-weight:bold">api0</span>
    <span style="color:#64748B;font-size:13px;margin-left:8px">API Platform</span>
  </div>
  <div style="padding:32px;color:#1E293B;line-height:1.6">{content}</div>
  <div style="padding:16px 32px;background:#F8FAFC;color:#64748B;font-size:12px;text-align:center">
    api0 — Programmable API Gateway ·
    <a href="https://app.api0.ai" style="color:#6366F1">app.api0.ai</a>
  </div>
</div>
</body>
</html>"#
    )
}

// ── Fire-and-forget helper (call from any handler) ────────────────────────────

pub fn send_async(store: Arc<EndpointStore>, to: impl Into<String>, kind: EmailKind) {
    let to = to.into();
    tokio::spawn(async move {
        match deliver_internal(&store, &to, &kind).await {
            Ok(()) => app_log!(info, to = %to, kind = %kind.name(), "Email sent"),
            Err(e) => app_log!(error, to = %to, kind = %kind.name(), "Email failed: {}", e),
        }
    });
}

// ── Delivery ──────────────────────────────────────────────────────────────────

async fn deliver_internal(store: &EndpointStore, to: &str, kind: &EmailKind) -> anyhow::Result<()> {
    let cfg = load_smtp_config(store).await
        .ok_or_else(|| anyhow::anyhow!("SMTP not configured"))?;

    let email = lettre::Message::builder()
        .from(format!("api0 <{}>", cfg.from_addr).parse()?)
        .to(to.parse()?)
        .subject(kind.subject())
        .header(ContentType::TEXT_HTML)
        .body(kind.html_body())?;

    let creds = Credentials::new(cfg.user.clone(), cfg.password.clone());
    let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)?
        .credentials(creds)
        .port(cfg.port)
        .build();

    transport.send(email).await?;
    Ok(())
}

// ── SMTP config (DB + env fallback) ──────────────────────────────────────────

struct SmtpCfg { host: String, port: u16, user: String, password: String, from_addr: String }

async fn load_smtp_config(store: &EndpointStore) -> Option<SmtpCfg> {
    let client = store.get_admin_conn().await.ok()?;
    let rows = client
        .query("SELECT key, value FROM system_config WHERE key LIKE 'email.%'", &[])
        .await.ok()?;

    let mut map = std::collections::HashMap::new();
    for row in &rows {
        let k: &str = row.get(0);
        let v: &str = row.get(1);
        map.insert(k.to_string(), v.to_string());
    }

    let host     = map.get("email.smtp_host").cloned().or_else(|| std::env::var("SMTP_HOST").ok())?;
    let user     = map.get("email.smtp_user").cloned().or_else(|| std::env::var("SMTP_USER").ok())?;
    let password = map.get("email.smtp_password").cloned().or_else(|| std::env::var("SMTP_PASSWORD").ok())?;
    let port     = map.get("email.smtp_port").and_then(|v| v.parse().ok())
        .or_else(|| std::env::var("SMTP_PORT").ok().and_then(|v| v.parse().ok()))
        .unwrap_or(587);
    let from_addr = map.get("email.from_addr").cloned()
        .or_else(|| std::env::var("EMAIL_FROM").ok())
        .unwrap_or_else(|| user.clone());

    Some(SmtpCfg { host, port, user, password, from_addr })
}

async fn save_config_key(store: &EndpointStore, key: &str, value: &str) -> anyhow::Result<()> {
    let client = store.get_admin_conn().await?;
    client.execute(
        "INSERT INTO system_config (key, value, updated_at) VALUES ($1, $2, NOW())
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = NOW()",
        &[&key, &value],
    ).await?;
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
        return HttpResponse::Unauthorized().json(serde_json::json!({"success":false,"error":"Unauthorized"}));
    }
    let cfg = match load_smtp_config(&store).await {
        Some(c) => c,
        None => return HttpResponse::ServiceUnavailable()
            .json(serde_json::json!({"success":false,"error":"SMTP not configured"})),
    };

    let email = match Message::builder()
        .from(format!("api0 <{}>", cfg.from_addr).parse().unwrap())
        .to(body.to.parse().unwrap())
        .subject(&body.subject)
        .header(ContentType::TEXT_HTML)
        .body(body.html_body.clone())
    {
        Ok(m) => m,
        Err(e) => return HttpResponse::BadRequest().json(serde_json::json!({"success":false,"error":format!("{e}")})),
    };

    let transport = match AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host) {
        Ok(b) => b.credentials(Credentials::new(cfg.user, cfg.password)).port(cfg.port).build(),
        Err(e) => return HttpResponse::InternalServerError().json(serde_json::json!({"success":false,"error":format!("{e}")})),
    };

    match transport.send(email).await {
        Ok(_) => {
            app_log!(info, to = %body.to, "Email sent via api0");
            HttpResponse::Ok().json(serde_json::json!({"success":true}))
        }
        Err(e) => {
            app_log!(error, to = %body.to, "Email send failed: {}", e);
            HttpResponse::InternalServerError().json(serde_json::json!({"success":false,"error":format!("{e}")}))
        }
    }
}

// ── GET /api/admin/smtp-config ────────────────────────────────────────────────

#[derive(Serialize)]
struct SmtpConfigResponse {
    success: bool, smtp_host: Option<String>, smtp_port: Option<u16>,
    smtp_user: Option<String>, email_from: Option<String>, has_password: bool,
}

pub async fn get_smtp_config_handler(req: HttpRequest, store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized().json(serde_json::json!({"success":false,"error":"Unauthorized"}));
    }
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(serde_json::json!({"error":format!("{e}")})),
    };
    let rows = client.query("SELECT key, value FROM system_config WHERE key LIKE 'email.%'", &[])
        .await.unwrap_or_default();
    let mut map = std::collections::HashMap::new();
    for row in &rows { let k: &str = row.get(0); let v: &str = row.get(1); map.insert(k, v.to_string()); }
    HttpResponse::Ok().json(SmtpConfigResponse {
        success: true,
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
    pub smtp_host: Option<String>, pub smtp_port: Option<u16>,
    pub smtp_user: Option<String>, pub smtp_password: Option<String>, pub email_from: Option<String>,
}

pub async fn update_smtp_config_handler(
    req: HttpRequest, store: web::Data<Arc<EndpointStore>>,
    body: web::Json<UpdateSmtpConfigRequest>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized().json(serde_json::json!({"success":false,"error":"Unauthorized"}));
    }
    macro_rules! save {
        ($key:expr, $val:expr) => {
            if let Err(e) = save_config_key(&store, $key, $val).await {
                app_log!(error, "Failed to save {}: {}", $key, e);
                return HttpResponse::InternalServerError().json(serde_json::json!({"error":format!("{e}")}));
            }
        };
    }
    if let Some(v) = &body.smtp_host     { save!("email.smtp_host", v); }
    if let Some(v) = body.smtp_port      { save!("email.smtp_port", &v.to_string()); }
    if let Some(v) = &body.smtp_user     { save!("email.smtp_user", v); }
    if let Some(v) = &body.smtp_password { save!("email.smtp_password", v); }
    if let Some(v) = &body.email_from    { save!("email.from_addr", v); }
    app_log!(info, "Admin updated SMTP config");
    HttpResponse::Ok().json(serde_json::json!({"success":true}))
}

// ── POST /api/admin/broadcast/whats-new ──────────────────────────────────────

#[derive(Deserialize)]
pub struct WhatsNewRequest {
    pub feature_title: String,
    pub description:   String,
}

pub async fn broadcast_whats_new_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<WhatsNewRequest>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized().json(serde_json::json!({"success":false,"error":"Unauthorized"}));
    }
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => return HttpResponse::InternalServerError().json(serde_json::json!({"error":format!("{e}")})),
    };
    let rows = client
        .query("SELECT email FROM user_preferences WHERE email IS NOT NULL AND email != ''", &[])
        .await.unwrap_or_default();

    let count = rows.len();
    for row in rows {
        let email: &str = row.get(0);
        send_async(store.as_ref().clone(), email, EmailKind::WhatsNew {
            feature_title: body.feature_title.clone(),
            description:   body.description.clone(),
        });
    }
    app_log!(info, "[broadcast] WhatsNew sent to {} users: {}", count, body.feature_title);
    HttpResponse::Ok().json(serde_json::json!({"success":true,"sent_to":count}))
}
