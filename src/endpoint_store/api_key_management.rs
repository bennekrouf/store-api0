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

    // Get current time with nanosecond precision
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0));

    // Create a unique string from the timestamp and a random part
    let timestamp = duration.as_nanos().to_string();

    // Add some randomness
    let mut rng = rng();
    let random_number: u64 = rng.random();
    let combined = format!("{}{}", timestamp, random_number);

    // Create a hash of the combined string
    let mut hasher = Sha256::new();
    hasher.update(combined.as_bytes());
    let hash = hasher.finalize();

    // Encode the hash and take first 32 characters
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
    // Take the first 8 characters of the key after the prefix
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
    let conn = store.get_conn().await?;

    tracing::debug!(email = %email, "Checking API keys status");

    // First, check if user exists in preferences
    let user_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM user_preferences WHERE email = ?)",
            [email],
            |row| row.get(0),
        )
        .unwrap_or(false);

    tracing::debug!(email = %email, user_exists = user_exists, "User exists in preferences");

    if !user_exists {
        // Create user if not exists
        tracing::info!(email = %email, "Creating new user in preferences table");
        conn.execute(
            "INSERT INTO user_preferences (email, hidden_defaults, credit_balance) VALUES (?, '', 0)",
            [email],
        ).to_store_error()?;
    }

    // Get credit balance
    let balance: i64 = conn
        .query_row(
            "SELECT credit_balance FROM user_preferences WHERE email = ?",
            [email],
            |row| row.get(0),
        )
        .unwrap_or(0);

    tracing::debug!(email = %email, balance = balance, "Retrieved credit balance");

    // Check if the user has any ACTIVE API keys
    let key_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM api_keys WHERE email = ? AND is_active = true",
            [email],
            |row| row.get(0),
        )
        .unwrap_or(0);

    tracing::debug!(email = %email, active_key_count = key_count, "Found active API keys");

    if key_count == 0 {
        tracing::info!(email = %email, "No active API keys found - user is considered new");
        return Ok(KeyPreference {
            has_keys: false,
            active_key_count: 0,
            keys: vec![],
            balance,
        });
    }

    tracing::info!(email = %email, key_count = key_count, "User has active API keys");

    // Get all active keys
    let mut stmt = conn
        .prepare(
            "SELECT 
                id, 
                key_prefix, 
                key_name, 
                generated_at, 
                COALESCE(last_used, '') as last_used_str, 
                usage_count 
            FROM api_keys 
            WHERE email = ? AND is_active = true 
            ORDER BY generated_at DESC",
        )
        .to_store_error()?;

    let keys_iter = stmt
        .query_map([email], |row| {
            Ok(ApiKeyInfo {
                id: row.get(0)?,
                key_prefix: row.get(1)?,
                key_name: row.get(2)?,
                generated_at: row.get(3)?,
                last_used: row.get::<_, String>(4).ok().filter(|s| !s.is_empty()),
                usage_count: row.get(5)?,
            })
        })
        .to_store_error()?;

    let mut keys = Vec::new();
    for key_result in keys_iter {
        keys.push(key_result.to_store_error()?);
    }

    tracing::debug!(email = %email, keys_found = keys.len(), "Retrieved API key details");

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
    let conn = store.get_conn().await?;

    // Check if key exists and belongs to user
    let key_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM api_keys WHERE id = ? AND email = ? AND is_active = true)",
            [key_id, email],
            |row| row.get(0),
        )
        .to_store_error()?;

    if !key_exists {
        return Ok(false);
    }

    // Deactivate the key
    conn.execute(
        "UPDATE api_keys SET is_active = false WHERE id = ? AND email = ?",
        [key_id, email],
    )
    .to_store_error()?;

    Ok(true)
}

/// Revoke all API keys for a user
pub async fn revoke_all_api_keys(store: &EndpointStore, email: &str) -> Result<usize, StoreError> {
    let conn = store.get_conn().await?;

    // Deactivate all keys for the user
    let affected = conn
        .execute(
            "UPDATE api_keys SET is_active = false WHERE email = ? AND is_active = true",
            [email],
        )
        .to_store_error()?;

    Ok(affected as usize)
}

/// Record API key usage
pub async fn record_api_key_usage(store: &EndpointStore, key_id: &str) -> Result<(), StoreError> {
    let conn = store.get_conn().await?;
    let now = Utc::now().to_rfc3339();

    // Update the last used time and increment usage count
    conn.execute(
        "UPDATE api_keys SET 
         last_used = ?, 
         usage_count = usage_count + 1 
         WHERE id = ? AND is_active = true",
        &[&now, key_id],
    )
    .to_store_error()?;

    Ok(())
}

