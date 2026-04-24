use crate::app_log;
use crate::{
    endpoint_store::EndpointStore,
    infra::models::{ValidateKeyRequest, ValidateKeyResponse},
};
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

// ── Brute-force rate limiter ──────────────────────────────────────────────────
// Tracks failed validation attempts per key-prefix (first 16 chars of the raw
// API key submitted).  Exceeding MAX_FAILURES in WINDOW_SECS returns HTTP 429.
// This is an in-process defence layer; a reverse-proxy limit should also exist.

const MAX_FAILURES: u32 = 10;
const WINDOW_SECS: u64 = 60;
// Evict stale entries when the map exceeds this size to bound memory usage.
const EVICT_THRESHOLD: usize = 5_000;

struct FailureWindow {
    count: u32,
    started: Instant,
}

static FAIL_MAP: OnceLock<Mutex<HashMap<String, FailureWindow>>> = OnceLock::new();

fn fail_map() -> &'static Mutex<HashMap<String, FailureWindow>> {
    FAIL_MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Returns `true` if the caller is within limits (request allowed).
fn rate_limit_check(limit_key: &str) -> bool {
    let mut map = fail_map().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();

    // Periodic eviction of expired windows to cap memory.
    if map.len() >= EVICT_THRESHOLD {
        map.retain(|_, w| now.duration_since(w.started).as_secs() < WINDOW_SECS);
    }

    match map.get(limit_key) {
        None => true,
        Some(w) => {
            if now.duration_since(w.started).as_secs() >= WINDOW_SECS {
                true // window has expired — reset on next record_failure call
            } else {
                w.count < MAX_FAILURES
            }
        }
    }
}

/// Call after a failed validation to increment the counter.
fn record_failure(limit_key: &str) {
    let mut map = fail_map().lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    let entry = map.entry(limit_key.to_string()).or_insert(FailureWindow { count: 0, started: now });
    if now.duration_since(entry.started).as_secs() >= WINDOW_SECS {
        entry.count = 1;
        entry.started = now;
    } else {
        entry.count = entry.count.saturating_add(1);
    }
}

// ── Handler ───────────────────────────────────────────────────────────────────

// Handler for validating an API key
pub async fn validate_api_key(
    store: web::Data<Arc<EndpointStore>>,
    req: web::Json<ValidateKeyRequest>,
    http_req: HttpRequest,
) -> impl Responder {
    // Try to get API key from request body first, then from Authorization header
    let api_key = if !req.api_key.is_empty() {
        req.api_key.clone()
    } else if let Some(auth_header) = http_req.headers().get("authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            if auth_str.starts_with("Bearer ") {
                auth_str.strip_prefix("Bearer ").unwrap_or("").to_string()
            } else {
                auth_str.to_string()
            }
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    if api_key.is_empty() {
        app_log!(
            warn,
            "No API key provided in request body or Authorization header"
        );
        return HttpResponse::BadRequest().json(ValidateKeyResponse {
            valid: false,
            email: None,
            key_id: None,
            tenant_id: None,
            provider_tenant_id: None,
            message: "No API key provided".to_string(),
        });
    }

    // Rate-limit by the first 16 chars of the submitted key so brute-force
    // attempts against a known key prefix are throttled without leaking the key.
    let limit_key = if api_key.len() >= 16 { &api_key[..16] } else { &api_key };
    if !rate_limit_check(limit_key) {
        app_log!(warn, limit_key = %limit_key, "Rate limit exceeded on key validation");
        return HttpResponse::TooManyRequests().json(ValidateKeyResponse {
            valid: false,
            email: None,
            key_id: None,
            tenant_id: None,
            provider_tenant_id: None,
            message: "Too many validation attempts — try again later".to_string(),
        });
    }

    app_log!(info, expected_tenant_id = ?req.expected_tenant_id, "Validating API key");

    match store.validate_api_key(&api_key, req.expected_tenant_id.as_deref()).await {
        Ok(Some((email, key_id, tenant_id, provider_tenant_id))) => {
            app_log!(info,
                email = %email,
                key_id = %key_id,
                tenant_id = %tenant_id,
                provider_tenant_id = ?provider_tenant_id,
                "API key validation successful"
            );

            HttpResponse::Ok().json(ValidateKeyResponse {
                valid: true,
                email: Some(email),
                key_id: Some(key_id),
                tenant_id: Some(tenant_id),
                provider_tenant_id,
                message: "API key is valid".to_string(),
            })
        }
        Ok(None) => {
            record_failure(limit_key);
            app_log!(warn, "Invalid API key provided");

            HttpResponse::Ok().json(ValidateKeyResponse {
                valid: false,
                email: None,
                key_id: None,
                tenant_id: None,
                provider_tenant_id: None,
                message: "Invalid API key".to_string(),
            })
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                "Database error during API key validation"
            );

            HttpResponse::InternalServerError().json(ValidateKeyResponse {
                valid: false,
                email: None,
                key_id: None,
                tenant_id: None,
                provider_tenant_id: None,
                message: "Validation error".to_string(),
            })
        }
    }
}
