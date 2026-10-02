// src/endpoint_store/workspace_access.rs
//
// Who may change an API group or endpoint: any member of the workspace it
// belongs to. This replaced "the email that created it" (the user_groups /
// user_endpoints link tables), which in a shared workspace let one member's
// uploads be invisible or untouchable to another, and let a delete remove only
// one person's link while the group lived on — tools included.

use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::tenant_management::MEMBER_ROLES;
use crate::endpoint_store::StoreError;

/// The workspace a group belongs to, if the group exists.
pub async fn group_tenant(
    tx: &deadpool_postgres::Transaction<'_>,
    group_id: &str,
) -> Result<Option<String>, StoreError> {
    Ok(tx
        .query_opt("SELECT tenant_id FROM api_groups WHERE id = $1", &[&group_id])
        .await
        .to_store_error()?
        .and_then(|row| row.get::<_, Option<String>>(0)))
}

/// The workspace an endpoint belongs to (through its group), if it exists.
pub async fn endpoint_tenant(
    tx: &deadpool_postgres::Transaction<'_>,
    endpoint_id: &str,
) -> Result<Option<String>, StoreError> {
    Ok(tx
        .query_opt(
            "SELECT g.tenant_id FROM endpoints e JOIN api_groups g ON g.id = e.group_id WHERE e.id = $1",
            &[&endpoint_id],
        )
        .await
        .to_store_error()?
        .and_then(|row| row.get::<_, Option<String>>(0)))
}

/// Is `email` a member (owner, admin or member — not a consumer) of `tenant_id`?
pub async fn is_member(
    tx: &deadpool_postgres::Transaction<'_>,
    email: &str,
    tenant_id: &str,
) -> Result<bool, StoreError> {
    Ok(tx
        .query_opt(
            "SELECT 1 FROM tenant_users
              WHERE tenant_id = $1 AND LOWER(email) = LOWER($2) AND role = ANY($3)",
            &[&tenant_id, &email, &MEMBER_ROLES],
        )
        .await
        .to_store_error()?
        .is_some())
}

/// Delete endpoints and everything hanging off them.
pub async fn delete_endpoints(
    tx: &deadpool_postgres::Transaction<'_>,
    endpoint_ids: &[String],
) -> Result<(), StoreError> {
    for sql in [
        "DELETE FROM parameter_alternatives WHERE endpoint_id = ANY($1)",
        "DELETE FROM parameters WHERE endpoint_id = ANY($1)",
        "DELETE FROM user_endpoints WHERE endpoint_id = ANY($1)",
        "DELETE FROM endpoints WHERE id = ANY($1)",
    ] {
        tx.execute(sql, &[&endpoint_ids]).await.to_store_error()?;
    }
    Ok(())
}
