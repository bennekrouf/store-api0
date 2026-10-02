use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::workspace_access::{delete_endpoints, group_tenant, is_member};
use crate::endpoint_store::{EndpointStore, StoreError};

/// Deletes an API group and all its endpoints, for any member of its workspace.
///
/// It used to remove only the caller's own link to the group, deleting it only
/// once nobody else was linked — so in a shared workspace a "deleted" group
/// lived on, and its tools stayed callable from Claude. `Ok(false)` when it
/// does not exist or the caller is not a member.
pub async fn delete_user_api_group(
    store: &EndpointStore,
    email: &str,
    group_id: &str,
) -> Result<bool, StoreError> {
    let mut client = store.get_admin_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let Some(tenant_id) = group_tenant(&tx, group_id).await? else {
        return Ok(false);
    };
    if !is_member(&tx, email, &tenant_id).await? {
        app_log!(warn, email = %email, group_id = %group_id, tenant_id = %tenant_id,
            "Refused to delete a group outside the caller's workspaces");
        return Ok(false);
    }

    let endpoint_ids: Vec<String> = tx
        .query("SELECT id FROM endpoints WHERE group_id = $1", &[&group_id])
        .await
        .to_store_error()?
        .iter()
        .map(|row| row.get(0))
        .collect();
    delete_endpoints(&tx, &endpoint_ids).await?;
    tx.execute("DELETE FROM user_groups WHERE group_id = $1", &[&group_id])
        .await
        .to_store_error()?;
    tx.execute("DELETE FROM api_groups WHERE id = $1", &[&group_id])
        .await
        .to_store_error()?;
    tx.commit().await.to_store_error()?;

    app_log!(info, email = %email, group_id = %group_id, tenant_id = %tenant_id,
        endpoint_count = endpoint_ids.len(), "API group deleted");

    // Its tools are switched off in the same step.
    crate::endpoint_store::mcp_tools_management::resync_tenant_tools(store, &tenant_id).await;
    Ok(true)
}
