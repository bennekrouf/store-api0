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
//   POST /api/internal/email/unsubscribe | resubscribe   (see unsubscribe.rs)
//   GET  /api/admin/smtp-config
//   PUT  /api/admin/smtp-config

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use lettre::message::header::{ContentType, HeaderName, HeaderValue};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub mod unsubscribe;

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

/// One URL path segment, percent-encoded: a client id is chosen by the
/// workspace, so it is not trusted to be URL- or HTML-safe as written.
fn path_segment(raw: &str) -> String {
    raw.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{:02X}", b),
        })
        .collect()
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
    /// Someone was added to, or invited into, a workspace. Every field but
    /// `has_account` is user-supplied text and is escaped when rendered.
    /// `link_ref` names the workspace's Get started page (/link/<link_ref>).
    WorkspaceInvite { workspace: String, role: String, invited_by: String, has_account: bool, link_ref: String },
    // ── Tier 3 — engagement ──────────────────────────────────────────────────
    Nudge { name: String, credits: i64 },
    WinBack { name: String },
    WhatsNew { feature_title: String, description: String },
    // ── Desktop app licences (sold on mayorana.ch, not api0) ─────────────────
    /// `how`: where the key goes in that app, as HTML ("open GitAgent, click …").
    LicenseIssued { product_name: String, how: String, key: String, updates_until: String },
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
            Self::WorkspaceInvite { .. }   => "workspace_invite",
            Self::Nudge { .. }             => "nudge",
            Self::WinBack { .. }           => "win_back",
            Self::WhatsNew { .. }          => "whats_new",
            Self::LicenseIssued { .. }     => "license_issued",
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
            Self::FirstCallMilestone { .. }                  => "Your first tool call — you're live!".into(),
            Self::MonthlyDigest { month, .. }                => format!("Your api0 usage summary — {}", month),
            Self::ProviderConnected { provider }             => format!("{} connected to api0", provider),
            Self::WorkspaceInvite { workspace, has_account, .. } => {
                if *has_account { format!("You've been added to {}", workspace) }
                else            { format!("You're invited to join {}", workspace) }
            }
            Self::Nudge { credits, .. }                      => if *credits > 0 { format!("You have {credits} credits waiting — try api0 today") } else { "Your api0 API key is ready to use".into() },
            Self::WinBack { .. }                             => "We miss you — here's what's new on api0".into(),
            Self::WhatsNew { feature_title, .. }             => format!("New on api0: {}", feature_title),
            Self::LicenseIssued { product_name, .. }         => format!("Your {} licence key", product_name),
        }
    }

    /// Emails a user can opt out of (see unsubscribe.rs). Everything else is
    /// about their account — receipts, keys, invites, licences — and always sent.
    pub fn is_optional(&self) -> bool {
        matches!(
            self,
            Self::MonthlyDigest { .. } | Self::Nudge { .. } | Self::WinBack { .. } | Self::WhatsNew { .. }
        )
    }

    /// The name the email is sent as. Licence emails go to people who bought a
    /// desktop app from mayorana.ch and have never heard of api0.
    fn sender_name(&self) -> &'static str {
        match self {
            Self::LicenseIssued { .. } => "mayorana",
            _ => "api0",
        }
    }

    #[cfg(test)]
    pub fn html_body(&self) -> String {
        self.html_body_with(None)
    }

    /// The body, with an unsubscribe link in the footer when one is given.
    pub fn html_body_with(&self, unsubscribe: Option<&str>) -> String {
        if let Self::LicenseIssued { product_name, how, key, updates_until } = self {
            return wrap_mayorana_layout(&format!(
                r#"<h1>Thank you for buying {product_name}</h1>
<p>Here is your licence key. To activate it, {how}</p>
<pre style="white-space:pre-wrap;word-break:break-all;background:#F1F5F9;padding:12px;border-radius:6px;font-size:12px">{key}</pre>
<p>It includes every update released until <strong>{updates_until}</strong>. Versions released before that date keep working after it.</p>
<p style="color:#64748B;font-size:13px">Keep this email: the key works offline and is all you need to activate {product_name} on another computer.</p>"#
            ));
        }
        let content = match self {
            // ── Tier 1 ───────────────────────────────────────────────────────
            Self::Welcome { name, key_prefix, credits } => format!(
                r#"<h1>Welcome to api0, {name}!</h1>
<p>Your account is live. api0 is an MCP gateway — it exposes your imported APIs as tools that AI assistants (Claude, Cursor, and others) can use on your behalf.</p>
<table style="border-collapse:collapse;margin:16px 0;background:#F8FAFC;border-radius:6px;overflow:hidden">
  <tr><td style="padding:8px 16px;font-weight:bold;color:#475569">Your API key</td><td style="padding:8px 16px;font-family:monospace;color:#6366F1">{key_prefix}…</td></tr>
  <tr><td style="padding:8px 16px;font-weight:bold;color:#475569">Starting credits</td><td style="padding:8px 16px">{credits}</td></tr>
</table>
<h2>Connect in 2 steps</h2>
<ol style="padding-left:20px">
  <li>Copy your API key from the dashboard</li>
  <li>Paste it into your MCP client (Claude Desktop or any MCP-compatible client) using the server URL <code style="background:#F1F5F9;padding:2px 4px;border-radius:3px">https://gateway.api0.ai/mcp</code></li>
</ol>
<p>Once connected, your AI assistant can discover and call any API you import — no manual HTTP requests needed.</p>
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
                r#"<h1>First tool call — you're live! 🎉</h1>
<p>Your MCP integration just made its first successful call to <code style="background:#F1F5F9;padding:2px 6px;border-radius:4px">{endpoint}</code>.</p>
<p>Your AI assistant can now discover and use all the APIs you import through api0. Here's what to do next:</p>
<ul>
  <li>Import more APIs as MCP tools in the dashboard</li>
  <li>Check your usage logs to monitor which tools are being called</li>
  <li>Top up credits to keep your tools running uninterrupted</li>
</ul>
<p><a href="https://app.api0.ai" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">View Dashboard</a></p>"#
            ),

            Self::MonthlyDigest { month, total_calls, credits_spent, top_endpoints } => {
                let endpoint_list = if top_endpoints.is_empty() {
                    "<li style=\"color:#94A3B8\">No tools called this month</li>".to_string()
                } else {
                    top_endpoints.iter()
                        .map(|e| format!("<li><code style=\"background:#F1F5F9;padding:2px 4px;border-radius:3px;font-size:12px\">{e}</code></li>"))
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                format!(
                    r#"<h1>Your api0 Summary — {month}</h1>
<table style="border-collapse:collapse;margin:16px 0">
  <tr><td style="padding:4px 12px;font-weight:bold">Tool calls made</td><td style="padding:4px 12px">{total_calls}</td></tr>
  <tr><td style="padding:4px 12px;font-weight:bold">Credits spent</td><td style="padding:4px 12px">{credits_spent}</td></tr>
</table>
<h2>Most-used tools</h2>
<ul style="padding-left:20px">{endpoint_list}</ul>
<p><a href="https://app.api0.ai/?view=stats" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">Full Stats</a></p>"#
                )
            }

            Self::ProviderConnected { provider } => format!(
                r#"<h1>{provider} Connected</h1>
<p>Your <strong>{provider}</strong> account is now linked to api0.</p>
<p>APIs authenticated via {provider} are now available as MCP tools — your AI assistant can call them on your behalf.</p>
<p><a href="https://app.api0.ai" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">View Dashboard</a></p>"#
            ),

            Self::WorkspaceInvite { workspace, role, invited_by, has_account, link_ref } => {
                // The button leads to the workspace's own Get started page —
                // Claude, Telegram, WhatsApp — not to the api0 dashboard, which
                // only the people running the workspace need.
                let page = format!("https://app.api0.ai/link/{}", path_segment(link_ref));
                let (workspace, role, invited_by) =
                    (escape_html(workspace), escape_html(role), escape_html(invited_by));
                let sign_in = if *has_account {
                    ""
                } else {
                    "<p>Sign in with Google using <strong>this email address</strong> — that is what the invitation is attached to.</p>"
                };
                let manage = if role == "owner" || role == "admin" {
                    "<p style=\"color:#64748B;font-size:13px\">To set the workspace up — tools, bots, members — use the <a href=\"https://app.api0.ai\" style=\"color:#6366F1\">dashboard</a>.</p>"
                } else {
                    ""
                };
                format!(
                    r#"<h1>Join {workspace}</h1>
<p><strong>{invited_by}</strong> gave you the <strong>{role}</strong> role in <strong>{workspace}</strong>.</p>
<p>The page below shows how to use it from Claude, and from the team's Telegram or WhatsApp bot if it has one.</p>
{sign_in}
<p><a href="{page}" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">Get started with {workspace}</a></p>
{manage}
<p style="color:#64748B;font-size:13px">Not expecting this? You can ignore it — nothing happens unless you sign in.</p>"#
                )
            }

            // ── Tier 3 ───────────────────────────────────────────────────────
            Self::Nudge { name, credits } => {
                let credits_line = if *credits > 0 {
                    format!("<p>You have <strong>{credits} credits</strong> ready — enough for thousands of tool calls.</p>")
                } else {
                    "<p>Connect your MCP client and your AI assistant will start using your tools immediately.</p>".to_string()
                };
                format!(
                    r#"<h1>Your MCP tools are ready, {name}</h1>
<p>You signed up for api0 but haven't connected an MCP client yet.</p>
{credits_line}
<h2>Connect in 2 steps:</h2>
<ol style="padding-left:20px">
  <li>Copy your API key from the dashboard</li>
  <li>Add it to your MCP client (Claude Desktop or any MCP-compatible client) pointing to <code style="background:#1E293B;padding:2px 6px;border-radius:3px">https://gateway.api0.ai/mcp</code></li>
</ol>
<p>Your AI assistant will then discover and call your imported APIs automatically.</p>
<p><a href="https://app.api0.ai" style="display:inline-block;padding:10px 20px;background:#6366F1;color:white;text-decoration:none;border-radius:6px">Open Dashboard</a></p>"#
                )
            },

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
            Self::LicenseIssued { .. } => unreachable!("rendered above"),
        };

        wrap_layout(&content, unsubscribe)
    }
}

