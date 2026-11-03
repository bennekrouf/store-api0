// src/api/signup_source.rs
//
// Where an account came from. Behind X-Internal-Secret (gateway-proxied).
//
//   PUT /user/signup-source — { email, source: { referrer, landing, utm_source, … } }
//
// The dashboard keeps what it saw on a visitor's first landing (the referrer,
// the page, the utm_* tags) and sends it once after they first sign in. Only a
// fresh account takes it, and only once: a returning user who arrives from a
// newsletter link has not "signed up" from it, and the first touch is the one
// that brought them.

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::sync::Arc;

/// The fields kept, and nothing else: the browser sends whatever it likes.
const FIELDS: &[&str] = &[
    "referrer",
    "landing",
    "utm_source",
    "utm_medium",
    "utm_campaign",
    "utm_term",
    "utm_content",
    "ref",
];

/// A URL or a tag, not an essay.
const MAX_LEN: usize = 300;

#[derive(Debug, Deserialize)]
pub struct SignupSourceRequest {
    pub email: String,
    pub source: Value,
}

/// The kept fields of `source`, trimmed and cut to length; `None` when nothing
/// is left to record.
pub fn clean_source(source: &Value) -> Option<Value> {
    let obj = source.as_object()?;
    let mut kept = Map::new();
    for field in FIELDS {
        if let Some(value) = obj.get(*field).and_then(Value::as_str) {
            let value = value.trim();
            if !value.is_empty() {
                kept.insert(field.to_string(), Value::String(value.chars().take(MAX_LEN).collect()));
            }
        }
    }
    (!kept.is_empty()).then_some(Value::Object(kept))
}

pub async fn set_signup_source_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SignupSourceRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let Some(source) = clean_source(&body.source) else {
        return HttpResponse::Ok().json(serde_json::json!({ "success": true, "recorded": false }));
    };

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "Signup source: no database connection");
            return HttpResponse::InternalServerError().json(serde_json::json!({ "success": false }));
        }
    };
    // A day is long enough for the dashboard's first visit after sign-up, and
    // short enough that an old account never takes a source.
    match client
        .execute(
            "UPDATE user_preferences SET signup_source = $2
              WHERE LOWER(email) = LOWER($1) AND signup_source IS NULL
                AND created_at > NOW() - INTERVAL '1 day'",
            &[&body.email, &source],
        )
        .await
    {
        Ok(n) => {
            if n > 0 {
                app_log!(info, email = %body.email, source = %source, "Signup source recorded");
            }
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "recorded": n > 0 }))
        }
        Err(e) => {
            app_log!(error, error = %e, email = %body.email, "Signup source not recorded");
            HttpResponse::InternalServerError().json(serde_json::json!({ "success": false }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn keeps_only_known_fields_trimmed_and_cut() {
        let long = "x".repeat(MAX_LEN + 50);
        let cleaned = clean_source(&json!({
            "referrer": "  https://www.reddit.com/r/mcp  ",
            "utm_source": long,
            "password": "nope",
            "utm_medium": "",
        }))
        .unwrap();
        assert_eq!(cleaned["referrer"], "https://www.reddit.com/r/mcp");
        assert_eq!(cleaned["utm_source"].as_str().unwrap().len(), MAX_LEN);
        assert!(cleaned.get("password").is_none());
        assert!(cleaned.get("utm_medium").is_none());
    }

    #[test]
    fn nothing_to_record_is_none() {
        assert!(clean_source(&json!({})).is_none());
        assert!(clean_source(&json!({ "referrer": "  " })).is_none());
        assert!(clean_source(&json!("https://x.com")).is_none());
    }
}
