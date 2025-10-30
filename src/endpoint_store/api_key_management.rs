use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::models::{ApiKeyInfo, KeyPreference};
use crate::endpoint_store::{EndpointStore, StoreError};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use rand::{rng, Rng};
use sha2::{Digest, Sha256};
use uuid::Uuid;
/// Generate a secure API key with the prefix "sk_live_"
pub fn generate_secure_key() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0));

    let timestamp = duration.as_nanos().to_string();
    let mut rng = rng();
    let random_number: u64 = rng.random();
    let combined = format!("{}{}", timestamp, random_number);

    let mut hasher = Sha256::new();
    hasher.update(combined.as_bytes());
    let hash = hasher.finalize();

    let base64_hash = URL_SAFE_NO_PAD.encode(hash);
    let key = &base64_hash[0..32];

    format!("sk_live_{}", key)
}

/// Hash the API key for secure storage
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let result = hasher.finalize();
    URL_SAFE_NO_PAD.encode(result)
}

/// Extract the key prefix for display purposes
pub fn extract_key_prefix(key: &str) -> String {
    let parts: Vec<&str> = key.split('_').collect();
    if parts.len() >= 3 {
        format!("sk_{}", &parts[2][..6])
    } else {
        format!("sk_{}", &key[7..13])
    }
}

/// Get API keys status for a user
pub async fn get_api_keys_status(
    store: &EndpointStore,
    email: &str,
) -> Result<KeyPreference, StoreError> {
    let client = store.get_conn().await?;

    app_log!(debug, email = %email, "Checking API keys status");

    let user_exists_row = client
        .query_opt("SELECT 1 FROM user_preferences WHERE email = $1", &[&email])
        .await
        .to_store_error()?;

    let user_exists = user_exists_row.is_some();

    app_log!(debug, email = %email, user_exists = user_exists, "User exists in preferences");

    if !user_exists {
        app_log!(info, email = %email, "Creating new user in preferences table");
        client
            .execute(
                "INSERT INTO user_preferences (email, hidden_defaults, credit_balance) VALUES ($1, '', 0)",
                &[&email],
            )
            .await
            .to_store_error()?;
    }

    let balance_row = client
        .query_one(
            "SELECT credit_balance FROM user_preferences WHERE email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;
    let balance: i64 = balance_row.get(0);

    app_log!(debug, email = %email, balance = balance, "Retrieved credit balance");

    let key_count_row = client
        .query_one(
            "SELECT COUNT(*) FROM api_keys WHERE email = $1 AND is_active = true",
            &[&email],
        )
        .await
        .to_store_error()?;
    let key_count: i64 = key_count_row.get(0);

    app_log!(debug, email = %email, active_key_count = key_count, "Found active API keys");

    if key_count == 0 {
        app_log!(info, email = %email, "No active API keys found - user is considered new");
        return Ok(KeyPreference {
            has_keys: false,
            active_key_count: 0,
            keys: vec![],
            balance,
        });
    }

    app_log!(info, email = %email, key_count = key_count, "User has active API keys");

    let rows = client
        .query(
            "SELECT id, key_prefix, key_name, generated_at, last_used
            FROM api_keys 
            WHERE email = $1 AND is_active = true 
            ORDER BY generated_at DESC",
            &[&email],
        )
        .await
        .to_store_error()?;

    let mut keys = Vec::new();
    for row in rows {
        keys.push(ApiKeyInfo {
            id: row.get(0),
            key_prefix: row.get(1),
            key_name: row.get(2),
            generated_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
            last_used: row
                .get::<_, Option<chrono::DateTime<chrono::Utc>>>(4)
                .map(|dt| dt.to_rfc3339()),
        });
    }

    app_log!(debug, email = %email, keys_found = keys.len(), "Retrieved API key details");

    Ok(KeyPreference {
        has_keys: true,
        active_key_count: key_count as usize,
        keys,
        balance,
    })
}

/// Revoke a specific API key
pub async fn revoke_api_key(
    store: &EndpointStore,
    email: &str,
    key_id: &str,
) -> Result<bool, StoreError> {
    let client = store.get_conn().await?;

    let key_exists_row = client
        .query_opt(
            "SELECT 1 FROM api_keys WHERE id = $1 AND email = $2 AND is_active = true",
            &[&key_id, &email],
        )
        .await
        .to_store_error()?;

    if key_exists_row.is_none() {
        return Ok(false);
    }

    client
        .execute(
            "UPDATE api_keys SET is_active = false WHERE id = $1 AND email = $2",
            &[&key_id, &email],
        )
        .await
        .to_store_error()?;

    Ok(true)
}

/// Revoke all API keys for a user
pub async fn revoke_all_api_keys(store: &EndpointStore, email: &str) -> Result<usize, StoreError> {
    let client = store.get_conn().await?;

    let affected = client
        .execute(
            "UPDATE api_keys SET is_active = false WHERE email = $1 AND is_active = true",
            &[&email],
        )
        .await
        .to_store_error()?;

    Ok(affected as usize)
}

/// Record API key usage
// pub async fn record_api_key_usage(store: &EndpointStore, key_id: &str) -> Result<(), StoreError> {
//     let client = store.get_conn().await?;
//     let now = Utc::now();
//
//     client
//         .execute(
//             "UPDATE api_keys SET
//              last_used = $1,
//              usage_count = usage_count + 1
//              WHERE id = $2 AND is_active = true",
//             &[&now, &key_id],
//         )
//         .await
//         .to_store_error()?;
//
//     Ok(())
// }

/// Validate an API key and return the key_id and email if valid
pub async fn validate_api_key(
    store: &EndpointStore,
    key: &str,
) -> Result<Option<(String, String)>, StoreError> {
    let client = store.get_conn().await?;
    let key_hash = hash_api_key(key);

    let row = client
        .query_opt(
            "SELECT id, email FROM api_keys WHERE key_hash = $1 AND is_active = true",
            &[&key_hash],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| (r.get(1), r.get(0))))
}

