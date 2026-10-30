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
//
// This is also the one record of who wants mayorana.ch's product emails (tips
// for the apps, credit reminders, the monthly digest), set from the box on the
// pricing page. The gateway passes the email of the person signed in:
//
//   GET  /api/internal/email/consent/{email}  → { opt_in: true|false|null, receives }
//   PUT  /api/internal/email/consent          { email, opt_in, source, locale? }
//
// An address is opted in (email_opt_ins), opted out (email_opt_outs) or has
// not chosen. Whether it gets product emails (`receives`):
//   opted in    yes
//   opted out   never, whatever its plan
//   not chosen  only while it has a plan it bought: an existing customer, told
//               before buying that they can refuse, may get news of similar
//               products (Swiss UWG art. 3 al. 1 let. o)
// Unsubscribing from any email opts out; undoing it opts back in.

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

/// The address's own choice: Some(true) opted in, Some(false) opted out, None
/// not chosen yet.
pub async fn choice(store: &EndpointStore, email: &str) -> anyhow::Result<Option<bool>> {
    let client = store.get_admin_conn().await?;
    let row = client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM email_opt_ins WHERE email = $1), \
                    EXISTS (SELECT 1 FROM email_opt_outs WHERE email = $1)",
            &[&normalise(email)],
        )
        .await?;
    Ok(match (row.get::<_, bool>(0), row.get::<_, bool>(1)) {
        (_, true) => Some(false),
        (true, false) => Some(true),
        (false, false) => None,
    })
}

/// Whether the address gets product emails, by the rule at the top.
pub fn receives(choice: Option<bool>, bought_plan: bool) -> bool {
    choice.unwrap_or(bought_plan)
}

/// Records a choice, replacing any earlier one. `source` says where it was made.
async fn set_choice(
    store: &EndpointStore,
    email: &str,
    opt_in: bool,
    source: &str,
    locale: Option<&str>,
) -> anyhow::Result<()> {
    let mut client = store.get_admin_conn().await?;
    let email = normalise(email);
    let tx = client.transaction().await?;
    if opt_in {
        tx.execute("DELETE FROM email_opt_outs WHERE email = $1", &[&email]).await?;
        tx.execute(
            "INSERT INTO email_opt_ins (email, source, locale) VALUES ($1, $2, $3) \
             ON CONFLICT (email) DO UPDATE SET source = EXCLUDED.source, locale = EXCLUDED.locale, created_at = NOW()",
            &[&email, &source, &locale],
        )
        .await?;
    } else {
        tx.execute("DELETE FROM email_opt_ins WHERE email = $1", &[&email]).await?;
        tx.execute(
            "INSERT INTO email_opt_outs (email) VALUES ($1) ON CONFLICT (email) DO NOTHING",
            &[&email],
        )
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn set_opted_out(store: &EndpointStore, email: &str, opted_out: bool) -> anyhow::Result<()> {
    set_choice(store, email, !opted_out, "email-link", None).await
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

// ── The box on mayorana.ch ───────────────────────────────────────────────────

/// GET /api/internal/email/consent/{email}
pub async fn consent_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> HttpResponse {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized().json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }
    let email = normalise(&path.into_inner());
    let (choice, bought) = match (
        choice(&store, &email).await,
        crate::payment::subscription::has_bought_plan(&store, &email).await,
    ) {
        (Ok(c), Ok(b)) => (c, b),
        (Err(e), _) => return consent_failed(&e.to_string()),
        (_, Err(e)) => return consent_failed(&e),
    };
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "opt_in": choice,
        "receives": receives(choice, bought),
    }))
}

#[derive(Deserialize)]
pub struct ConsentRequest {
    pub email: String,
    pub opt_in: bool,
    /// Where the choice was made: "pricing", "founding".
    pub source: String,
    #[serde(default)]
    pub locale: Option<String>,
}

/// PUT /api/internal/email/consent
pub async fn set_consent_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<ConsentRequest>,
) -> HttpResponse {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized().json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }
    let source = body.source.trim();
    if source.is_empty() || source.len() > 32 || !source.chars().all(|c| c.is_ascii_lowercase() || c == '-') {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"success": false, "error": "Unknown source"}));
    }
    let locale = body.locale.as_deref().map(|l| super::Lang::from_tag(Some(l)).tag());
    if let Err(e) = set_choice(&store, &body.email, body.opt_in, source, locale).await {
        return consent_failed(&e.to_string());
    }
    app_log!(info, opt_in = %body.opt_in, source = %source, "Email consent changed");
    HttpResponse::Ok().json(serde_json::json!({"success": true, "opt_in": body.opt_in}))
}

fn consent_failed(error: &str) -> HttpResponse {
    app_log!(error, error = %error, "Could not read or save email consent");
    HttpResponse::InternalServerError()
        .json(serde_json::json!({"success": false, "error": "Could not read or save the preference"}))
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
    fn a_choice_wins_over_the_plan_and_no_choice_follows_it() {
        assert!(receives(Some(true), false));
        assert!(!receives(Some(false), true), "a no stays a no for a customer");
        assert!(receives(None, true), "a customer who never chose gets them");
        assert!(!receives(None, false), "anyone else must ask");
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
