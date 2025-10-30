use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
/// Deletes a single endpoint for a user
pub async fn delete_user_endpoint(
    store: &EndpointStore,
    email: &str,
    endpoint_id: &str,
) -> Result<bool, StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    app_log!(debug,
        email = %email,
        endpoint_id = %endpoint_id,
        "Starting endpoint deletion process"
    );

    // Check if user has access to this endpoint
    let user_endpoint_row = tx
        .query_opt(
            "SELECT 1 FROM user_endpoints WHERE email = $1 AND endpoint_id = $2",
            &[&email, &endpoint_id],
        )
        .await
        .to_store_error()?;

    if user_endpoint_row.is_none() {
        app_log!(debug,
            email = %email,
            endpoint_id = %endpoint_id,
            "User does not have access to this endpoint"
        );
        return Ok(false);
    }

    // Remove user-endpoint association
    tx.execute(
        "DELETE FROM user_endpoints WHERE email = $1 AND endpoint_id = $2",
        &[&email, &endpoint_id],
    )
    .await
    .to_store_error()?;

    // Check if any other user still uses this endpoint
    let still_used_row = tx
        .query_opt(
            "SELECT 1 FROM user_endpoints WHERE endpoint_id = $1",
            &[&endpoint_id],
        )
        .await
        .to_store_error()?;

    // If no other user uses this endpoint, delete it completely
    if still_used_row.is_none() {
        app_log!(debug,
            endpoint_id = %endpoint_id,
            "No other users reference this endpoint, deleting completely"
        );

        // Delete parameter alternatives
        tx.execute(
            "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
            &[&endpoint_id],
        )
        .await
        .to_store_error()?;

        // Delete parameters
        tx.execute(
            "DELETE FROM parameters WHERE endpoint_id = $1",
            &[&endpoint_id],
        )
        .await
        .to_store_error()?;

        // Delete the endpoint itself
        tx.execute("DELETE FROM endpoints WHERE id = $1", &[&endpoint_id])
            .await
            .to_store_error()?;
    }

    app_log!(info,
        email = %email,
        endpoint_id = %endpoint_id,
        "Endpoint successfully deleted"
    );

    tx.commit().await.to_store_error()?;
    Ok(true)
}
