use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::EndpointStore;
use crate::endpoint_store::StoreError;

/// Cleans up user data in a more conservative way (fallback)
pub async fn fallback_clean_user_data(
    store: &EndpointStore,
    email: &str,
) -> Result<(), StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    // Get endpoint IDs
    let rows = tx
        .query(
            "SELECT e.id 
            FROM endpoints e
            JOIN user_endpoints ue ON e.id = ue.endpoint_id
            WHERE ue.email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    let endpoint_ids: Vec<String> = rows.iter().map(|row| row.get(0)).collect();

    // Remove user-endpoint associations
    for id in &endpoint_ids {
        tx.execute(
            "DELETE FROM user_endpoints WHERE email = $1 AND endpoint_id = $2",
            &[&email, id],
        )
        .await
        .to_store_error()?;
    }

    // Check and clean up unused endpoints
    for id in &endpoint_ids {
        let still_used_row = tx
            .query_opt("SELECT 1 FROM user_endpoints WHERE endpoint_id = $1", &[id])
            .await
            .to_store_error()?;

        if still_used_row.is_none() {
            tx.execute(
                "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
                &[id],
            )
            .await
            .to_store_error()?;

            tx.execute("DELETE FROM parameters WHERE endpoint_id = $1", &[id])
                .await
                .to_store_error()?;

            tx.execute("DELETE FROM endpoints WHERE id = $1", &[id])
                .await
                .to_store_error()?;
        }
    }

    tx.commit().await.to_store_error()?;
    Ok(())
}

/// Forces a clean of user data
pub async fn force_clean_user_data(store: &EndpointStore, email: &str) -> Result<(), StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    // Get user's custom endpoints
    let rows = tx
        .query(
            "SELECT e.id 
            FROM endpoints e
            JOIN user_endpoints ue ON e.id = ue.endpoint_id
            WHERE ue.email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    let endpoint_ids: Vec<String> = rows.iter().map(|row| row.get(0)).collect();

    // Delete parameter alternatives
    for id in &endpoint_ids {
        tx.execute(
            "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
            &[id],
        )
        .await
        .to_store_error()?;
    }

    // Delete parameters
    for id in &endpoint_ids {
        tx.execute("DELETE FROM parameters WHERE endpoint_id = $1", &[id])
            .await
            .to_store_error()?;
    }

    // Delete user endpoint associations
    tx.execute("DELETE FROM user_endpoints WHERE email = $1", &[&email])
        .await
        .to_store_error()?;

    // Delete endpoints
    // ... existing endpoint deletion logic ...
    for id in &endpoint_ids {
        let still_referenced = tx
            .query_opt("SELECT 1 FROM user_endpoints WHERE endpoint_id = $1", &[id])
            .await
            .to_store_error()?;

        if still_referenced.is_none() {
            tx.execute("DELETE FROM endpoints WHERE id = $1", &[id])
                .await
                .to_store_error()?;
        }
    }

    // NEW: Clean up tenant/key/preference data
    
    // 1. Delete API usage logs
    tx.execute("DELETE FROM api_usage_logs WHERE email = $1", &[&email]).await.to_store_error()?;
    
    // 2. Delete API keys
    tx.execute("DELETE FROM api_keys WHERE email = $1", &[&email]).await.to_store_error()?;
    
    // 3. Delete Tenant Users
    tx.execute("DELETE FROM tenant_users WHERE email = $1", &[&email]).await.to_store_error()?;
    
    // 4. Delete Personal Tenant (if name == email)
    // We first need to delete any remaining groups linked to this tenant? 
    // Assuming personal tenant only has this user's stuff.
    // If we delete tenant, we need to handle constraints.
    // Let's identify the tenant first.
    let tenant_rows = tx.query("SELECT id FROM tenants WHERE name = $1", &[&email]).await.to_store_error()?;
    for row in tenant_rows {
        let tenant_id: String = row.get(0);
        // Delete logs for tenant (redundant if by email, but covers edge cases)
        tx.execute("DELETE FROM api_usage_logs WHERE tenant_id = $1", &[&tenant_id]).await.to_store_error()?;
        // Delete keys for tenant
        tx.execute("DELETE FROM api_keys WHERE tenant_id = $1", &[&tenant_id]).await.to_store_error()?;
        // Delete groups for tenant
        tx.execute("DELETE FROM api_groups WHERE tenant_id = $1", &[&tenant_id]).await.to_store_error()?;
        // Delete tenant_users again? (FK should cascade or we did it above)
        tx.execute("DELETE FROM tenant_users WHERE tenant_id = $1", &[&tenant_id]).await.to_store_error()?;
        
        // Finally delete tenant
        tx.execute("DELETE FROM tenants WHERE id = $1", &[&tenant_id]).await.to_store_error()?;
    }

    // 5. Delete User Preferences
    tx.execute("DELETE FROM user_preferences WHERE email = $1", &[&email]).await.to_store_error()?;

    tx.commit().await.to_store_error()?;
    Ok(())
}

