// src/payment/license.rs
//
// Licences for the desktop apps sold on mayorana.ch (Splitter first).
//
// A buyer pays on a Stripe Checkout page; the licence is issued when the
// session is paid, whichever comes first of:
//   - the checkout.session.completed webhook (payment::webhook), or
//   - the thank-you page asking for its key (GET /licenses/session/{id}),
// and only once, enforced by the UNIQUE stripe_session_id on `licenses`.
//
// The key is checked by the app offline, against the public half of
// LICENSE_SIGNING_KEY built into it:
//
//     <base64url(payload JSON)>.<base64url(Ed25519 signature of those bytes)>
//
// payload: {"v":1,"id","product","edition","email","issued","updates_until"}
// Dates are YYYY-MM-DD. The app compares updates_until with its own release
// date: a licence unlocks every release up to that day, and keeps unlocking
// those releases after it.

use crate::app_log;
use crate::email::{send_async, EmailKind};
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::service::PaymentService;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{Days, NaiveDate, Utc};
use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use stripe::{CheckoutSession, CheckoutSessionPaymentStatus, PriceTaxBehavior};

/// Metadata `purpose` on licence Checkout sessions and their PaymentIntents.
pub const PURPOSE_LICENSE: &str = "license";

/// How long a licence unlocks new releases.
const UPDATES_DAYS: u64 = 365;

/// What can be sold: (product, edition) → the display name. The Stripe price
/// for each comes from LICENSE_PRICE_<PRODUCT>_<EDITION>.
fn product_name(product: &str, edition: &str) -> Option<&'static str> {
    match (product, edition) {
        ("splitter", "pro") => Some("Splitter Pro"),
        ("gitagent", "pro") => Some("GitAgent Pro"),
        ("small-video", "pro") => Some("Small Video Pro"),
        ("spreadwatch", "pro") => Some("Spreadwatch Pro"),
        _ => None,
    }
}

/// Where the key goes in each app, for the licence email (HTML).
fn activation_html(product: &str) -> &'static str {
    match product {
        "splitter" => {
            "open Splitter and click <strong>Get Pro…</strong> at the top of the file list \
             (in versions before 0.1.14, click the <strong>Splitter</strong> name there), then paste the key in."
        }
        "gitagent" => "open GitAgent, click <strong>Get Pro…</strong> in the top bar and paste the key in.",
        "small-video" => "open Small Video, click <strong>Get Pro…</strong> in the sidebar and paste the key in.",
        "spreadwatch" => {
            "open Spreadwatch, click <strong>Get Pro…</strong> at the right of the tab bar and paste the key in."
        }
        _ => "open the app and paste the key into its licence window.",
    }
}

/// `small-video` → `SMALL_VIDEO`: the name has to be a valid shell variable,
/// since store.env is read with `source`.
fn price_env(product: &str, edition: &str) -> String {
    let var = |s: &str| s.to_uppercase().replace('-', "_");
    format!("LICENSE_PRICE_{}_{}", var(product), var(edition))
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct LicensePayload {
    pub v: u8,
    pub id: String,
    pub product: String,
    pub edition: String,
    pub email: String,
    pub issued: NaiveDate,
    pub updates_until: NaiveDate,
}

/// The signing key, from LICENSE_SIGNING_KEY: the 32-byte Ed25519 seed,
/// standard base64. Generate one with `openssl rand -base64 32`; the matching
/// public key goes into the app.
fn signing_key() -> Result<SigningKey, String> {
    let b64 = std::env::var("LICENSE_SIGNING_KEY")
        .map_err(|_| "LICENSE_SIGNING_KEY is not set".to_string())?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| format!("LICENSE_SIGNING_KEY is not base64: {}", e))?;
    let seed: [u8; 32] = bytes
        .try_into()
        .map_err(|_| "LICENSE_SIGNING_KEY must be 32 bytes".to_string())?;
    Ok(SigningKey::from_bytes(&seed))
}