/// Text a user typed, made safe to put inside an HTML email.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The sender's postal address, which CAN-SPAM requires on commercial email.
const POSTAL_ADDRESS: &str = "Mayorana, Saint-Prex, Switzerland";

fn wrap_layout(content: &str, unsubscribe: Option<&str>) -> String {
    let unsubscribe = unsubscribe
        .map(|url| format!(
            r#"<br><a href="{}" style="color:#64748B">Unsubscribe from these emails</a> — account emails such as receipts and key changes are still sent."#,
            escape_html(url)
        ))
        .unwrap_or_default();
    format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head>
<body style="margin:0;padding:0;background:#F8FAFC;font-family:Arial,Helvetica,sans-serif">
<div style="max-width:600px;margin:24px auto;background:#fff;border-radius:8px;overflow:hidden;box-shadow:0 1px 3px rgba(0,0,0,0.1)">
  <div style="background:#0F172A;padding:20px 32px">
    <span style="color:white;font-size:20px;font-weight:bold">api0</span>
    <span style="color:#64748B;font-size:13px;margin-left:8px">MCP gateway</span>
  </div>
  <div style="padding:32px;color:#1E293B;line-height:1.6">{content}</div>
  <div style="padding:16px 32px;background:#F8FAFC;color:#64748B;font-size:12px;text-align:center">
    api0 — MCP gateway ·
    <a href="https://app.api0.ai" style="color:#6366F1">app.api0.ai</a>
    <br>{POSTAL_ADDRESS}{unsubscribe}
  </div>
</div>
</body>
</html>"#
    )
}