/// Validate an API key and return the key_id and email if valid
pub async fn validate_api_key(
    store: &EndpointStore,
    key: &str,
) -> Result<Option<(String, String)>, StoreError> {
    let conn = store.get_conn().await?;
    let key_hash = hash_api_key(key);

    // Try to find a key with the matching hash
    let result: Result<(String, String), _> = conn.query_row(
        "SELECT id, email FROM api_keys WHERE key_hash = ? AND is_active = true",
        [key_hash],
        |row| Ok((row.get(0)?, row.get(1)?)),
    );

    match result {
        Ok((key_id, email)) => Ok(Some((key_id, email))),
        Err(_) => Ok(None),
    }
}

/// Get usage statistics for a specific API key
pub async fn get_api_key_usage(
    store: &EndpointStore,
    key_id: &str,
) -> Result<Option<ApiKeyInfo>, StoreError> {
    let conn = store.get_conn().await?;

    let result: Result<ApiKeyInfo, _> = conn.query_row(
        "SELECT id, key_prefix, key_name, generated_at, last_used, usage_count 
         FROM api_keys 
         WHERE id = ? AND is_active = true",
        [key_id],
        |row| {
            Ok(ApiKeyInfo {
                id: row.get(0)?,
                key_prefix: row.get(1)?,
                key_name: row.get(2)?,
                generated_at: row.get(3)?,
                last_used: row.get(4)?,
                usage_count: row.get(5)?,
            })
        },
    );

    match result {
        Ok(info) => Ok(Some(info)),
        Err(_) => Ok(None),
    }
}

/// Update credit balance for a user
pub async fn update_credit_balance(
    store: &EndpointStore,
    email: &str,
    amount: i64,
) -> Result<i64, StoreError> {
    let conn = store.get_conn().await?;

    // Check if user exists
    let user_exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM user_preferences WHERE email = ?)",
            [email],
            |row| row.get(0),
        )
        .to_store_error()?;

    if !user_exists {
        // Create user if not exists
        conn.execute(
            "INSERT INTO user_preferences (email, hidden_defaults, credit_balance) VALUES (?, '', ?)",
            duckdb::params![email, amount], // Use duckdb::params! for type safety
        ).to_store_error()?;

        return Ok(amount);
    }

    // Update credit balance
    conn.execute(
        "UPDATE user_preferences SET credit_balance = credit_balance + ? WHERE email = ?",
        duckdb::params![amount, email], // Use duckdb::params! macro
    )
    .to_store_error()?;

    // Get updated balance
    let new_balance: i64 = conn
        .query_row(
            "SELECT credit_balance FROM user_preferences WHERE email = ?",
            [email],
            |row| row.get(0),
        )
        .to_store_error()?;

    Ok(new_balance)
}

/// Get credit balance for a user
pub async fn get_credit_balance(store: &EndpointStore, email: &str) -> Result<i64, StoreError> {
    let conn = store.get_conn().await?;

    let balance: i64 = conn
        .query_row(
            "SELECT credit_balance FROM user_preferences WHERE email = ?",
            [email],
            |row| row.get(0),
        )
        .unwrap_or(0);

    Ok(balance)
}

/// Generate a new API key for a user (updated to handle credits properly)
pub async fn generate_api_key(
    store: &EndpointStore,
    email: &str,
    key_name: &str,
) -> Result<(String, String, String), StoreError> {
    let mut conn = store.get_conn().await?;
    let tx = conn.transaction().to_store_error()?;

    // Generate a new secure API key
    let new_key = generate_secure_key();
    let key_hash = hash_api_key(&new_key);
    let key_prefix = extract_key_prefix(&new_key);
    let now = Utc::now().to_rfc3339();
    let key_id = Uuid::new_v4().to_string();

    // Check if the user exists in preferences
    let user_exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM user_preferences WHERE email = ?)",
            [email],
            |row| row.get(0),
        )
        .to_store_error()?;

    if !user_exists {
        // Create user if not exists with default credit balance
        tx.execute(
            "INSERT INTO user_preferences (email, hidden_defaults, credit_balance) VALUES (?, '', ?)",
            duckdb::params![email, 0i64], // Explicitly specify as i64
        ).to_store_error()?;
    }

    // Insert the new API key
    tx.execute(
        "INSERT INTO api_keys (
            id, 
            email, 
            key_hash, 
            key_prefix, 
            key_name, 
            generated_at, 
            usage_count,
            is_active
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        duckdb::params![key_id, email, key_hash, key_prefix, key_name, now, 0i64, true],
    )
    .to_store_error()?;

    tx.commit().to_store_error()?;

    Ok((new_key, key_prefix, key_id))
}
