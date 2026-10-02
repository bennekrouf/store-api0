use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::workspace_access::{delete_endpoints, endpoint_tenant, is_member};
use crate::endpoint_store::{EndpointStore, StoreError};

/// Deletes an endpoint, for any member of the workspace it belongs to.
///
/// `Ok(false)` when it does not exist or the caller is not a member — the same
/// answer for both, so a caller cannot probe another workspace's ids.
pub async fn delete_user_endpoint(
    store: &EndpointStore,
    email: &str,
    endpoint_id: &str,
) -> Result<bool, StoreError> {
    let mut client = store.get_admin_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let Some(tenant_id) = endpoint_tenant(&tx, endpoint_id).await? else {
        return Ok(false);
    };
    if !is_member(&tx, email, &tenant_id).await? {
        app_log!(warn, email = %email, endpoint_id = %endpoint_id, tenant_id = %tenant_id,
            "Refused to delete an endpoint outside the caller's workspaces");
        return Ok(false);
    }

    delete_endpoints(&tx, &[endpoint_id.to_string()]).await?;
    tx.commit().await.to_store_error()?;
    app_log!(info, email = %email, endpoint_id = %endpoint_id, tenant_id = %tenant_id, "Endpoint deleted");

    // The endpoint's tool goes with it.
    crate::endpoint_store::mcp_tools_management::resync_tenant_tools(store, &tenant_id).await;
    Ok(true)
}
