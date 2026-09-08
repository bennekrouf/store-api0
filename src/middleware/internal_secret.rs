// src/middleware/internal_secret.rs
//
// The store sits behind the gateway and has no user authentication of its own.
// Routes that read or write a tenant's credentials must therefore prove the
// caller is the gateway, not a browser that happened to find the port.
//
// Returns `Some(401)` to short-circuit the handler, `None` to continue:
//
//     if let Some(deny) = require_internal_secret(&req) { return deny; }
//
// An unset or empty `API0_INTERNAL_SECRET` denies everything rather than
// allowing it. A misconfigured deployment should fail closed and loudly.

use crate::app_log;
use actix_web::{HttpRequest, HttpResponse};

pub fn require_internal_secret(req: &HttpRequest) -> Option<HttpResponse> {
    let expected = std::env::var("API0_INTERNAL_SECRET").ok();
    let presented = req
        .headers()
        .get("X-Internal-Secret")
        .and_then(|v| v.to_str().ok());

    if is_authorised(expected.as_deref(), presented) {
        return None;
    }

    if expected.as_deref().unwrap_or_default().is_empty() {
        app_log!(error, "API0_INTERNAL_SECRET is unset — denying an internal request");
    } else {
        app_log!(
            warn,
            path = %req.path(),
            "Rejected a request to an internal route without the shared secret"
        );
    }
    Some(unauthorized())
}

/// The decision, separated from the request so it can be tested without
/// mutating process-wide environment state from a parallel test run.
fn is_authorised(expected: Option<&str>, presented: Option<&str>) -> bool {
    match (expected, presented) {
        // Fail closed: no configured secret means nothing is authorised, rather
        // than everything being authorised against the empty string.
        (None, _) | (Some(""), _) => false,
        (Some(expected), Some(presented)) => expected == presented,
        (Some(_), None) => false,
    }
}

fn unauthorized() -> HttpResponse {
    HttpResponse::Unauthorized().json(serde_json::json!({
        "success": false,
        "error": "Unauthorized"
    }))
}

#[cfg(test)]
mod tests {
    use super::is_authorised;

    #[test]
    fn the_matching_secret_is_allowed() {
        assert!(is_authorised(Some("s3cret"), Some("s3cret")));
    }

    #[test]
    fn a_wrong_or_absent_header_is_denied() {
        assert!(!is_authorised(Some("s3cret"), Some("guess")));
        assert!(!is_authorised(Some("s3cret"), None));
    }

    #[test]
    fn an_unset_secret_denies_everything() {
        // The dangerous shape: a deployment that forgot the variable must not
        // become one where every caller matches the empty string.
        assert!(!is_authorised(None, None));
        assert!(!is_authorised(None, Some("")));
        assert!(!is_authorised(Some(""), Some("")));
    }
}