/// Get usage statistics for a specific API key
pub async fn get_api_key_usage(
    store: &EndpointStore,
    key_id: &str,
) -> Result<Option<ApiKeyInfo>, StoreError> {
    let client = store.get_conn().await?;

    let row = client
        .query_opt(
            "SELECT id, key_prefix, key_name, generated_at, last_used
             FROM api_keys 
             WHERE id = $1 AND is_active = true",
            &[&key_id],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| ApiKeyInfo {
        id: r.get(0),
        key_prefix: r.get(1),
        key_name: r.get(2),
        generated_at: r.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
        last_used: r
            .get::<_, Option<chrono::DateTime<chrono::Utc>>>(4)
            .map(|dt| dt.to_rfc3339()),
    }))
}

/// Update credit balance for a user
pub async fn update_credit_balance(
    store: &EndpointStore,
    email: &str,
    amount: i64,
) -> Result<i64, StoreError> {
    let client = store.get_conn().await?;

    let user_exists_row = client
        .query_opt("SELECT 1 FROM user_preferences WHERE email = $1", &[&email])
        .await
        .to_store_error()?;

    if user_exists_row.is_none() {
        client
            .execute(
                "INSERT INTO user_preferences (email, hidden_defaults, credit_balance) VALUES ($1, '', $2)",
                &[&email, &amount],
            )
            .await
            .to_store_error()?;

        return Ok(amount);
    }

    client
        .execute(
            "UPDATE user_preferences SET credit_balance = credit_balance + $1 WHERE email = $2",
            &[&amount, &email],
        )
        .await
        .to_store_error()?;

    let balance_row = client
        .query_one(
            "SELECT credit_balance FROM user_preferences WHERE email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    Ok(balance_row.get(0))
}

/// Get credit balance for a user
pub async fn get_credit_balance(store: &EndpointStore, email: &str) -> Result<i64, StoreError> {
    let client = store.get_conn().await?;

    let row = client
        .query_opt(
            "SELECT credit_balance FROM user_preferences WHERE email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| r.get(0)).unwrap_or(0))
}

/// Generate a new API key for a user
pub async fn generate_api_key(
    store: &EndpointStore,
    email: &str,
    key_name: &str,
) -> Result<(String, String, String), StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let new_key = generate_secure_key();
    let key_hash = hash_api_key(&new_key);
    let key_prefix = extract_key_prefix(&new_key);
    let now = Utc::now();
    let key_id = Uuid::new_v4().to_string();

    let user_exists_row = tx
        .query_opt("SELECT 1 FROM user_preferences WHERE email = $1", &[&email])
        .await
        .to_store_error()?;

    if user_exists_row.is_none() {
        tx.execute(
            "INSERT INTO user_preferences (email, hidden_defaults, credit_balance) VALUES ($1, '', 0)",
            &[&email],
        )
        .await
        .to_store_error()?;
    }

    tx.execute(
        "INSERT INTO api_keys (
            id, email, key_hash, key_prefix, key_name, 
            generated_at, usage_count, is_active
        ) VALUES ($1, $2, $3, $4, $5, $6, 0, true)",
        &[&key_id, &email, &key_hash, &key_prefix, &key_name, &now],
    )
    .await
    .to_store_error()?;

    tx.commit().await.to_store_error()?;

    Ok((new_key, key_prefix, key_id))
}
