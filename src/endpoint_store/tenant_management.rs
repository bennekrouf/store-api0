use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::endpoint_store::models::Tenant;
use uuid::Uuid;
use chrono::Utc;

pub async fn get_or_create_personal_tenant(
    store: &EndpointStore,
    email: &str,
) -> Result<Tenant, StoreError> {
    let client = store.get_conn().await?;

    // 1. Check if user has a default tenant
    let default_tenant_reow = client
        .query_opt(
            "SELECT t.id, t.name, t.credit_balance, t.created_at 
             FROM user_preferences up
             JOIN tenants t ON up.default_tenant_id = t.id
             WHERE up.email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    if let Some(row) = default_tenant_reow {
        return Ok(Tenant {
            id: row.get(0),
            name: row.get(1),
            credit_balance: row.get(2),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
        });
    }

    // 2. Check if user belongs to ANY tenant (if no default set)
    // For now, if no default is set, we prefer their PERSONAL tenant (named after email)
    // Check if a tenant with name == email exists AND user is a member
    // Actually, let's just create a personal tenant if they don't have one linked as default.
    // Or strictly: Create a new tenant.

    app_log!(info, email = %email, "Creating personal tenant for user");

    let tenant_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let name = email.to_string(); // Personal tenant name is email

    // We need a transaction
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    // 0. Ensure user exists in user_preferences
    // We check existence again inside transaction to be safe
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

    // Create Tenant
    tx.execute(
        "INSERT INTO tenants (id, name, credit_balance, created_at) VALUES ($1, $2, 0, $3)",
        &[&tenant_id, &name, &now],
    )
    .await
    .to_store_error()?;

    // Link User to Tenant
    tx.execute(
        "INSERT INTO tenant_users (tenant_id, email, role) VALUES ($1, $2, 'owner')",
        &[&tenant_id, &email],
    )
    .await
    .to_store_error()?;

    // Set as default
    tx.execute(
        "UPDATE user_preferences SET default_tenant_id = $1 WHERE email = $2",
        &[&tenant_id, &email],
    )
    .await
    .to_store_error()?;

    // MIGRATION: If user had credits in user_preferences, move them?
    // Let's assume user_preferences.credit_balance is the legacy source of truth.
    // We should move it.
    let old_balance_row = tx.query_one("SELECT credit_balance FROM user_preferences WHERE email = $1", &[&email]).await.to_store_error()?;
    let old_balance: i64 = old_balance_row.get(0);

    if old_balance > 0 {
        app_log!(info, email = %email, amount = old_balance, "Migrating legacy credits to new personal tenant");
        
        tx.execute(
            "UPDATE tenants SET credit_balance = credit_balance + $1 WHERE id = $2",
            &[&old_balance, &tenant_id]
        ).await.to_store_error()?;
        
        // Zero out legacy balance to prevent double spending during transition?
        // Or keep it for safety? Plan said "Data Migration".
        // Let's zero it out to enforce new source of truth.
        tx.execute("UPDATE user_preferences SET credit_balance = 0 WHERE email = $1", &[&email]).await.to_store_error()?;
    }

    tx.commit().await.to_store_error()?;

    Ok(Tenant {
        id: tenant_id,
        name,
        credit_balance: old_balance, // Approx
        created_at: now.to_rfc3339(),
    })
}

pub async fn get_default_tenant(
    store: &EndpointStore,
    email: &str,
) -> Result<Tenant, StoreError> {
    // This function ensures a tenant exists.
    // If it exists, return it.
    // If NOT, create it (Personal Tenant).
    
    // First ensure user exists in preferences (legacy requirement, but good for consistency)
    // We can rely on `get_api_keys_status` logic usually doing this, 
    // but here we should probably ensure it.
    
    // Logic:
    get_or_create_personal_tenant(store, email).await
}