pub fn encode_key(payload: &LicensePayload, key: &SigningKey) -> String {
    let json = serde_json::to_vec(payload).expect("licence payload serialises");
    let signature = key.sign(&json);
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&json),
        URL_SAFE_NO_PAD.encode(signature.to_bytes())
    )
}

pub struct Issued {
    pub key: String,
    pub email: String,
    pub updates_until: NaiveDate,
    pub product_name: &'static str,
}

#[derive(Debug)]
pub enum IssueError {
    /// Not a licence session, or not paid (yet).
    NotIssuable(String),
    Config(String),
    Store(String),
}

impl std::fmt::Display for IssueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotIssuable(why) => write!(f, "{}", why),
            Self::Config(e) => write!(f, "Licence signing is not configured: {}", e),
            Self::Store(e) => write!(f, "Licence could not be stored: {}", e),
        }
    }
}

/// The licence for a completed Checkout session, created the first time this
/// is called for it and read back every time after. Emails the key once, when
/// it is created.
pub async fn issue_for_session(
    store: &Arc<EndpointStore>,
    session: &CheckoutSession,
) -> Result<Issued, IssueError> {
    let meta = session.metadata.clone().unwrap_or_default();
    if meta.get("purpose").map(String::as_str) != Some(PURPOSE_LICENSE) {
        return Err(IssueError::NotIssuable("Not a licence purchase".into()));
    }
    if session.payment_status != CheckoutSessionPaymentStatus::Paid {
        return Err(IssueError::NotIssuable(format!(
            "Payment not completed ({:?})",
            session.payment_status
        )));
    }
    let product = meta.get("product").cloned().unwrap_or_default();
    let edition = meta.get("edition").cloned().unwrap_or_default();
    let name = product_name(&product, &edition)
        .ok_or_else(|| IssueError::NotIssuable(format!("Unknown product {}/{}", product, edition)))?;
    let email = session
        .customer_details
        .as_ref()
        .and_then(|d| d.email.clone())
        .or_else(|| session.customer_email.clone())
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
        .ok_or_else(|| IssueError::NotIssuable("Checkout collected no email".into()))?;
    let session_id = session.id.to_string();
    let intent_id = session.payment_intent.as_ref().map(|p| p.id().to_string());

    if let Some(existing) = find_by_session(store, &session_id).await.map_err(|e| IssueError::Store(e.to_string()))? {
        return Ok(Issued { product_name: name, ..existing });
    }

    let signer = signing_key().map_err(IssueError::Config)?;
    let issued = Utc::now().date_naive();
    let payload = LicensePayload {
        v: 1,
        id: format!("lic_{}", uuid::Uuid::new_v4().simple()),
        product,
        edition,
        email: email.clone(),
        issued,
        updates_until: issued + Days::new(UPDATES_DAYS),
    };
    let key = encode_key(&payload, &signer);

    let client = store.get_admin_conn().await.map_err(|e| IssueError::Store(e.to_string()))?;
    let inserted = client
        .execute(
            "INSERT INTO licenses \
             (id, product, edition, email, stripe_session_id, stripe_payment_intent_id, key, updates_until, issued_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW()) \
             ON CONFLICT (stripe_session_id) DO NOTHING",
            &[&payload.id, &payload.product, &payload.edition, &email, &session_id, &intent_id, &key, &payload.updates_until],
        )
        .await
        .map_err(|e| IssueError::Store(e.to_string()))?;

    if inserted == 0 {
        // The webhook and the thank-you page raced; the other one won.
        let existing = find_by_session(store, &session_id)
            .await
            .map_err(|e| IssueError::Store(e.to_string()))?
            .ok_or_else(|| IssueError::Store("licence vanished after a conflict".into()))?;
        return Ok(Issued { product_name: name, ..existing });
    }

    app_log!(info, license_id = %payload.id, product = %payload.product, email = %email, session_id = %session_id, "Licence issued");
    send_async(Arc::clone(store), email.clone(), EmailKind::LicenseIssued {
        product_name: name.to_string(),
        how: activation_html(&payload.product).to_string(),
        key: key.clone(),
        updates_until: payload.updates_until.to_string(),
    });
    Ok(Issued { key, email, updates_until: payload.updates_until, product_name: name })
}

