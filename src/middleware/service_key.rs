// src/middleware/service_key.rs
//
// Scoped credentials for first-party services that call the store directly —
// so they no longer need API0_INTERNAL_SECRET, which opens every
// internal route the store has.
//
// A service sends `X-Service-Key: <key>`. The store holds only each key's
// SHA-256 and the scopes it grants, in one environment variable:
//
//   API0_SERVICE_KEYS = "billing-app:<sha256 hex>:credits.read,credits.write,email.send"
//
// Several services are separated by `;`. To issue a key:
//
//   key=$(openssl rand -hex 32)
//   printf %s "$key" | shasum -a 256      # → the hex that goes here
//
// The key goes to the service, the hash to the store. A malformed entry is
// skipped with an error in the log, and an unset variable grants nothing.
//
// Routes that take a service key still accept X-Internal-Secret: the gateway
// and the bridge call the same routes and are trusted with everything.

use crate::app_log;
use actix_web::{HttpRequest, HttpResponse};
use sha2::{Digest, Sha256};

pub const SERVICE_KEYS_VAR: &str = "API0_SERVICE_KEYS";
pub const SERVICE_KEY_HEADER: &str = "X-Service-Key";

/// What a service may do. Each guarded route names exactly one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Read a person's credit balance.
    CreditsRead,
    /// Add or spend credits on a person's balance.
    CreditsWrite,
    /// Send an email through the platform's SMTP account.
    EmailSend,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::CreditsRead => "credits.read",
            Scope::CreditsWrite => "credits.write",
            Scope::EmailSend => "email.send",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "credits.read" => Some(Scope::CreditsRead),
            "credits.write" => Some(Scope::CreditsWrite),
            "email.send" => Some(Scope::EmailSend),
            _ => None,
        }
    }
}

/// Who a guarded request turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Caller {
    /// The gateway or the bridge, with the shared secret.
    Internal,
    /// A named service holding a key with the required scope.
    Service(String),
}

impl Caller {
    pub fn name(&self) -> &str {
        match self {
            Caller::Internal => "internal",
            Caller::Service(name) => name,
        }
    }
}

#[derive(Debug, PartialEq)]
struct ServiceEntry {
    name: String,
    hash: [u8; 32],
    scopes: Vec<Scope>,
}

/// Parse API0_SERVICE_KEYS. Entries that do not parse are dropped and logged —
/// one typo must not grant more, and should not take the other entries down.
fn parse_entries(raw: &str) -> Vec<ServiceEntry> {
    raw.split(';')
        .map(str::trim)
        .filter(|e| !e.is_empty())
        .filter_map(|entry| {
            let parsed = parse_entry(entry);
            if parsed.is_none() {
                // The entry holds only a hash, but log its name alone anyway.
                let name = entry.split(':').next().unwrap_or_default();
                app_log!(error, service = %name, "Ignoring a malformed {} entry", SERVICE_KEYS_VAR);
            }
            parsed
        })
        .collect()
}

fn parse_entry(entry: &str) -> Option<ServiceEntry> {
    let mut parts = entry.splitn(3, ':');
    let name = parts.next()?.trim();
    let hash_hex = parts.next()?.trim();
    let scopes_raw = parts.next()?;

    if name.is_empty() {
        return None;
    }
    let bytes = hex::decode(hash_hex).ok()?;
    let hash: [u8; 32] = bytes.try_into().ok()?;

    let scopes: Option<Vec<Scope>> = scopes_raw.split(',').map(Scope::parse).collect();
    let scopes = scopes.filter(|s| !s.is_empty())?;

    Some(ServiceEntry { name: name.to_string(), hash, scopes })
}

/// Compare without leaking how many leading bytes matched.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// The decision, separated from the request and the environment for tests.
fn decide(
    internal_secret: Option<&str>,
    presented_secret: Option<&str>,
    service_keys: &[ServiceEntry],
    presented_key: Option<&str>,
    scope: Scope,
) -> Result<Caller, Denial> {
    if let (Some(expected), Some(presented)) = (internal_secret, presented_secret) {
        if !expected.is_empty() && constant_time_eq(expected.as_bytes(), presented.as_bytes()) {
            return Ok(Caller::Internal);
        }
    }

    let key = match presented_key.filter(|k| !k.is_empty()) {
        Some(k) => k,
        None => return Err(Denial::Unauthenticated),
    };
    let digest: [u8; 32] = Sha256::digest(key.as_bytes()).into();

    match service_keys.iter().find(|e| constant_time_eq(&e.hash, &digest)) {
        None => Err(Denial::Unauthenticated),
        Some(e) if e.scopes.contains(&scope) => Ok(Caller::Service(e.name.clone())),
        Some(e) => Err(Denial::OutOfScope(e.name.clone())),
    }
}

