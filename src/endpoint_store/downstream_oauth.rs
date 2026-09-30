// src/endpoint_store/downstream_oauth.rs
//
// The in-flight half of three-legged OAuth: an authorization that has been
// started but not yet finished.
//
// The callback the provider redirects to is, necessarily, unauthenticated —
// it is a browser following a redirect, carrying no api0 session. So the only
// thing connecting that request back to a person is the `state` value we
// generated when we sent them out. That makes this table the CSRF defence, and
// the reason the callback can trust who it is acting for.
//
// Rows are single use and short lived. Redeeming deletes, so a replayed
// callback finds nothing.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use chrono::{Duration, Utc};
use rand::Rng;

/// Long enough that guessing is hopeless, short enough for a URL.
const TOKEN_BYTES: usize = 32;

/// An authorization should be completed in one sitting. Ten minutes is the same
/// budget the messaging link codes get, for the same reason: it bounds how long
/// a stolen redirect stays useful.
const REQUEST_TTL_MINUTES: i64 = 10;

pub struct PendingAuthorization {
    pub tenant_id: String,
    pub user_email: String,
    pub code_verifier: String,
}

fn random_token() -> String {
    use base64::Engine;
    let mut bytes = [0u8; TOKEN_BYTES];
    rand::rng().fill(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Start an authorization, returning `(state, code_verifier)`.
///
/// Both are generated here rather than by the caller so that neither can be
/// supplied from outside: a caller-chosen state would let someone fix the value
/// in advance, which is exactly the attack the state parameter exists to stop.
pub async fn start_authorization(
    store: &EndpointStore,
    tenant_id: &str,
    user_email: &str,
) -> Result<(String, String), StoreError> {
    let client = store.get_admin_conn().await?;

    // Sweep expired rows on the way in; these are short-lived by design and
    // nothing else ever looks at them.
    let _ = client
        .execute(
            "DELETE FROM downstream_oauth_requests WHERE expires_at < NOW()",
            &[],
        )
        .await;

    let state = random_token();
    let code_verifier = random_token();
    let expires_at = Utc::now() + Duration::minutes(REQUEST_TTL_MINUTES);

    client
        .execute(
            "INSERT INTO downstream_oauth_requests
                (state, tenant_id, user_email, code_verifier, expires_at)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &state,
                &tenant_id,
                &user_email.to_lowercase(),
                &code_verifier,
                &expires_at,
            ],
        )
        .await
        .to_store_error()?;

    app_log!(info, tenant_id = %tenant_id, "Started a downstream OAuth authorization");
    Ok((state, code_verifier))
}

/// Redeem a state value, returning who it was for.
///
/// The DELETE *is* the redemption: a state can be used once, so a replayed
/// callback — or a second tab finishing the same flow — finds nothing rather
/// than authorizing twice.
pub async fn redeem_authorization(
    store: &EndpointStore,
    state: &str,
) -> Result<Option<PendingAuthorization>, StoreError> {
    let client = store.get_admin_conn().await?;

    let row = client
        .query_opt(
            "DELETE FROM downstream_oauth_requests
             WHERE state = $1 AND expires_at > NOW()
             RETURNING tenant_id, user_email, code_verifier",
            &[&state],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| PendingAuthorization {
        tenant_id: r.get(0),
        user_email: r.get(1),
        code_verifier: r.get(2),
    }))
}