async fn find_by_session(store: &EndpointStore, session_id: &str) -> Result<Option<Issued>, StoreError> {
    let client = store.get_admin_conn().await?;
    let row = client
        .query_opt(
            "SELECT key, email, updates_until FROM licenses WHERE stripe_session_id = $1",
            &[&session_id],
        )
        .await
        .to_store_error()?;
    Ok(row.map(|r| Issued {
        key: r.get(0),
        email: r.get(1),
        updates_until: r.get(2),
        product_name: "",
    }))
}

/// Marks the licences paid by `payment_intent_id` as revoked (refunds).
/// Returns how many were revoked.
pub async fn revoke_for_payment(store: &EndpointStore, payment_intent_id: &str) -> Result<u64, StoreError> {
    let client = store.get_admin_conn().await?;
    client
        .execute(
            "UPDATE licenses SET revoked_at = NOW() WHERE stripe_payment_intent_id = $1 AND revoked_at IS NULL",
            &[&payment_intent_id],
        )
        .await
        .to_store_error()
}

// ── HTTP (X-Internal-Secret: reached through the gateway) ────────────────────

#[derive(Deserialize)]
pub struct CheckoutRequest {
    pub product: String,
    pub edition: String,
}

/// POST /api/licenses/checkout — a Stripe Checkout URL to send the buyer to.
pub async fn create_checkout_handler(
    req: HttpRequest,
    payment_service: web::Data<Arc<PaymentService>>,
    body: web::Json<CheckoutRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let (product, edition) = (body.product.to_lowercase(), body.edition.to_lowercase());
    if product_name(&product, &edition).is_none() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false, "message": "Unknown product",
        }));
    }
    let Ok(price_id) = std::env::var(price_env(&product, &edition)) else {
        app_log!(error, env = %price_env(&product, &edition), "No Stripe price configured for licence checkout");
        return HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "success": false, "message": "This product is not on sale yet",
        }));
    };
    let site = std::env::var("LICENSE_SITE_URL").unwrap_or_else(|_| "https://mayorana.ch/en/apps".into());
    let success_url = format!("{}/{}/thanks?session_id={{CHECKOUT_SESSION_ID}}", site, product);
    let cancel_url = format!("{}/{}", site, product);
    let metadata = HashMap::from([
        ("purpose".to_string(), PURPOSE_LICENSE.to_string()),
        ("product".to_string(), product.clone()),
        ("edition".to_string(), edition.clone()),
    ]);

    match payment_service
        .create_license_checkout(&price_id, &success_url, &cancel_url, metadata)
        .await
    {
        Ok(session) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "url": session.url,
        })),
        Err(e) => {
            app_log!(error, error = %e, product = %product, "Failed to create licence checkout");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false, "message": "Checkout could not be started",
            }))
        }
    }
}

/// How long a price read from Stripe is reused. The product pages ask on
/// every view; a price change shows up on the site within this long.
const PRICE_TTL: Duration = Duration::from_secs(600);

