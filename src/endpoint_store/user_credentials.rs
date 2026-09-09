// src/endpoint_store/user_credentials.rs
//
// CRUD for user_downstream_credentials — the per-user secret a tool call
// authenticates with, so Azure DevOps records the person who asked rather than
// whoever owns a shared token.
//
// Everything here goes through infra::secret_box. A plaintext secret exists in
// this module only between the argument and the INSERT, and between the SELECT
// and the return value; it is never logged and never stored.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::infra::secret_box::{self, SecretContext};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// What a credential is, without what it *is*. This is the shape the dashboard
/// gets back: enough to manage a token, never enough to use one.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialSummary {
    pub tenant_id: String,
    pub user_email: String,
    pub kind: String,
    pub label: String,
    pub expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveCredentialRequest {
    /// 'pat' today; 'entra_refresh' when federation lands.
    pub kind: Option<String>,
    pub secret: String,
    pub label: Option<String>,
    /// RFC 3339. Advisory — what the user says the provider will enforce.
    pub expires_at: Option<String>,
}

pub async fn save_credential(
    store: &EndpointStore,
    tenant_id: &str,
    user_email: &str,
    req: &SaveCredentialRequest,
) -> Result<CredentialSummary, StoreError> {
    let kind = req.kind.as_deref().unwrap_or("pat").to_string();
    let secret = req.secret.trim();
    if secret.is_empty() {
        return Err(StoreError::InvalidInput("secret must not be empty".into()));
    }

    let sealed = secret_box::seal(secret, &SecretContext { tenant_id, purpose: &kind })
        .map_err(|e| {
            // The message names the failure, never the secret.
            app_log!(error, tenant_id = %tenant_id, error = %e, "Could not seal a user credential");
            StoreError::InvalidInput(format!("could not store the credential: {}", e))
        })?;

    let expires_at = match req.expires_at.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => Some(
            DateTime::parse_from_rfc3339(raw)
                .map_err(|e| StoreError::InvalidInput(format!("expires_at is not RFC 3339: {}", e)))?
                .with_timezone(&Utc),
        ),
        None => None,
    };

    let label = req.label.as_deref().unwrap_or("").to_string();
    let user_email = user_email.to_lowercase();
    let client = store.get_conn(Some(tenant_id)).await?;

    let row = client
        .query_one(
            "INSERT INTO user_downstream_credentials
                (tenant_id, user_email, kind, secret, label, expires_at, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, NOW(), NOW())
             ON CONFLICT (tenant_id, user_email, kind) DO UPDATE SET
                secret     = EXCLUDED.secret,
                label      = EXCLUDED.label,
                expires_at = EXCLUDED.expires_at,
                updated_at = NOW()
             RETURNING tenant_id, user_email, kind, label, expires_at, created_at, updated_at",
            &[
                &tenant_id as &(dyn tokio_postgres::types::ToSql + Sync),
                &user_email as &(dyn tokio_postgres::types::ToSql + Sync),
                &kind as &(dyn tokio_postgres::types::ToSql + Sync),
                &sealed as &(dyn tokio_postgres::types::ToSql + Sync),
                &label as &(dyn tokio_postgres::types::ToSql + Sync),
                &expires_at as &(dyn tokio_postgres::types::ToSql + Sync),
            ],
        )
        .await
        .to_store_error()?;

    app_log!(info, tenant_id = %tenant_id, kind = %kind, "Stored a per-user downstream credential");
    Ok(row_to_summary(row))
}

/// The credential itself, decrypted. The gateway is the only caller.
pub async fn get_secret(
    store: &EndpointStore,
    tenant_id: &str,
    user_email: &str,
    kind: &str,
) -> Result<Option<String>, StoreError> {
    let user_email = user_email.to_lowercase();
    let client = store.get_conn(Some(tenant_id)).await?;

    let row = client
        .query_opt(
            "SELECT secret FROM user_downstream_credentials
             WHERE tenant_id = $1 AND user_email = $2 AND kind = $3",
            &[&tenant_id, &user_email, &kind],
        )
        .await
        .to_store_error()?;

    let sealed: Vec<u8> = match row {
        Some(r) => r.get(0),
        None => return Ok(None),
    };

    // A record that will not open is not a missing record: someone rotated the
    // key or altered the row, and saying "not found" would send the user off to
    // paste a new token that would fail the same way.
    secret_box::open(&sealed, &SecretContext { tenant_id, purpose: kind })
        .map(Some)
        .map_err(|e| {
            app_log!(error, tenant_id = %tenant_id, kind = %kind, error = %e, "Stored credential would not open");
            StoreError::Database(format!("stored credential could not be read: {}", e))
        })
}

pub async fn list_credentials(
    store: &EndpointStore,
    tenant_id: &str,
    user_email: &str,
) -> Result<Vec<CredentialSummary>, StoreError> {
    let user_email = user_email.to_lowercase();
    let client = store.get_conn(Some(tenant_id)).await?;

    let rows = client
        .query(
            "SELECT tenant_id, user_email, kind, label, expires_at, created_at, updated_at
             FROM user_downstream_credentials
             WHERE tenant_id = $1 AND user_email = $2
             ORDER BY kind",
            &[&tenant_id, &user_email],
        )
        .await
        .to_store_error()?;

    Ok(rows.into_iter().map(row_to_summary).collect())
}

