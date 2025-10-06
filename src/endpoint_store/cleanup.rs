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

    // Delete endpoints that are no longer referenced
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

    tx.commit().await.to_store_error()?;
    Ok(())
}

