// src/endpoint_store/channel_identities.rs
//
// Linking a messaging identity to an api0 person.
//
// The flow, from the person's side: sign in to the dashboard, ask for a code,
// send the code to the bot. From the bridge's side: an unlinked sender who sends
// something that looks like a code gets it redeemed; a linked sender gets their
// own api0 key, and every tool call runs as them.
//
// The key is minted here rather than shared: one key per (channel, identity,
// tenant), pinned to the tenant the channel belongs to. That is what gives a
// message from a phone the same attribution a Claude session has — the person's
// own downstream credentials, the person's own name on what they create.

use crate::app_log;
use crate::endpoint_store::api_key_management::generate_api_key_with_provider;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::user_credentials::require_tenant_access;
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::infra::secret_box::{self, SecretContext};
use chrono::{Duration, Utc};
use rand::Rng;
use serde::Serialize;

const CODE_TTL_MINUTES: i64 = 10;
const SECRET_PURPOSE: &str = "channel_api_key";

/// Six characters from an alphabet with no look-alikes. Typed on a phone, read
/// aloud, pasted into a chat — it has to survive all three.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

#[derive(Debug, Clone, Serialize)]
pub struct ChannelIdentity {
    pub channel: String,
    pub external_id: String,
    pub tenant_id: String,
    pub user_email: String,
    pub linked_at: String,
}

/// What the bridge needs to act as a linked person.
pub struct ResolvedIdentity {
    pub user_email: String,
    pub api_key: String,
}

pub fn new_code() -> String {
    let mut rng = rand::rng();
    (0..6)
        .map(|_| CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())] as char)
        .collect()
}

pub async fn create_link_code(
    store: &EndpointStore,
    user_email: &str,
) -> Result<(String, String), StoreError> {
    let client = store.get_admin_conn().await?;
    let email = user_email.to_lowercase();

    // Sweep anything stale on the way in; these are short-lived by design.
    let _ = client
        .execute("DELETE FROM channel_link_codes WHERE expires_at < NOW()", &[])
        .await;

    let code = new_code();
    let expires_at = Utc::now() + Duration::minutes(CODE_TTL_MINUTES);

    client
        .execute(
            "INSERT INTO channel_link_codes (code, user_email, expires_at) VALUES ($1, $2, $3)",
            &[&code, &email, &expires_at],
        )
        .await
        .to_store_error()?;

    Ok((code, expires_at.to_rfc3339()))
}

/// Redeem a code sent from a messaging identity, binding that identity to the
/// person who minted the code, in the tenant the channel belongs to.
pub async fn redeem_link_code(
    store: &EndpointStore,
    channel: &str,
    external_id: &str,
    tenant_id: &str,
    code: &str,
) -> Result<ChannelIdentity, StoreError> {
    if !secret_box::is_configured() {
        return Err(StoreError::InvalidInput(
            "API0_ENCRYPTION_KEY is not set — a linked key could not be protected".into(),
        ));
    }

    let client = store.get_admin_conn().await?;
    let code = code.trim().to_uppercase();

    // Single use: the DELETE is the redemption. A replayed code finds nothing.
    let row = client
        .query_opt(
            "DELETE FROM channel_link_codes
             WHERE code = $1 AND expires_at > NOW()
             RETURNING user_email",
            &[&code],
        )
        .await
        .to_store_error()?;

    let user_email: String = match row {
        Some(r) => r.get(0),
        None => {
            return Err(StoreError::InvalidInput(
                "That code is not valid — it may have expired. Ask for a new one in the dashboard.".into(),
            ))
        }
    };

    // The code proves who they are. Whether they may use *this* tenant is a
    // separate question, and it is the one that stops a code from one workspace
    // linking a phone into another.
    require_tenant_access(store, &user_email, tenant_id).await?;

    // One key per identity, so unlinking revokes exactly one thing.
    let key_name = format!("{} {}", channel, external_id);
    let (api_key, _prefix, key_id) = generate_api_key_with_provider(
        store,
        &user_email,
        &key_name,
        None,
        Some(tenant_id),
    )
    .await?;

    let sealed = secret_box::seal(&api_key, &SecretContext { tenant_id, purpose: SECRET_PURPOSE })
        .map_err(|e| StoreError::InvalidInput(format!("could not store the linked key: {}", e)))?;

    let row = client
        .query_one(
            "INSERT INTO channel_identities
                (channel, external_id, tenant_id, user_email, api_key_id, api_key_enc, linked_at)
             VALUES ($1, $2, $3, $4, $5, $6, NOW())
             ON CONFLICT (channel, external_id, tenant_id) DO UPDATE SET
                user_email  = EXCLUDED.user_email,
                api_key_id  = EXCLUDED.api_key_id,
                api_key_enc = EXCLUDED.api_key_enc,
                linked_at   = NOW()
             RETURNING channel, external_id, tenant_id, user_email, linked_at",
            &[&channel, &external_id, &tenant_id, &user_email, &key_id, &sealed],
        )
        .await
        .to_store_error()?;

    app_log!(info, channel = %channel, tenant_id = %tenant_id, "Linked a messaging identity");
    Ok(row_to_identity(row))
}