fn price_cache() -> &'static Mutex<HashMap<String, (Instant, serde_json::Value)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, serde_json::Value)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// GET /api/licenses/price/{product}/{edition} — what the Checkout page will
/// charge, read from the same Stripe price, so the number shown next to the
/// buy button is the one charged: { amount (smallest currency unit),
/// currency, vat_added (Stripe Tax adds VAT on top at checkout) }.
pub async fn price_handler(
    req: HttpRequest,
    payment_service: web::Data<Arc<PaymentService>>,
    path: web::Path<(String, String)>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let (product, edition) = (path.0.to_lowercase(), path.1.to_lowercase());
    if product_name(&product, &edition).is_none() {
        return HttpResponse::NotFound().json(serde_json::json!({
            "success": false, "message": "Unknown product",
        }));
    }
    let Ok(price_id) = std::env::var(price_env(&product, &edition)) else {
        return HttpResponse::NotFound().json(serde_json::json!({
            "success": false, "message": "This product is not on sale yet",
        }));
    };

    if let Some((at, body)) = price_cache().lock().unwrap().get(&price_id) {
        if at.elapsed() < PRICE_TTL {
            return HttpResponse::Ok().json(body);
        }
    }

    let price = match payment_service.retrieve_price(&price_id).await {
        Ok(price) => price,
        Err(e) => {
            app_log!(error, error = %e, product = %product, "Failed to read licence price from Stripe");
            return HttpResponse::ServiceUnavailable().json(serde_json::json!({
                "success": false, "message": "Price unavailable",
            }));
        }
    };
    let (Some(amount), Some(currency)) = (price.unit_amount, price.currency) else {
        app_log!(error, product = %product, "Licence price has no fixed amount");
        return HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "success": false, "message": "Price unavailable",
        }));
    };
    let automatic_tax = std::env::var("STRIPE_AUTOMATIC_TAX").as_deref() == Ok("true");
    let body = serde_json::json!({
        "success": true,
        "amount": amount,
        "currency": currency.to_string(),
        "vat_added": automatic_tax && price.tax_behavior == Some(PriceTaxBehavior::Exclusive),
    });
    price_cache().lock().unwrap().insert(price_id, (Instant::now(), body.clone()));
    HttpResponse::Ok().json(body)
}

