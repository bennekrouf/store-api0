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
    use crate::endpoint_store::tenant_management;

    app_log!(debug, email = %email, "Checking API keys status");

    // Read balance from the tenant (same source as get_credit_balance / header widget)
    let tenant = tenant_management::get_default_tenant(store, email).await?;
    let balance = tenant.credit_balance;

    app_log!(debug, email = %email, balance = balance, "Retrieved credit balance from tenant");

    let client = store.get_conn().await?;

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
            "SELECT id, key_prefix, key_name, generated_at, last_used, usage_count
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
            usage_count: row.get::<_, i64>(5),
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
            "SELECT id, key_prefix, key_name, generated_at, last_used, usage_count
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
        usage_count: r.get::<_, i64>(5),
    }))
}

pub async fn update_credit_balance(
    store: &EndpointStore,
    email: &str,
    amount: i64,
    action_type: &str,
    description: Option<&str>,
) -> Result<i64, StoreError> {
    use crate::endpoint_store::tenant_management;

    let tenant = tenant_management::get_default_tenant(store, email).await?;
    let tenant_id = tenant.id;

    let client = store.get_conn().await?;

    client
        .execute(
            "UPDATE tenants SET credit_balance = credit_balance + $1 WHERE id = $2",
            &[&amount, &tenant_id],
        )
        .await
        .to_store_error()?;

    let balance_row = client
        .query_one(
            "SELECT credit_balance FROM tenants WHERE id = $1",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    let new_balance: i64 = balance_row.get(0);

    let desc: Option<String> = description.map(|s| s.to_string());
    if let Err(e) = client
        .execute(
            "INSERT INTO credit_transactions \
             (tenant_id, email, amount, balance_after, action_type, description) \
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[&tenant_id, &email, &amount, &new_balance, &action_type, &desc],
        )
        .await
    {
        app_log!(error,
            email = %email,
            action_type = %action_type,
            amount = amount,
            "Failed to record credit transaction: {}", e
        );
    }

    Ok(new_balance)
}

pub async fn get_credit_transactions(
    store: &EndpointStore,
    email: &str,
    limit: i64,
) -> Result<Vec<crate::endpoint_store::models::CreditTransaction>, StoreError> {
    use crate::endpoint_store::tenant_management;

    let tenant = tenant_management::get_default_tenant(store, email).await?;
    let tenant_id = tenant.id;
    let client = store.get_conn().await?;

    let rows = client
        .query(
            "SELECT id, tenant_id, email, amount, balance_after, action_type, description, created_at \
             FROM credit_transactions \
             WHERE tenant_id = $1 \
             ORDER BY created_at DESC \
             LIMIT $2",
            &[&tenant_id, &limit],
        )
        .await
        .to_store_error()?;

    Ok(rows.iter().map(|row| {
        crate::endpoint_store::models::CreditTransaction {
            id: row.get::<_, i64>(0),
            tenant_id: row.get::<_, String>(1),
            email: row.get::<_, String>(2),
            amount: row.get::<_, i64>(3),
            balance_after: row.get::<_, i64>(4),
            action_type: row.get::<_, String>(5),
            description: row.get::<_, Option<String>>(6),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(7).to_rfc3339(),
        }
    }).collect())
}

/// Get credit balance for a user (via their default tenant)
pub async fn get_credit_balance(store: &EndpointStore, email: &str) -> Result<i64, StoreError> {
    use crate::endpoint_store::tenant_management;
    
    // Resolve tenant (this migrates legacy credits if needed)
    let tenant = tenant_management::get_default_tenant(store, email).await?;
    
    // We can rely on the returned tenant object having the balance, 
    // BUT since we might want the *latest* balance if `get_default_tenant` returned a cached object (it doesn't, but still),
    // Querying fresh is safer, or just use the returned value since `get_default_tenant` fetches it.
    // The `get_default_tenant` implementation fetches fresh data.
    Ok(tenant.credit_balance)
}

/// Generate a new API key for a user (associated with their default tenant)
pub async fn generate_api_key(
    store: &EndpointStore,
    email: &str,
    key_name: &str,
) -> Result<(String, String, String), StoreError> {
    use crate::endpoint_store::tenant_management;

    // Ensure tenant exists (this handles user_preferences creation too)
    let tenant = tenant_management::get_default_tenant(store, email).await?;
    let tenant_id = tenant.id;

    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let new_key = generate_secure_key();
    let key_hash = hash_api_key(&new_key);
    let key_prefix = extract_key_prefix(&new_key);
    let now = Utc::now();
    let key_id = Uuid::new_v4().to_string();

    tx.execute(
        "INSERT INTO api_keys (
            id, email, key_hash, key_prefix, key_name, 
            generated_at, usage_count, is_active, tenant_id
        ) VALUES ($1, $2, $3, $4, $5, $6, 0, true, $7)",
        &[&key_id, &email, &key_hash, &key_prefix, &key_name, &now, &tenant_id],
    )
    .await
    .to_store_error()?;

    tx.commit().await.to_store_error()?;

    Ok((new_key, key_prefix, key_id))
}
