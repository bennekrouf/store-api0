use crate::endpoint_store:: StoreError;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::EndpointStore;

/// Cleans up user data in a more conservative way (fallback)
pub fn fallback_clean_user_data(
    store: &EndpointStore,
    email: &str,
) -> Result<(), StoreError> {
    let mut conn = store.get_conn()?;
    let tx = conn.transaction().to_store_error()?;

    let mut stmt = tx.prepare(
        "SELECT e.id 
        FROM endpoints e
        JOIN user_endpoints ue ON e.id = ue.endpoint_id
        WHERE ue.email = ? AND e.is_default = false",
    ).to_store_error()?;

    let endpoint_ids: Vec<String> = stmt
        .query_map([email], |row| row.get(0))
        .to_store_error()?
        .collect::<Result<Vec<String>, _>>()
        .to_store_error()?;

    // Remove user-endpoint associations
    for id in &endpoint_ids {
        let _ = tx.execute(
            "DELETE FROM user_endpoints WHERE email = ? AND endpoint_id = ?",
            &[email, id],
        ).to_store_error()?;
    }

    // Now check which endpoints are no longer used
    for id in &endpoint_ids {
        let still_used: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM user_endpoints WHERE endpoint_id = ?)",
            [id],
            |row| row.get(0),
        ).to_store_error()?;

        if !still_used {
            // Remove parameter alternatives first
            let _ = tx.execute(
                "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
                [id],
            ).to_store_error()?;

            // Then remove parameters
            let _ = tx.execute("DELETE FROM parameters WHERE endpoint_id = ?", [id]).to_store_error()?;

            // Finally remove the endpoint
            let _ = tx.execute("DELETE FROM endpoints WHERE id = ?", [id]).to_store_error()?;
        }
    }

    tx.commit().to_store_error()?;
    Ok(())
}

/// Forces a clean of user data by disabling foreign keys temporarily
pub fn force_clean_user_data(
    store: &EndpointStore,
    email: &str,
) -> Result<(), StoreError> {
    let mut conn = store.get_conn()?;
    let tx = conn.transaction().to_store_error()?;

        // First turn off foreign keys
        tx.execute("PRAGMA foreign_keys=OFF;", []).to_store_error()?;

        // Create a temporary table to track user's custom endpoints
        tx.execute(
            "CREATE TEMPORARY TABLE user_custom_endpoints AS
            SELECT e.id 
            FROM endpoints e
            JOIN user_endpoints ue ON e.id = ue.endpoint_id
            WHERE ue.email = ? AND e.is_default = false",
            [email],
        ).to_store_error()?;

        // Delete parameter alternatives
        tx.execute(
            "DELETE FROM parameter_alternatives 
            WHERE endpoint_id IN (SELECT id FROM user_custom_endpoints)",
            [],
        ).to_store_error()?;

        // Delete parameters
        tx.execute(
            "DELETE FROM parameters
            WHERE endpoint_id IN (SELECT id FROM user_custom_endpoints)",
            [],
        ).to_store_error()?;

        // Delete user endpoint associations
        tx.execute("DELETE FROM user_endpoints WHERE email = ?", [email]).to_store_error()?;

        // Delete endpoints that are no longer referenced and not default
        tx.execute(
            "DELETE FROM endpoints 
            WHERE id IN (
                SELECT id FROM user_custom_endpoints
                WHERE id NOT IN (SELECT endpoint_id FROM user_endpoints)
            )",
            [],
        ).to_store_error()?;

        // Clean up temporary table
        tx.execute("DROP TABLE user_custom_endpoints", []).to_store_error()?;

        // Turn foreign keys back on
        tx.execute("PRAGMA foreign_keys=ON;", []).to_store_error()?;

        tx.commit().to_store_error()?;
        Ok(())
}