/// GET /api/licenses/session/{session_id} — the key for the thank-you page.
/// Issues it if the webhook has not arrived yet; 409 while the payment is
/// still processing, so the page can retry.
pub async fn session_license_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    payment_service: web::Data<Arc<PaymentService>>,
    session_id: web::Path<String>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let session = match payment_service.retrieve_checkout_session(&session_id).await {
        Ok(s) => s,
        Err(e) => {
            app_log!(warn, error = %e, session_id = %session_id, "Licence lookup for an unknown session");
            return HttpResponse::NotFound().json(serde_json::json!({
                "success": false, "message": "No such purchase",
            }));
        }
    };
    match issue_for_session(store.get_ref(), &session).await {
        Ok(issued) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "product_name": issued.product_name,
            "key": issued.key,
            "email": issued.email,
            "updates_until": issued.updates_until.to_string(),
        })),
        Err(IssueError::NotIssuable(why)) => HttpResponse::Conflict().json(serde_json::json!({
            "success": false, "message": why,
        })),
        Err(e) => {
            app_log!(error, error = %e, session_id = %session_id, "Licence could not be issued");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false, "message": "The licence could not be issued; we have been notified",
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier};

    #[test]
    fn key_is_payload_and_signature_that_verify() {
        let signer = SigningKey::from_bytes(&[7u8; 32]);
        let payload = LicensePayload {
            v: 1,
            id: "lic_test".into(),
            product: "splitter".into(),
            edition: "pro".into(),
            email: "anna@band.ch".into(),
            issued: NaiveDate::from_ymd_opt(2026, 9, 27).unwrap(),
            updates_until: NaiveDate::from_ymd_opt(2027, 9, 27).unwrap(),
        };
        let key = encode_key(&payload, &signer);
        let (body, sig) = key.split_once('.').unwrap();
        let json = URL_SAFE_NO_PAD.decode(body).unwrap();
        let sig = Signature::from_slice(&URL_SAFE_NO_PAD.decode(sig).unwrap()).unwrap();
        signer.verifying_key().verify(&json, &sig).unwrap();
        let back: LicensePayload = serde_json::from_slice(&json).unwrap();
        assert_eq!(back, payload);
        assert!(String::from_utf8(json).unwrap().contains(r#""updates_until":"2027-09-27""#));
    }

    /// The same key is checked by Splitter's own tests (splitter-core, license.rs):
    /// if either side changes the format, one of them fails.
    pub const FIXTURE_KEY: &str = "eyJ2IjoxLCJpZCI6ImxpY19maXh0dXJlIiwicHJvZHVjdCI6InNwbGl0dGVyIiwiZWRpdGlvbiI6InBybyIsImVtYWlsIjoiYW5uYUBiYW5kLmNoIiwiaXNzdWVkIjoiMjAyNi0wOS0yNyIsInVwZGF0ZXNfdW50aWwiOiIyMDI3LTA5LTI3In0.pgBj4R164hG5dGk-jfLUKINF7zOBSPO8to5PImF8QoDp4dRHGgX67ORXy_eC9Pyd9CqUxGj_r6_ETd1oHWI3BQ";

    #[test]
    fn issues_the_key_splitter_expects() {
        let payload = LicensePayload {
            v: 1,
            id: "lic_fixture".into(),
            product: "splitter".into(),
            edition: "pro".into(),
            email: "anna@band.ch".into(),
            issued: NaiveDate::from_ymd_opt(2026, 9, 27).unwrap(),
            updates_until: NaiveDate::from_ymd_opt(2027, 9, 27).unwrap(),
        };
        let key = encode_key(&payload, &SigningKey::from_bytes(&[7u8; 32]));
        assert_eq!(key, FIXTURE_KEY);
    }

    #[test]
    fn only_known_products_are_for_sale() {
        assert_eq!(product_name("splitter", "pro"), Some("Splitter Pro"));
        assert_eq!(product_name("gitagent", "pro"), Some("GitAgent Pro"));
        assert_eq!(product_name("splitter", "free"), None);
        assert!(activation_html("gitagent").contains("top bar"));
        assert_eq!(product_name("spreadwatch", "pro"), Some("Spreadwatch Pro"));
        assert!(activation_html("spreadwatch").contains("tab bar"));
        assert_eq!(price_env("spreadwatch", "pro"), "LICENSE_PRICE_SPREADWATCH_PRO");
        assert_eq!(price_env("splitter", "pro"), "LICENSE_PRICE_SPLITTER_PRO");
        assert_eq!(product_name("small-video", "pro"), Some("Small Video Pro"));
        assert_eq!(price_env("small-video", "pro"), "LICENSE_PRICE_SMALL_VIDEO_PRO");
    }

    /// `TEST_DATABASE_URL=postgres://… cargo test -- --ignored license_db`
    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL"]
    async fn license_db_one_licence_per_session_and_refund_revokes_it() {
        use stripe::{PaymentPagesCheckoutSessionCustomerDetails, Expandable};
        std::env::set_var("LICENSE_SIGNING_KEY", base64::engine::general_purpose::STANDARD.encode([9u8; 32]));
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = Arc::new(EndpointStore::new(&url).await.expect("store"));

        let suffix = uuid::Uuid::new_v4().simple().to_string();
        let intent_id = format!("pi_{}", suffix);
        let session = CheckoutSession {
            id: format!("cs_test_{}", suffix).parse().unwrap(),
            payment_status: CheckoutSessionPaymentStatus::Paid,
            metadata: Some(HashMap::from([
                ("purpose".to_string(), PURPOSE_LICENSE.to_string()),
                ("product".to_string(), "splitter".to_string()),
                ("edition".to_string(), "pro".to_string()),
            ])),
            customer_details: Some(PaymentPagesCheckoutSessionCustomerDetails {
                email: Some("Anna@Band.ch".into()),
                ..Default::default()
            }),
            payment_intent: Some(Expandable::Id(intent_id.parse().unwrap())),
            ..Default::default()
        };

        let (a, b) = tokio::join!(issue_for_session(&store, &session), issue_for_session(&store, &session));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a.key, b.key, "the webhook and the thank-you page get the same licence");
        assert_eq!(a.email, "anna@band.ch");
        assert_eq!(a.product_name, "Splitter Pro");

        let unpaid = CheckoutSession { payment_status: CheckoutSessionPaymentStatus::Unpaid, ..session.clone() };
        assert!(matches!(issue_for_session(&store, &unpaid).await, Err(IssueError::NotIssuable(_))));

        assert_eq!(revoke_for_payment(&store, &intent_id).await.unwrap(), 1);
        assert_eq!(revoke_for_payment(&store, &intent_id).await.unwrap(), 0);
    }
}
