use crate::endpoint_store::{EndpointStore, StoreError};
use crate::endpoint_store::models::KeyPreference;
use crate::endpoint_store::db_helpers::ResultExt;
use chrono::Utc;
use rand::{rng, Rng};
// use rand::distributions::Alphanumeric;
use sha2::{Sha256, Digest};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

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

/// Get API key status for a user
pub async fn get_api_key_status(
    store: &EndpointStore,
    email: &str,
) -> Result<KeyPreference, StoreError> {
    let conn = store.get_conn().await?;
    
    // Check if the user has an API key
    let has_key: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM user_preferences WHERE email = ? AND api_key_hash IS NOT NULL)",
        [email],
        |row| row.get(0),
    ).unwrap_or(false);
    
    if !has_key {
        return Ok(KeyPreference {
            has_key: false,
            generated_at: None,
            last_used: None,
            usage_count: 0,
            key_name: None,
            key_prefix: None,
            balance: 0,
        });
    }
    
    // Get key details
    let (generated_at, last_used, usage_count, key_name, key_prefix, balance): (
        Option<String>,
        Option<String>,
        i64,
        Option<String>,
        Option<String>,
        i64,
    ) = conn.query_row(
        "SELECT api_key_generated_at, api_key_last_used, api_key_usage_count, api_key_name, api_key_prefix, credit_balance 
         FROM user_preferences 
         WHERE email = ?",
        [email],
        |row| Ok((
            row.get(0)?,
            row.get(1)?,
            row.get(2)?,
            row.get(3)?,
            row.get(4)?,
            row.get(5)?,
        )),
    ).unwrap_or((None, None, 0, None, None, 0));
    
    Ok(KeyPreference {
        has_key,
        generated_at,
        last_used,
        usage_count,
        key_name,
        key_prefix,
        balance,
    })
}

/// Generate a new API key for a user
pub async fn generate_api_key(
    store: &EndpointStore,
    email: &str,
    key_name: &str,
) -> Result<(String, String), StoreError> {
    let mut conn = store.get_conn().await?;
    let tx = conn.transaction().to_store_error()?;
    
    // Generate a new secure API key
    let new_key = generate_secure_key();
    let key_hash = hash_api_key(&new_key);
    let key_prefix = extract_key_prefix(&new_key);
    let now = Utc::now().to_rfc3339();
    
    // Check if the user exists in preferences
    let user_exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM user_preferences WHERE email = ?)",
        [email],
        |row| row.get(0),
    ).to_store_error()?;
    
    if user_exists {
        // Update the existing user preferences
        tx.execute(
            "UPDATE user_preferences SET 
             api_key_hash = ?, 
             api_key_prefix = ?, 
             api_key_name = ?, 
             api_key_generated_at = ?, 
             api_key_last_used = NULL, 
             api_key_usage_count = 0 
             WHERE email = ?",
            &[&key_hash, &key_prefix, key_name, &now, email],
        ).to_store_error()?;
    } else {
        // Insert new user preferences
        tx.execute(
            "INSERT INTO user_preferences (
                email, 
                hidden_defaults, 
                api_key_hash, 
                api_key_prefix, 
                api_key_name, 
                api_key_generated_at, 
                api_key_usage_count, 
                credit_balance
            ) VALUES (?, ?, ?, ?, ?, ?, 0, 0)",
            &[email, "", &key_hash, &key_prefix, key_name, &now],
        ).to_store_error()?;
    }
    
    tx.commit().to_store_error()?;
    
    Ok((new_key, key_prefix))
}

/// Revoke an API key for a user
pub async fn revoke_api_key(
    store: &EndpointStore,
    email: &str,
) -> Result<bool, StoreError> {
    let conn = store.get_conn().await?;
    
    // Check if user has a key
    let has_key: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM user_preferences WHERE email = ? AND api_key_hash IS NOT NULL)",
        [email],
        |row| row.get(0),
    ).to_store_error()?;
    
    if !has_key {
        return Ok(false);
    }
    
    // Update user preferences to remove the API key
    conn.execute(
        "UPDATE user_preferences SET 
         api_key_hash = NULL, 
         api_key_prefix = NULL, 
         api_key_name = NULL, 
         api_key_generated_at = NULL, 
         api_key_last_used = NULL, 
         api_key_usage_count = 0 
         WHERE email = ?",
        [email],
    ).to_store_error()?;
    
    Ok(true)
}

/// Record API key usage
pub async fn record_api_key_usage(
    store: &EndpointStore,
    email: &str,
) -> Result<(), StoreError> {
    let conn = store.get_conn().await?;
    let now = Utc::now().to_rfc3339();
    
    // Update the last used time and increment usage count
    conn.execute(
        "UPDATE user_preferences SET 
         api_key_last_used = ?, 
         api_key_usage_count = api_key_usage_count + 1 
         WHERE email = ? AND api_key_hash IS NOT NULL",
        &[&now, email],
    ).to_store_error()?;
    
    Ok(())
}

/// Validate an API key
pub async fn validate_api_key(
    store: &EndpointStore,
    key: &str,
) -> Result<Option<String>, StoreError> {
    let conn = store.get_conn().await?;
    let key_hash = hash_api_key(key);
    
    // Try to find a user with the matching API key hash
    let result: Result<String, _> = conn.query_row(
        "SELECT email FROM user_preferences WHERE api_key_hash = ?",
        [key_hash],
        |row| row.get(0),
    );
    
    match result {
        Ok(email) => Ok(Some(email)),
        Err(_) => Ok(None),
    }
}

/// Get usage statistics for a specific API key
pub async fn get_api_key_usage(
    store: &EndpointStore,
    email: &str,
) -> Result<KeyPreference, StoreError> {
    // For now, just return the key status which includes usage count
    get_api_key_status(store, email).await
}