#[derive(Debug, PartialEq)]
enum Denial {
    Unauthenticated,
    OutOfScope(String),
}

/// Guard a route: the internal secret, or a service key carrying `scope`.
///
///     let caller = match require_scope(&req, Scope::CreditsWrite) {
///         Ok(c) => c,
///         Err(deny) => return deny,
///     };
pub fn require_scope(req: &HttpRequest, scope: Scope) -> Result<Caller, HttpResponse> {
    let header = |name: &str| req.headers().get(name).and_then(|v| v.to_str().ok());
    let internal = std::env::var("API0_INTERNAL_SECRET").ok();
    let entries = parse_entries(&std::env::var(SERVICE_KEYS_VAR).unwrap_or_default());

    match decide(
        internal.as_deref(),
        header("X-Internal-Secret"),
        &entries,
        header(SERVICE_KEY_HEADER),
        scope,
    ) {
        Ok(caller) => Ok(caller),
        Err(Denial::Unauthenticated) => {
            app_log!(warn, path = %req.path(), scope = scope.as_str(), "Rejected a request with no valid internal secret or service key");
            Err(HttpResponse::Unauthorized().json(serde_json::json!({
                "success": false,
                "error": "Unauthorized"
            })))
        }
        Err(Denial::OutOfScope(name)) => {
            app_log!(warn, service = %name, path = %req.path(), scope = scope.as_str(), "A service key was used outside its scopes");
            Err(HttpResponse::Forbidden().json(serde_json::json!({
                "success": false,
                "error": format!("This service key does not grant {}", scope.as_str())
            })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "billing-app-test-key";

    fn hash_hex(key: &str) -> String {
        hex::encode(Sha256::digest(key.as_bytes()))
    }

    fn billing_app() -> Vec<ServiceEntry> {
        parse_entries(&format!("billing-app:{}:credits.read,credits.write,email.send", hash_hex(KEY)))
    }

    #[test]
    fn the_internal_secret_passes_every_scope() {
        for scope in [Scope::CreditsRead, Scope::CreditsWrite, Scope::EmailSend] {
            assert_eq!(decide(Some("s"), Some("s"), &[], None, scope), Ok(Caller::Internal));
        }
    }

    #[test]
    fn a_service_key_passes_its_own_scopes() {
        let entries = billing_app();
        assert_eq!(
            decide(None, None, &entries, Some(KEY), Scope::CreditsWrite),
            Ok(Caller::Service("billing-app".into()))
        );
    }

    #[test]
    fn a_service_key_is_refused_outside_its_scopes() {
        let entries = parse_entries(&format!("mailer:{}:email.send", hash_hex(KEY)));
        assert_eq!(
            decide(None, None, &entries, Some(KEY), Scope::CreditsWrite),
            Err(Denial::OutOfScope("mailer".into()))
        );
    }

    #[test]
    fn a_wrong_or_missing_key_is_unauthenticated() {
        let entries = billing_app();
        assert_eq!(decide(None, None, &entries, Some("guess"), Scope::CreditsRead), Err(Denial::Unauthenticated));
        assert_eq!(decide(None, None, &entries, None, Scope::CreditsRead), Err(Denial::Unauthenticated));
        assert_eq!(decide(None, None, &entries, Some(""), Scope::CreditsRead), Err(Denial::Unauthenticated));
    }

    #[test]
    fn nothing_configured_grants_nothing() {
        // An unset secret must not match an empty header, nor the empty key list a key.
        assert_eq!(decide(None, Some(""), &[], Some(KEY), Scope::EmailSend), Err(Denial::Unauthenticated));
        assert_eq!(decide(Some(""), Some(""), &[], None, Scope::EmailSend), Err(Denial::Unauthenticated));
    }

    #[test]
    fn the_key_itself_is_not_accepted_as_its_hash() {
        // Someone who read the store's environment has the hash, not the key.
        let entries = billing_app();
        let hash = hash_hex(KEY);
        assert_eq!(decide(None, None, &entries, Some(&hash), Scope::CreditsRead), Err(Denial::Unauthenticated));
    }

    #[test]
    fn malformed_entries_are_dropped_without_affecting_the_rest() {
        let raw = format!(
            "broken:nothex:credits.read; :{h}:email.send; typo:{h}:credits.everything; billing-app:{h}:credits.read",
            h = hash_hex(KEY)
        );
        let entries = parse_entries(&raw);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "billing-app");
        assert_eq!(entries[0].scopes, vec![Scope::CreditsRead]);
    }
}
