// src/endpoint_store/oauth_codes.rs
//
// Single use for the gateway's OAuth authorization codes.
//
// A code is a signed JWT and carries everything the exchange needs; the store
// holds only the fact that it has been spent. See `oauth_code_redemptions` in
// sql/schema.sql.

use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};

/// Record that the code `jti` has been exchanged.
///
/// Returns `true` the first time and `false` for every attempt after, so the
/// caller can refuse a replay. The check and the record are one statement:
/// two concurrent exchanges of the same code cannot both see "unused".
pub async fn redeem_code(store: &EndpointStore, jti: &str) -> Result<bool, StoreError> {
    if jti.trim().is_empty() {
        return Err(StoreError::InvalidInput("jti is required".into()));
    }

    let client = store.get_admin_conn().await?;

    // Opportunistic sweep. Codes expire after five minutes; a row older than
    // fifteen guards nothing.
    let _ = client
        .execute(
            "DELETE FROM oauth_code_redemptions WHERE redeemed_at < NOW() - INTERVAL '15 minutes'",
            &[],
        )
        .await;

    let inserted = client
        .execute(
            "INSERT INTO oauth_code_redemptions (jti) VALUES ($1)
             ON CONFLICT (jti) DO NOTHING",
            &[&jti],
        )
        .await
        .to_store_error()?;

    Ok(inserted == 1)
}

/// Against a real database, like the other SQL-backed rules:
///   TEST_DATABASE_URL=postgres://app:app@localhost:55432/app \
///     cargo test --bin store -- --ignored a_code_is_redeemed_once
#[cfg(test)]
mod db_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL"]
    async fn a_code_is_redeemed_once() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = EndpointStore::new(&url).await.expect("store");

        let jti = uuid::Uuid::new_v4().to_string();
        assert!(redeem_code(&store, &jti).await.unwrap(), "first exchange is allowed");
        assert!(!redeem_code(&store, &jti).await.unwrap(), "a replay is refused");

        let other = uuid::Uuid::new_v4().to_string();
        assert!(redeem_code(&store, &other).await.unwrap(), "codes do not share a slot");

        assert!(matches!(redeem_code(&store, " ").await, Err(StoreError::InvalidInput(_))));
    }
}
