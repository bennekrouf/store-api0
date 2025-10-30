use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
/// Deletes an API group and all its endpoints for a user
pub async fn delete_user_api_group(
    store: &EndpointStore,
    email: &str,
    group_id: &str,
) -> Result<bool, StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    app_log!(info,
        email = %email,
        group_id = %group_id,
        "Deleting API group"
    );

    // Get all endpoint IDs for this group
    let rows = tx
        .query(
            "SELECT e.id 
            FROM endpoints e
            INNER JOIN user_endpoints ue ON e.id = ue.endpoint_id
            WHERE ue.email = $1 AND e.group_id = $2",
            &[&email, &group_id],
        )
        .await
        .to_store_error()?;

    let endpoint_ids: Vec<String> = rows.iter().map(|row| row.get(0)).collect();

    // Remove user-group association
    tx.execute(
        "DELETE FROM user_groups WHERE email = $1 AND group_id = $2",
        &[&email, &group_id],
    )
    .await
    .to_store_error()?;

    // Remove user-endpoint associations
    for endpoint_id in &endpoint_ids {
        tx.execute(
            "DELETE FROM user_endpoints WHERE email = $1 AND endpoint_id = $2",
            &[&email, endpoint_id],
        )
        .await
        .to_store_error()?;
    }

    // Check if the group is still associated with any user
    let group_still_used_row = tx
        .query_opt(
            "SELECT 1 FROM user_groups WHERE group_id = $1",
            &[&group_id],
        )
        .await
        .to_store_error()?;

    // If no user is using this group anymore, delete it and its endpoints
    if group_still_used_row.is_none() {
        for endpoint_id in &endpoint_ids {
            let endpoint_still_used_row = tx
                .query_opt(
                    "SELECT 1 FROM user_endpoints WHERE endpoint_id = $1",
                    &[endpoint_id],
                )
                .await
                .to_store_error()?;

            if endpoint_still_used_row.is_none() {
                // Delete parameter alternatives
                tx.execute(
                    "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
                    &[endpoint_id],
                )
                .await
                .to_store_error()?;

                // Delete parameters
                tx.execute(
                    "DELETE FROM parameters WHERE endpoint_id = $1",
                    &[endpoint_id],
                )
                .await
                .to_store_error()?;

                // Delete endpoint
                tx.execute("DELETE FROM endpoints WHERE id = $1", &[endpoint_id])
                    .await
                    .to_store_error()?;
            }
        }

        // Delete the group itself
        tx.execute("DELETE FROM api_groups WHERE id = $1", &[&group_id])
            .await
            .to_store_error()?;
    }

    app_log!(info,
        email = %email,
        group_id = %group_id,
        endpoint_count = endpoint_ids.len(),
        "API group successfully deleted"
    );

    tx.commit().await.to_store_error()?;
    Ok(true)
}
