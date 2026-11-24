// src/api/signup_source.rs
//
// Where a person came from, and where they sign in. Behind X-Internal-Secret
// (gateway-proxied).
//
//   PUT /user/signup-source — { email, via, source?: { referrer, landing, utm_source, … } }
//
// Every front-end reports a sign-in under its own `via`: the api0 dashboard,
// a site that signs people in through the gateway, a desktop app asking for
// its licence, a partner service with a store key (record, called from the
// licence route). Each report moves person_sources.seen[via] to now. The first
// report for a person new that day also becomes their first_source, with the
// referrer and tags that front-end saw on their first landing. Only then, and
// only once: a returning user who arrives from a newsletter link has not
// "signed up" from it, and the first touch is the one that brought them.

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::sync::Arc;
use deadpool_postgres::GenericClient;

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
/// A front-end's name is a short id.
const MAX_VIA_LEN: usize = 40;

#[derive(Debug, Deserialize)]
pub struct SignupSourceRequest {
    pub email: String,
    /// Which front-end the person signed in on. Older dashboards send none.
    #[serde(default)]
    pub via: Option<String>,
    #[serde(default)]
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

/// A front-end's name as a key: lower case, letters, digits and `.-_` only.
/// `None` when nothing usable is left.
pub fn clean_via(via: &str) -> Option<String> {
    let cleaned: String = via
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' })
        .take(MAX_VIA_LEN)
        .collect();
    let cleaned = cleaned.trim_matches('-').to_string();
    (!cleaned.is_empty()).then_some(cleaned)
}

/// The first_source a report would write: its tags, plus where it came in.
fn first_source(via: &str, source: Option<Value>) -> Value {
    let mut first = match source {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    first.insert("via".into(), Value::String(via.to_string()));
    Value::Object(first)
}

/// Record that `email` signed in on `via`, and where they came from if they
/// are new. Returns whether this report became their first_source.
pub async fn record<C: GenericClient>(
    client: &C,
    email: &str,
    via: &str,
    source: Option<Value>,
) -> Result<bool, tokio_postgres::Error> {
    let email = email.trim().to_lowercase();
    // New: nothing about them, account or licence, is older than a day. A day
    // covers the gap between creating the account and the first report.
    let new: bool = client
        .query_one(
            "SELECT NOT EXISTS (SELECT 1 FROM subscriptions
                                 WHERE email = $1 AND created_at < NOW() - INTERVAL '1 day')
                AND NOT EXISTS (SELECT 1 FROM user_preferences
                                 WHERE lower(email) = $1 AND created_at < NOW() - INTERVAL '1 day')",
            &[&email],
        )
        .await?
        .get(0);
    let first = new.then(|| first_source(via, source));
    let row = client
        .query_one(
            "INSERT INTO person_sources AS ps (email, first_source, seen)
             VALUES ($1, $2, jsonb_build_object($3::text, NOW()))
             ON CONFLICT (email) DO UPDATE
                SET seen = ps.seen || jsonb_build_object($3::text, NOW()),
                    first_source = COALESCE(ps.first_source, EXCLUDED.first_source)
             RETURNING first_source IS NOT NULL AND first_source = $2",
            &[&email, &first, &via],
        )
        .await?;
    // NULL when nothing was offered ($2 NULL).
    Ok(row.get::<_, Option<bool>>(0).unwrap_or(false))
}

/// `record`, from a route whose answer does not depend on it: a failure is
/// only logged.
pub async fn record_quietly(store: &EndpointStore, email: &str, via: &str, source: Option<Value>) {
    let result = match store.get_admin_conn().await {
        Ok(client) => record(&client, email, via, source).await.map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    if let Err(e) = result {
        app_log!(error, error = %e, email = %email, via = %via, "Sign-in source not recorded");
    }
}

pub async fn set_signup_source_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SignupSourceRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    // The gateway names api0's own sign-ins; a dashboard from before `via`
    // sends none, and is api0's.
    let Some(via) = clean_via(body.via.as_deref().unwrap_or("api0")) else {
        return HttpResponse::BadRequest().json(serde_json::json!({ "success": false, "error": "Unknown front-end" }));
    };
    let source = clean_source(&body.source);

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "Signup source: no database connection");
            return HttpResponse::InternalServerError().json(serde_json::json!({ "success": false }));
        }
    };
    match record(&client, &body.email, &via, source).await {
        Ok(first) => {
            if first {
                app_log!(info, email = %body.email, via = %via, "Signup source recorded");
            }
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "recorded": first }))
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
        assert!(clean_source(&Value::Null).is_none());
    }

    #[test]
    fn via_is_a_short_key() {
        assert_eq!(clean_via(" Mayorana.CH ").as_deref(), Some("mayorana.ch"));
        assert_eq!(clean_via("git agent/2").as_deref(), Some("git-agent-2"));
        assert_eq!(clean_via(&"a".repeat(80)).unwrap().len(), MAX_VIA_LEN);
        assert!(clean_via("  ").is_none());
        assert!(clean_via("///").is_none());
    }

    #[test]
    fn first_source_names_where_it_came_in() {
        assert_eq!(first_source("api0", None), json!({ "via": "api0" }));
        assert_eq!(
            first_source("site", Some(json!({ "utm_source": "reddit", "via": "spoofed" }))),
            json!({ "utm_source": "reddit", "via": "site" })
        );
    }
}