fn wrap_mayorana_layout(content: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html>
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"></head>
<body style="margin:0;padding:0;background:#F8FAFC;font-family:Arial,Helvetica,sans-serif">
<div style="max-width:600px;margin:24px auto;background:#fff;border-radius:8px;overflow:hidden;box-shadow:0 1px 3px rgba(0,0,0,0.1)">
  <div style="padding:32px;color:#1E293B;line-height:1.6">{content}</div>
  <div style="padding:16px 32px;background:#F8FAFC;color:#64748B;font-size:12px;text-align:center">
    <a href="https://mayorana.ch" style="color:#475569">mayorana.ch</a>
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
    // Optional emails go only to people who have not opted out, and always
    // carry the way to opt out. Checked here so no sender can skip it.
    let unsubscribe = if kind.is_optional() {
        if unsubscribe::is_opted_out(store, to).await {
            app_log!(info, kind = %kind.name(), "Skipped optional email: recipient opted out");
            return Ok(());
        }
        Some(unsubscribe::link(to).ok_or_else(|| {
            anyhow::anyhow!("no unsubscribe link (API0_ENCRYPTION_KEY unset); optional email not sent")
        })?)
    } else {
        None
    };

    let cfg = load_smtp_config(store).await
        .ok_or_else(|| anyhow::anyhow!("SMTP not configured"))?;

    let mut builder = lettre::Message::builder()
        .from(format!("{} <{}>", kind.sender_name(), cfg.from_addr).parse()?)
        .to(to.parse()?)
        .subject(kind.subject())
        .header(ContentType::TEXT_HTML);
    if let Some(url) = &unsubscribe {
        builder = with_list_unsubscribe(builder, url);
    }
    let email = builder.body(kind.html_body_with(unsubscribe.as_deref()))?;

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

/// RFC 2369 and RFC 8058: mail clients show their own unsubscribe
/// button, which POSTs to the URL without opening a page.
fn with_list_unsubscribe(builder: lettre::message::MessageBuilder, url: &str) -> lettre::message::MessageBuilder {
    builder
        .raw_header(HeaderValue::new(HeaderName::new_from_ascii_str("List-Unsubscribe"), format!("<{url}>")))
        .raw_header(HeaderValue::new(
            HeaderName::new_from_ascii_str("List-Unsubscribe-Post"),
            "List-Unsubscribe=One-Click".to_string(),
        ))
}

/// A caller-supplied unsubscribe URL goes into a header verbatim, so it must be
/// a plain https URL: no whitespace, control characters or angle brackets.
fn is_valid_unsubscribe_url(url: &str) -> bool {
    url.starts_with("https://")
        && url.len() <= 2000
        && url.chars().all(|c| c.is_ascii_graphic() && c != '<' && c != '>')
}

// ── POST /api/internal/email/send ─────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SendEmailRequest {
    pub to:        String,
    pub subject:   String,
    pub html_body: String,
    /// Optional one-click unsubscribe URL, sent as List-Unsubscribe headers.
    #[serde(default)]
    pub list_unsubscribe: Option<String>,
    /// Optional display name for the From header (defaults to "api0"), so a
    /// product sending through api0 appears under its own name.
    #[serde(default)]
    pub from_name: Option<String>,
}

