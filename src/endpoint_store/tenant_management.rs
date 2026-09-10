use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::endpoint_store::models::Tenant;
use crate::infra::db::PgConnection;
use uuid::Uuid;
use chrono::Utc;

/// A readable default name for a personal tenant, e.g. `bob.smith@x.com` →
/// `Personal — bob.smith`. Never contains `@`, so it cannot be mistaken for the
/// owner's address.
pub fn personal_tenant_name(email: &str) -> String {
    let local = email.split('@').next().unwrap_or(email).trim();
    if local.is_empty() {
        "Personal workspace".to_string()
    } else {
        format!("Personal — {}", local)
    }
}

/// Is this a name a person chose, rather than an address that leaked into the
/// name column? Used to reject renames that would reintroduce the confusion.
pub fn is_valid_tenant_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty() && !trimmed.contains('@')
}

pub async fn get_or_create_personal_tenant(
    store: &EndpointStore,
    email: &str,
) -> Result<Tenant, StoreError> {
    let client = store.get_admin_conn().await?;
    get_or_create_personal_tenant_with_conn(&client, email).await
}

pub async fn get_or_create_personal_tenant_with_conn(
    client: &PgConnection,
    email: &str,
) -> Result<Tenant, StoreError> {
    let email = email.to_lowercase();
    // 1. Check if user has a default tenant
    let default_tenant_reow = client
        .query_opt(
            "SELECT t.id, t.name, t.credit_balance, t.created_at 
             FROM user_preferences up
             JOIN tenants t ON up.default_tenant_id = t.id
             WHERE LOWER(up.email) = LOWER($1)",
            &[&email],
        )
        .await
        .to_store_error()?;

    if let Some(row) = default_tenant_reow {
        return Ok(Tenant {
            id: row.get(0),
            name: row.get(1),
            credit_balance: row.get(2),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
        });
    }

    app_log!(info, email = %email, "Creating personal tenant for user");

    let tenant_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    // A tenant name is a label people read in a connector list and a dashboard
    // header, so it must not be an email address: seeing "someone@example.com"
    // where a workspace name belongs makes every screen ambiguous about whether
    // it is naming a person or a workspace. Derive something readable instead.
    let name = personal_tenant_name(&email);

    // Note: We are using a client that likely has bypass_rls = true (from get_admin_conn)
    
    // 0. Ensure user exists in user_preferences
    let user_exists_row = client
        .query_opt("SELECT 1 FROM user_preferences WHERE LOWER(email) = LOWER($1)", &[&email])
        .await
        .to_store_error()?;

    if user_exists_row.is_none() {
        client.execute(
            "INSERT INTO user_preferences (email, hidden_defaults, credit_balance) VALUES ($1, '', 0)",
            &[&email],
        )
        .await
        .to_store_error()?;
    }

    // Create Tenant
    client.execute(
        "INSERT INTO tenants (id, name, credit_balance, created_at) VALUES ($1, $2, 0, $3)",
        &[&tenant_id, &name, &now],
    )
    .await
    .to_store_error()?;

    // Link User to Tenant
    client.execute(
        "INSERT INTO tenant_users (tenant_id, email, role) VALUES ($1, $2, 'owner')",
        &[&tenant_id, &email],
    )
    .await
    .to_store_error()?;

    // Set as default
    client.execute(
        "UPDATE user_preferences SET default_tenant_id = $1 WHERE email = $2",
        &[&tenant_id, &email],
    )
    .await
    .to_store_error()?;

    // MIGRATION: If user had credits in user_preferences, move them
    let old_balance_row = client.query_one("SELECT credit_balance FROM user_preferences WHERE email = $1", &[&email]).await.to_store_error()?;
    let old_balance: i64 = old_balance_row.get(0);

    if old_balance > 0 {
        app_log!(info, email = %email, amount = old_balance, "Migrating legacy credits to new personal tenant");
        
        client.execute(
            "UPDATE tenants SET credit_balance = credit_balance + $1 WHERE id = $2",
            &[&old_balance, &tenant_id]
        ).await.to_store_error()?;
        
        client.execute("UPDATE user_preferences SET credit_balance = 0 WHERE email = $1", &[&email]).await.to_store_error()?;
    }

    Ok(Tenant {
        id: tenant_id,
        name,
        credit_balance: old_balance,
        created_at: now.to_rfc3339(),
    })
}

