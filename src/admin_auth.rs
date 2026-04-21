// src/admin_auth.rs
//
// Lightweight Firebase JWT verifier for the api0 admin endpoints.
//
// Only one email is authorised: ADMIN_EMAIL.
// Firebase public keys are fetched on first use and cached in a global
// RwLock so key rotation (every ~6 h) is handled transparently.

use actix_web::{
    dev::Payload, error::ErrorUnauthorized, web, Error, FromRequest, HttpRequest,
};
use futures::future::{ready, Ready};
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};
use tokio::sync::RwLock;

const ADMIN_EMAIL: &str = "mohamed.bennekrouf@gmail.com";
const FIREBASE_KEYS_URL: &str =
    "https://www.googleapis.com/robot/v1/metadata/x509/securetoken@system.gserviceaccount.com";

// ── Global key cache ──────────────────────────────────────────────────────────

static KEY_CACHE: OnceLock<Arc<RwLock<HashMap<String, String>>>> = OnceLock::new();

fn key_cache() -> Arc<RwLock<HashMap<String, String>>> {
    KEY_CACHE
        .get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
        .clone()
}

async fn fetch_firebase_keys() -> anyhow::Result<HashMap<String, String>> {
    let client = reqwest::Client::builder()
        // Force IPv4 — Google blocks some IPv6 ranges (OVH etc.)
        .local_address(std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
        .build()?;
    let keys: HashMap<String, String> = client.get(FIREBASE_KEYS_URL).send().await?.json().await?;
    Ok(keys)
}

async fn get_public_key(kid: &str, project_id: &str) -> anyhow::Result<String> {
    // 1. Try cached keys
    {
        let cache = key_cache();
        let guard = cache.read().await;
        if let Some(k) = guard.get(kid) {
            return Ok(k.clone());
        }
    }

    // 2. Refresh
    let _ = project_id; // used only for validation below
    let keys = fetch_firebase_keys().await?;
    let result = keys
        .get(kid)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("Unknown Firebase key id '{}'", kid));

    // Update cache
    let cache = key_cache();
    let mut guard = cache.write().await;
    *guard = keys;

    result
}

// ── JWT claims ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
struct Claims {
    aud: String,
    iss: String,
    sub: String,
    email: String,
    email_verified: bool,
    exp: usize,
    iat: usize,
}

// ── Admin guard ───────────────────────────────────────────────────────────────

/// Actix-web extractor that verifies the Firebase JWT and checks the caller is
/// the admin. Inject it as a handler parameter; the request is rejected with
/// 401 if auth fails or the email doesn't match.
pub struct AdminUser {
    #[allow(dead_code)]
    pub email: String,
}

impl FromRequest for AdminUser {
    type Error = Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        // We can't .await inside FromRequest directly — spawn a blocking task
        // instead by extracting data synchronously from a pre-populated
        // request extension, or (simpler) use a manual async wrapper.
        //
        // Actix v4 supports async extractors via a custom Future; here we use
        // the simpler pattern of storing the verified email in an extension
        // set by middleware, or just block_on for the infrequent admin call.
        let token = match extract_bearer(req) {
            Some(t) => t,
            None => return ready(Err(ErrorUnauthorized("Missing Authorization header"))),
        };

        let project_id = match req.app_data::<web::Data<String>>() {
            Some(id) => id.get_ref().clone(),
            None => return ready(Err(ErrorUnauthorized("Server misconfiguration"))),
        };

        // Block on the async work — acceptable for an infrequent admin call
        // and avoids the complexity of a custom Future impl.
        let result = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(verify(&token, &project_id))
        });

        match result {
            Ok(email) if email.to_lowercase() == ADMIN_EMAIL => {
                ready(Ok(AdminUser { email }))
            }
            Ok(email) => {
                crate::app_log!(warn, caller = %email, "Admin endpoint: unauthorized caller");
                ready(Err(ErrorUnauthorized("Unauthorized")))
            }
            Err(e) => {
                crate::app_log!(warn, error = %e, "Admin endpoint: token verification failed");
                ready(Err(ErrorUnauthorized("Token verification failed")))
            }
        }
    }
}

fn extract_bearer(req: &HttpRequest) -> Option<String> {
    req.headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

async fn verify(token: &str, project_id: &str) -> anyhow::Result<String> {
    let header = jsonwebtoken::decode_header(token)?;
    let kid = header
        .kid
        .ok_or_else(|| anyhow::anyhow!("Missing kid in token header"))?;

    let public_key = get_public_key(&kid, project_id).await?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[project_id]);
    validation.set_issuer(&[format!(
        "https://securetoken.google.com/{}",
        project_id
    )]);

    let decoding_key = DecodingKey::from_rsa_pem(public_key.as_bytes())?;
    let token_data = decode::<Claims>(token, &decoding_key, &validation)?;

    Ok(token_data.claims.email)
}