/// A display name for the From header: short, printable, single-line.
fn is_valid_from_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty() && name.chars().count() <= 64 && !name.chars().any(|c| c.is_control())
}

pub async fn send_email_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SendEmailRequest>,
) -> impl Responder {
    // The internal secret, or a service key with `email.send`.
    if let Err(deny) = crate::middleware::service_key::require_scope(
        &req,
        crate::middleware::service_key::Scope::EmailSend,
    ) {
        return deny;
    }
    let cfg = match load_smtp_config(&store).await {
        Some(c) => c,
        None => return HttpResponse::ServiceUnavailable()
            .json(serde_json::json!({"success":false,"error":"SMTP not configured"})),
    };

    if let Some(url) = &body.list_unsubscribe {
        if !is_valid_unsubscribe_url(url) {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"success":false,"error":"list_unsubscribe must be an https URL"}));
        }
    }

    let from_name = match &body.from_name {
        Some(name) if !is_valid_from_name(name) => {
            return HttpResponse::BadRequest()
                .json(serde_json::json!({"success":false,"error":"from_name must be 1-64 printable characters"}));
        }
        Some(name) => name.trim().to_string(),
        None => "api0".to_string(),
    };
    let from_addr: lettre::Address = match cfg.from_addr.parse() {
        Ok(a) => a,
        Err(e) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success":false,"error":format!("invalid SMTP from address: {e}")})),
    };

    let mut builder = Message::builder()
        // Mailbox::new quotes/encodes the display name, so commas or accents can't break the header.
        .from(lettre::message::Mailbox::new(Some(from_name), from_addr))
        .to(body.to.parse().unwrap())
        .subject(&body.subject)
        .header(ContentType::TEXT_HTML);
    if let Some(url) = &body.list_unsubscribe {
        builder = with_list_unsubscribe(builder, url);
    }
    let email = match builder.body(body.html_body.clone()) {
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
        .query(
            "SELECT email FROM user_preferences up
              WHERE email IS NOT NULL AND email != ''
                AND NOT EXISTS (SELECT 1 FROM email_opt_outs o WHERE o.email = lower(up.email))",
            &[],
        )
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

#[cfg(test)]
mod invite_tests {
    use super::*;

    fn invite(role: &str, link_ref: &str) -> EmailKind {
        EmailKind::WorkspaceInvite {
            workspace: "Oryx <DevOps>".into(),
            role: role.into(),
            invited_by: "admin@example.com".into(),
            has_account: false,
            link_ref: link_ref.into(),
        }
    }

    #[test]
    fn the_invitation_leads_to_the_workspace_page_not_the_dashboard() {
        let html = invite("member", "oryx-devops").html_body();
        assert!(html.contains(r#"href="https://app.api0.ai/link/oryx-devops""#));
        assert!(html.contains("Get started with Oryx &lt;DevOps&gt;"));
        assert!(!html.contains("use the <a href=\"https://app.api0.ai\""), "members get no dashboard pointer");
        assert!(!invite("member", "x").subject().contains("api0"));
    }

    #[test]
    fn owners_are_also_pointed_at_the_dashboard() {
        assert!(invite("owner", "x").html_body().contains(">dashboard</a>"));
    }

    #[test]
    fn a_hostile_client_id_cannot_break_out_of_the_link() {
        let html = invite("member", "a\"><script>/b c").html_body();
        assert!(html.contains("/link/a%22%3E%3Cscript%3E%2Fb%20c\""));
        assert!(!html.contains("<script>"));
    }
}

#[cfg(test)]
mod send_request_tests {
    use super::*;

    #[test]
    fn only_plain_https_urls_are_accepted_as_unsubscribe_headers() {
        assert!(is_valid_unsubscribe_url("https://api.cvenom.com/email/unsubscribe?token=a.b-c_d"));
        assert!(!is_valid_unsubscribe_url("http://api.cvenom.com/u"));
        assert!(!is_valid_unsubscribe_url("https://x.com/u>\r\nBcc: victim@example.com"));
        assert!(!is_valid_unsubscribe_url("https://x.com/a b"));
    }

    #[test]
    fn from_names_must_be_short_single_line_text() {
        assert!(is_valid_from_name("CVenom"));
        assert!(is_valid_from_name("Café, Inc."));
        assert!(!is_valid_from_name("   "));
        assert!(!is_valid_from_name("CVenom\r\nBcc: x@y.z"));
        assert!(!is_valid_from_name(&"x".repeat(65)));
    }

    #[test]
    fn a_from_name_with_a_comma_stays_one_mailbox() {
        let from = lettre::message::Mailbox::new(Some("Café, Inc.".into()), "no-reply@api0.ai".parse().unwrap());
        let msg = Message::builder().from(from).to("d@e.f".parse().unwrap()).subject("s")
            .body(String::from("h")).unwrap();
        let raw = String::from_utf8(msg.formatted()).unwrap();
        let from_line = raw.lines().find(|l| l.starts_with("From:")).unwrap();
        assert!(from_line.ends_with("<no-reply@api0.ai>"), "{from_line}");
        assert!(!from_line.contains("Café, Inc. <"), "display name must be quoted or encoded: {from_line}");
    }

    #[test]
    fn the_request_field_is_optional() {
        let r: SendEmailRequest = serde_json::from_str(r#"{"to":"a@b.c","subject":"s","html_body":"h"}"#).unwrap();
        assert!(r.list_unsubscribe.is_none());
        assert!(r.from_name.is_none());
        let r: SendEmailRequest = serde_json::from_str(
            r#"{"to":"a@b.c","subject":"s","html_body":"h","list_unsubscribe":null}"#).unwrap();
        assert!(r.list_unsubscribe.is_none());
    }

    #[test]
    fn the_headers_are_set_on_the_message() {
        let msg = with_list_unsubscribe(
            Message::builder().from("a@b.c".parse().unwrap()).to("d@e.f".parse().unwrap()).subject("s"),
            "https://x.com/u?token=t",
        )
        .body(String::from("h"))
        .unwrap();
        let raw = String::from_utf8(msg.formatted()).unwrap();
        assert!(raw.contains("List-Unsubscribe: <https://x.com/u?token=t>"));
        assert!(raw.contains("List-Unsubscribe-Post: List-Unsubscribe=One-Click"));
    }
}