pub async fn get_default_tenant(
    store: &EndpointStore,
    email: &str,
) -> Result<Tenant, StoreError> {
    let email = email.to_lowercase();
    get_or_create_personal_tenant(store, &email).await
}

/// The tenant behind an OAuth client id, with how its users sign in:
/// `(tenant, google_client_id, allow_api0_signin)`.
pub async fn get_tenant_by_mcp_client_id(
    store: &EndpointStore,
    mcp_client_id: &str,
) -> Result<Option<(Tenant, Option<String>, bool)>, StoreError> {
    let client = store.get_admin_conn().await?;
    let row = client
        .query_opt(
            "SELECT id, name, credit_balance, created_at, google_client_id,
                    allow_api0_signin
             FROM tenants WHERE mcp_client_id = $1",
            &[&mcp_client_id],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| {
        let tenant = Tenant {
            id:             r.get(0),
            name:           r.get(1),
            credit_balance: r.get(2),
            created_at:     r.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
        };
        let google_client_id: Option<String> = r.get(4);
        let allow_api0_signin: bool = r.get(5);
        (tenant, google_client_id, allow_api0_signin)
    }))
}

/// Set how a tenant's people sign in.
///
/// `allow_api0_signin` is `Option` rather than `bool` so a caller that only means
/// to change a client id leaves the opt-in alone: `None` keeps the stored value.
/// It is the sharper of the two settings — turning it on lets *any* api0 account
/// connect to this workspace — so it must never move as a side effect.
pub async fn set_mcp_client_id(
    store: &EndpointStore,
    email: &str,
    mcp_client_id: Option<&str>,
    google_client_id: Option<&str>,
    allow_api0_signin: Option<bool>,
) -> Result<(), StoreError> {
    let tenant = get_default_tenant(store, email).await?;
    let client = store.get_admin_conn().await?;

    client
        .execute(
            "UPDATE tenants
             SET mcp_client_id     = $1,
                 google_client_id  = $2,
                 allow_api0_signin = COALESCE($3, allow_api0_signin)
             WHERE id = $4",
            &[
                &mcp_client_id,
                &google_client_id,
                &allow_api0_signin,
                &tenant.id,
            ],
        )
        .await
        .to_store_error()?;

    Ok(())
}

pub async fn update_tenant_name(
    store: &EndpointStore,
    email: &str,
    new_name: &str,
) -> Result<(), StoreError> {
    let tenant = get_default_tenant(store, email).await?;
    let client = store.get_admin_conn().await?;

    client
        .execute(
            "UPDATE tenants SET name = $1 WHERE id = $2",
            &[&new_name, &tenant.id],
        )
        .await
        .to_store_error()?;

    Ok(())
}

#[allow(dead_code)]
pub async fn verify_tenant_access(
    store: &EndpointStore,
    email: &str,
    tenant_id: &str,
) -> Result<bool, StoreError> {
    let client = store.get_admin_conn().await?;
    verify_tenant_access_with_conn(&client, email, tenant_id).await
}

pub async fn verify_tenant_access_with_conn(
    client: &PgConnection,
    email: &str,
    tenant_id: &str,
) -> Result<bool, StoreError> {
    let email = email.to_lowercase();
    let row = client
        .query_opt(
            "SELECT 1 FROM tenant_users WHERE tenant_id = $1 AND LOWER(email) = LOWER($2)",
            &[&tenant_id, &email],
        )
        .await
        .to_store_error()?;

    Ok(row.is_some())
}

#[allow(dead_code)]
pub async fn list_user_tenants(
    store: &EndpointStore,
    email: &str,
) -> Result<Vec<Tenant>, StoreError> {
    let client = store.get_admin_conn().await?;
    list_user_tenants_with_conn(&client, email).await
}

pub async fn list_user_tenants_with_conn(
    client: &PgConnection,
    email: &str,
) -> Result<Vec<Tenant>, StoreError> {
    let email = email.to_lowercase();
    let rows = client
        .query(
            "SELECT t.id, t.name, t.credit_balance, t.created_at
             FROM tenants t
             JOIN tenant_users tu ON t.id = tu.tenant_id
             WHERE LOWER(tu.email) = LOWER($1)
             ORDER BY t.created_at ASC",
            &[&email],
        )
        .await
        .to_store_error()?;

    let mut tenants = Vec::new();
    for row in rows {
        tenants.push(Tenant {
            id: row.get(0),
            name: row.get(1),
            credit_balance: row.get(2),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
        });
    }

    Ok(tenants)
}
