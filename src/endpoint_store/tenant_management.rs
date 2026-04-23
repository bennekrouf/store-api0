use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::endpoint_store::models::Tenant;
use crate::infra::db::PgConnection;
use uuid::Uuid;
use chrono::Utc;

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
    let name = email.to_string(); // Personal tenant name is email

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

pub async fn get_tenant_by_mcp_client_id(
    store: &EndpointStore,
    mcp_client_id: &str,
) -> Result<Option<(Tenant, Option<String>)>, StoreError> {
    let client = store.get_admin_conn().await?;
    let row = client
        .query_opt(
            "SELECT id, name, credit_balance, created_at, google_client_id
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
        (tenant, google_client_id)
    }))
}

pub async fn set_mcp_client_id(
    store: &EndpointStore,
    email: &str,
    mcp_client_id: Option<&str>,
    google_client_id: Option<&str>,
) -> Result<(), StoreError> {
    let tenant = get_default_tenant(store, email).await?;
    let client = store.get_admin_conn().await?;

    client
        .execute(
            "UPDATE tenants
             SET mcp_client_id    = $1,
                 google_client_id = $2
             WHERE id = $3",
            &[&mcp_client_id, &google_client_id, &tenant.id],
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
