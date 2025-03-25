use crate::endpoint_store::{EndpointStore, StoreError};
use crate::endpoint_store::db_helpers::ResultExt;

/// Deletes an API group and all its endpoints for a user
pub async fn delete_user_api_group(
    store: &EndpointStore,
    email: &str,
    group_id: &str,
) -> Result<bool, StoreError> {
    let mut conn = store.get_conn()?;
    let tx = conn.transaction().to_store_error()?;

        tracing::info!(
            email = %email,
            group_id = %group_id,
            "Deleting API group"
        );

        // First, get all endpoint IDs for this group
        let mut stmt = tx.prepare(
            "SELECT e.id 
            FROM endpoints e
            JOIN user_endpoints ue ON e.id = ue.endpoint_id
            WHERE ue.email = ? AND e.group_id = ?",
        ).to_store_error()?;

        let endpoint_ids_iter = stmt
            .query_map([email, group_id], |row| row.get::<_, String>(0))
            .to_store_error()?; 

        // Collect the iterator of Results into a Vec
        let mut endpoint_ids = Vec::new();
        for id_result in endpoint_ids_iter {
            endpoint_ids.push(id_result.to_store_error()?);
        }

        // Remove user-group association
        tx.execute(
            "DELETE FROM user_groups WHERE email = ? AND group_id = ?",
            [email, group_id],
        ).to_store_error()?;

        // Remove user-endpoint associations for all endpoints in this group
        for endpoint_id in &endpoint_ids {
            tx.execute(
                "DELETE FROM user_endpoints WHERE email = ? AND endpoint_id = ?",
                [email, endpoint_id],
            ).to_store_error()?;
        }

        // Check if the group is still associated with any user
        let group_still_used: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM user_groups WHERE group_id = ?)",
            [group_id],
            |row| row.get(0),
        ).to_store_error()?;

        // If no user is using this group anymore, delete it and its endpoints
        if !group_still_used {
            // For each endpoint that's no longer used by any user, delete its data
            for endpoint_id in &endpoint_ids {
                let endpoint_still_used: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM user_endpoints WHERE endpoint_id = ?)",
                    [endpoint_id],
                    |row| row.get(0),
                ).to_store_error()?;

                if !endpoint_still_used {
                    // Delete parameter alternatives
                    tx.execute(
                        "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
                        [endpoint_id],
                    ).to_store_error()?;

                    // Delete parameters
                    tx.execute(
                        "DELETE FROM parameters WHERE endpoint_id = ?",
                        [endpoint_id],
                    ).to_store_error()?;

                    // Delete endpoint
                    tx.execute(
                        "DELETE FROM endpoints WHERE id = ? AND is_default = false",
                        [endpoint_id],
                    ).to_store_error()?;
                }
            }

            // Delete the group itself (if it's not a default group)
            tx.execute(
                "DELETE FROM api_groups WHERE id = ? AND is_default = false",
                [group_id],
            ).to_store_error()?;
        }

        tracing::info!(
            email = %email,
            group_id = %group_id,
            endpoint_count = endpoint_ids.len(),
            "API group successfully deleted"
        );

        tx.commit().to_store_error()?;
        Ok(true)
}