/// A tenant this user may hold a credential in, and whether they hold one.
#[derive(Debug, Clone, Serialize)]
pub struct TenantCredentialSlot {
    pub tenant_id: String,
    pub tenant_name: String,
    /// True when the tenant authenticates as each user — the only case where a
    /// personal credential is used at all.
    pub per_user: bool,
    pub credential: Option<CredentialSummary>,
}

/// The tenants whose tools this user can call, and therefore the tenants they
/// may store a credential in.
///
/// Three ways to qualify, and a consumer reaches a provider's tenant only by
/// the third:
///   1. it is their own default tenant
///   2. they are a member of it
///   3. they hold an active API key pinned to it — which is what the OAuth
///      consent flow issues, and is the proof that the tenant let them in
pub async fn accessible_tenants(
    store: &EndpointStore,
    user_email: &str,
) -> Result<Vec<(String, String)>, StoreError> {
    let email = user_email.to_lowercase();
    let client = store.get_admin_conn().await?;

    let rows = client
        .query(
            "SELECT DISTINCT t.id, t.name
             FROM tenants t
             WHERE t.id IN (
                 SELECT up.default_tenant_id FROM user_preferences up
                  WHERE LOWER(up.email) = $1 AND up.default_tenant_id IS NOT NULL
                 UNION
                 SELECT tu.tenant_id FROM tenant_users tu WHERE LOWER(tu.email) = $1
                 UNION
                 SELECT k.tenant_id FROM api_keys k
                  WHERE LOWER(k.email) = $1 AND k.is_active AND k.tenant_id IS NOT NULL
                 UNION
                 SELECT k.provider_tenant_id FROM api_keys k
                  WHERE LOWER(k.email) = $1 AND k.is_active AND k.provider_tenant_id IS NOT NULL
             )
             ORDER BY t.name",
            &[&email],
        )
        .await
        .to_store_error()?;

    Ok(rows.into_iter().map(|r| (r.get(0), r.get(1))).collect())
}

/// Reject a credential written for a tenant the user has no relationship with.
/// Without this, an email in a request body would be enough to plant a
/// credential in somebody else's workspace.
pub async fn require_tenant_access(
    store: &EndpointStore,
    user_email: &str,
    tenant_id: &str,
) -> Result<(), StoreError> {
    let allowed = accessible_tenants(store, user_email).await?;
    if allowed.iter().any(|(id, _)| id == tenant_id) {
        Ok(())
    } else {
        app_log!(warn, tenant_id = %tenant_id, "Rejected a credential for an inaccessible tenant");
        Err(StoreError::InvalidInput(
            "You do not have access to that workspace".into(),
        ))
    }
}

/// Every tenant the user can reach, with the credential they hold in it.
pub async fn credential_slots(
    store: &EndpointStore,
    user_email: &str,
) -> Result<Vec<TenantCredentialSlot>, StoreError> {
    let email = user_email.to_lowercase();
    let tenants = accessible_tenants(store, &email).await?;
    let client = store.get_admin_conn().await?;
    let mut slots = Vec::with_capacity(tenants.len());

    for (tenant_id, tenant_name) in tenants {
        let per_user = client
            .query_opt(
                "SELECT auth_mode FROM tenant_downstream_auth WHERE tenant_id = $1",
                &[&tenant_id],
            )
            .await
            .to_store_error()?
            .map(|r| r.get::<_, String>(0) == "per_user")
            .unwrap_or(false);

        let credential = client
            .query_opt(
                "SELECT tenant_id, user_email, kind, label, expires_at, created_at, updated_at
                 FROM user_downstream_credentials
                 WHERE tenant_id = $1 AND user_email = $2",
                &[&tenant_id, &email],
            )
            .await
            .to_store_error()?
            .map(row_to_summary);

        slots.push(TenantCredentialSlot { tenant_id, tenant_name, per_user, credential });
    }

    Ok(slots)
}

/// How many people in this tenant have added a credential.
///
/// A count, deliberately — not a list. An owner needs to know whether their
/// workspace is set up; they do not need to see who has and has not, and that
/// distinction is somebody's business but not this screen's.
pub async fn count_credentials(
    store: &EndpointStore,
    tenant_id: &str,
) -> Result<i64, StoreError> {
    let client = store.get_conn(Some(tenant_id)).await?;

    let row = client
        .query_one(
            "SELECT COUNT(DISTINCT user_email) FROM user_downstream_credentials
             WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    Ok(row.get(0))
}

pub async fn delete_credential(
    store: &EndpointStore,
    tenant_id: &str,
    user_email: &str,
    kind: &str,
) -> Result<bool, StoreError> {
    let user_email = user_email.to_lowercase();
    let client = store.get_conn(Some(tenant_id)).await?;

    let n = client
        .execute(
            "DELETE FROM user_downstream_credentials
             WHERE tenant_id = $1 AND user_email = $2 AND kind = $3",
            &[&tenant_id, &user_email, &kind],
        )
        .await
        .to_store_error()?;

    Ok(n > 0)
}

fn row_to_summary(row: tokio_postgres::Row) -> CredentialSummary {
    CredentialSummary {
        tenant_id:  row.get(0),
        user_email: row.get(1),
        kind:       row.get(2),
        label:      row.get(3),
        expires_at: row
            .get::<_, Option<DateTime<Utc>>>(4)
            .map(|t| t.to_rfc3339()),
        created_at: row.get::<_, DateTime<Utc>>(5).to_rfc3339(),
        updated_at: row.get::<_, DateTime<Utc>>(6).to_rfc3339(),
    }
}
