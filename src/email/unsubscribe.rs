// src/email/unsubscribe.rs
//
// Opting out of the optional emails (EmailKind::is_optional): the monthly
// digest, the nudge, the win-back and the "what's new" broadcast. Account
// emails — welcome, receipts, keys, invites, licences — are not affected.
//
// Each optional email carries a link, and a List-Unsubscribe header, to
//   <gateway>/api/email/unsubscribe?e=<email>&t=<token>
// The token is HMAC-SHA256 over the address with a key derived for this one
// purpose (secret_box::derive_key), so only this store can mint one, and a link
// works only for the address it was sent to. Visiting the link shows a
// confirmation page, because mail scanners follow links on their own; the POST,
// from that page or from the mail client's one-click button, records the
// opt-out. The gateway serves those pages and forwards the address and token:
//
//   POST /api/internal/email/unsubscribe   { email, token }
//   POST /api/internal/email/resubscribe   { email, token }

use super::{check_internal_secret, path_segment};
use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::infra::secret_box;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use std::sync::Arc;

const PURPOSE: &str = "api0 email unsubscribe v1";

fn normalise(email: &str) -> String {
    email.trim().to_lowercase()
}

fn mac_for(email: &str) -> Option<Hmac<Sha256>> {
    let key = secret_box::derive_key(PURPOSE).ok()?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key).ok()?;
    mac.update(normalise(email).as_bytes());
    Some(mac)
}

/// The token for `email`, or None when no encryption key is configured.
pub fn token(email: &str) -> Option<String> {
    Some(hex::encode(mac_for(email)?.finalize().into_bytes()))
}

/// Whether `token` was minted for `email`. Compared in constant time.
pub fn verify(email: &str, token: &str) -> bool {
    let (Some(mac), Ok(given)) = (mac_for(email), hex::decode(token.trim())) else {
        return false;
    };
    mac.verify_slice(&given).is_ok()
}

/// The public unsubscribe URL for `email`, served by the gateway.
pub fn link(email: &str) -> Option<String> {
    let base = std::env::var("API0_PUBLIC_GATEWAY_URL")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| "https://gateway.api0.ai".to_string());
    Some(format!(
        "{}/api/email/unsubscribe?e={}&t={}",
        base.trim_end_matches('/'),
        path_segment(&normalise(email)),
        token(email)?
    ))
}

/// Whether `email` opted out of the optional emails. A lookup that fails
/// counts as opted out: skipping an optional email is the safe mistake.
pub async fn is_opted_out(store: &EndpointStore, email: &str) -> bool {
    let Ok(client) = store.get_admin_conn().await else { return true };
    match client
        .query_opt("SELECT 1 FROM email_opt_outs WHERE email = $1", &[&normalise(email)])
        .await
    {
        Ok(row) => row.is_some(),
        Err(e) => {
            app_log!(error, error = %e, "Could not read email opt-outs; skipping optional email");
            true
        }
    }
}

async fn set_opted_out(store: &EndpointStore, email: &str, opted_out: bool) -> anyhow::Result<()> {
    let client = store.get_admin_conn().await?;
    let email = normalise(email);
    if opted_out {
        client
            .execute(
                "INSERT INTO email_opt_outs (email) VALUES ($1) ON CONFLICT (email) DO NOTHING",
                &[&email],
            )
            .await?;
    } else {
        client.execute("DELETE FROM email_opt_outs WHERE email = $1", &[&email]).await?;
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct OptOutRequest {
    pub email: String,
    pub token: String,
}

async fn change(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<OptOutRequest>,
    opted_out: bool,
) -> HttpResponse {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized().json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }
    if !verify(&body.email, &body.token) {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"success": false, "error": "This link is not valid"}));
    }
    match set_opted_out(&store, &body.email, opted_out).await {
        Ok(()) => {
            app_log!(info, opted_out = %opted_out, "Email preference changed");
            HttpResponse::Ok().json(serde_json::json!({"success": true, "opted_out": opted_out}))
        }
        Err(e) => {
            app_log!(error, error = %e, "Could not change email preference");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Could not save the preference"}))
        }
    }
}

pub async fn unsubscribe_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<OptOutRequest>,
) -> impl Responder {
    change(req, store, body, true).await
}

pub async fn resubscribe_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<OptOutRequest>,
) -> impl Responder {
    change(req, store, body, false).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_verifies_for_its_own_address_only() {
        secret_box::with_test_key(|| {
            let t = token("Jane@Example.com").expect("key is set");
            assert!(verify("jane@example.com", &t), "case and spacing do not matter");
            assert!(verify(" jane@example.com ", &t));
            assert!(!verify("john@example.com", &t));
            assert!(!verify("jane@example.com", "not-hex"));
            assert!(!verify("jane@example.com", ""));
        });
    }

    #[test]
    fn the_link_carries_the_encoded_address_and_its_token() {
        secret_box::with_test_key(|| {
            let url = link("jane+test@example.com").expect("key is set");
            let t = token("jane+test@example.com").unwrap();
            assert!(url.ends_with(&format!("?e=jane%2Btest%40example.com&t={t}")), "{url}");
        });
    }

    #[test]
    fn only_the_engagement_emails_are_optional() {
        use crate::email::EmailKind;
        let digest = EmailKind::MonthlyDigest { month: "May".into(), total_calls: 1, credits_spent: 1, top_endpoints: vec![] };
        assert!(digest.is_optional());
        assert!(EmailKind::Nudge { name: "x".into(), credits: 0 }.is_optional());
        assert!(EmailKind::WinBack { name: "x".into() }.is_optional());
        assert!(EmailKind::WhatsNew { feature_title: "x".into(), description: "y".into() }.is_optional());
        assert!(!EmailKind::Welcome { name: "x".into(), key_prefix: "k".into(), credits: 0 }.is_optional());
        assert!(!EmailKind::KeyRevoked { key_prefix: "k".into() }.is_optional());
        assert!(!EmailKind::AccountDeleted.is_optional());
    }

    #[test]
    fn the_footer_link_appears_only_when_given() {
        use crate::email::EmailKind;
        let nudge = EmailKind::Nudge { name: "Jane".into(), credits: 5 };
        let html = nudge.html_body_with(Some("https://gw.example/api/email/unsubscribe?e=a%40b&t=1"));
        assert!(html.contains(r#"href="https://gw.example/api/email/unsubscribe?e=a%40b&amp;t=1""#), "{html}");
        assert!(!nudge.html_body().contains("Unsubscribe"));
        assert!(!html.contains("Cursor"));
    }
}