/// The person behind a messaging identity, with the key to act as them.
pub async fn resolve_identity(
    store: &EndpointStore,
    channel: &str,
    external_id: &str,
    tenant_id: &str,
) -> Result<Option<ResolvedIdentity>, StoreError> {
    let client = store.get_admin_conn().await?;

    let row = client
        .query_opt(
            "SELECT user_email, api_key_enc FROM channel_identities
             WHERE channel = $1 AND external_id = $2 AND tenant_id = $3",
            &[&channel, &external_id, &tenant_id],
        )
        .await
        .to_store_error()?;

    let row = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    let sealed: Vec<u8> = row.get(1);
    let api_key = secret_box::open(&sealed, &SecretContext { tenant_id, purpose: SECRET_PURPOSE })
        .map_err(|e| {
            app_log!(error, channel = %channel, error = %e, "Linked key would not open");
            StoreError::Database("linked key could not be read".into())
        })?;

    Ok(Some(ResolvedIdentity { user_email: row.get(0), api_key }))
}

pub async fn list_identities(
    store: &EndpointStore,
    user_email: &str,
) -> Result<Vec<ChannelIdentity>, StoreError> {
    let client = store.get_admin_conn().await?;
    let rows = client
        .query(
            "SELECT channel, external_id, tenant_id, user_email, linked_at
             FROM channel_identities WHERE user_email = $1 ORDER BY linked_at DESC",
            &[&user_email.to_lowercase()],
        )
        .await
        .to_store_error()?;
    Ok(rows.into_iter().map(row_to_identity).collect())
}

/// Unlink, and revoke the key that was minted for it.
pub async fn unlink_identity(
    store: &EndpointStore,
    user_email: &str,
    channel: &str,
    external_id: &str,
) -> Result<bool, StoreError> {
    let client = store.get_admin_conn().await?;

    let row = client
        .query_opt(
            "DELETE FROM channel_identities
             WHERE user_email = $1 AND channel = $2 AND external_id = $3
             RETURNING api_key_id",
            &[&user_email.to_lowercase(), &channel, &external_id],
        )
        .await
        .to_store_error()?;

    match row {
        Some(r) => {
            let key_id: String = r.get(0);
            let _ = client
                .execute("UPDATE api_keys SET is_active = false WHERE id = $1", &[&key_id])
                .await;
            Ok(true)
        }
        None => Ok(false),
    }
}

fn row_to_identity(row: tokio_postgres::Row) -> ChannelIdentity {
    ChannelIdentity {
        channel:     row.get(0),
        external_id: row.get(1),
        tenant_id:   row.get(2),
        user_email:  row.get(3),
        linked_at:   row.get::<_, chrono::DateTime<Utc>>(4).to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_is_six_unambiguous_characters() {
        for _ in 0..50 {
            let c = new_code();
            assert_eq!(c.len(), 6);
            // No 0/O or 1/I — the whole point of the alphabet.
            assert!(!c.contains(['0', 'O', '1', 'I']), "ambiguous char in {c}");
            assert!(c.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        }
    }

    #[test]
    fn codes_do_not_repeat() {
        let a = new_code();
        let b = new_code();
        assert_ne!(a, b);
    }
}
