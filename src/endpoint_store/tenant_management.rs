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

/// Tenant names are unique ignoring case and surrounding space: "solanize" and
/// "Solanize" are two workspaces nobody can tell apart in a connector list or
/// the admin panel. The `tenants_name_ci_key` index enforces it; this query is
/// how callers ask first, so a clash is answered with a sentence rather than a
/// constraint violation. `$2` is the tenant being renamed (NULL when creating),
/// which may of course keep its own name.
pub const NAME_TAKEN_SQL: &str = "SELECT 1 FROM tenants
     WHERE lower(btrim(name)) = lower(btrim($1)) AND id IS DISTINCT FROM $2";

/// The refusal for a name already in use. Carried in `StoreError::InvalidInput`;
/// handlers compare against it to answer 409 rather than 400.
pub const NAME_TAKEN: &str = "That name is already used by another workspace";

/// The unique index behind [`NAME_TAKEN_SQL`], for recognising the race where
/// two renames to the same name both pass the check.
pub const NAME_INDEX: &str = "tenants_name_ci_key";

/// Roles that grant authority over a tenant.
///
/// `consumer` is deliberately absent. A consumer row records that someone uses a
/// provider's tools through a connector — it is a relationship, not a permission.
/// Treating it as membership would let any cvenom end-user upload endpoints into
/// cvenom's namespace or read its settings.
pub const MEMBER_ROLES: &[&str] = &["owner", "member", "admin"];

pub async fn get_or_create_personal_tenant(
    store: &EndpointStore,
    email: &str,
) -> Result<Tenant, StoreError> {
    let mut client = store.get_admin_conn().await?;
    get_or_create_personal_tenant_with_conn(&mut client, email).await
}

pub async fn get_or_create_personal_tenant_with_conn(
    client: &mut PgConnection,
    email: &str,
) -> Result<Tenant, StoreError> {
    let email = email.to_lowercase();

    // Creating a tenant is several writes — the tenant, its owner, the user's
    // default — and they stand or fall together. Without a transaction a failure
    // after the first one left a tenant nobody belonged to, and the next request
    // (finding no default) created another.
    let tx = client.transaction().await.to_store_error()?;

    // Check-then-create is a race: a dashboard opening fires several requests
    // at once, and each found no tenant and created one — two "Personal"
    // tenants created in the same second. A per-email lock makes the second
    // caller wait, then find the first one's tenant. Transaction-scoped, so it
    // is released on commit or rollback alike.
    tx.execute("SELECT pg_advisory_xact_lock(hashtext($1))", &[&email])
        .await
        .to_store_error()?;
    let tenant = get_or_create_personal_tenant_locked(&tx, &email).await?;
    tx.commit().await.to_store_error()?;
    Ok(tenant)
}

async fn get_or_create_personal_tenant_locked(
    client: &deadpool_postgres::Transaction<'_>,
    email: &str,
) -> Result<Tenant, StoreError> {
    let email = email.to_string();
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
    //
    // Two people can share a local part (bob@a.com, bob@b.com), and names are
    // unique, so the second gets "Personal — bob 2".
    let base = personal_tenant_name(&email);
    let mut name = base.clone();
    for n in 2.. {
        let taken = client
            .query_opt(NAME_TAKEN_SQL, &[&name, &None::<String>])
            .await
            .to_store_error()?;
        if taken.is_none() {
            break;
        }
        name = format!("{} {}", base, n);
    }

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

    // Someone invited into a workspace joins it instead of getting a personal
    // one: a workspace nobody asked for is exactly the clutter that leaves
    // accounts with several tenants and no idea which is theirs.
    if let Some(invited_id) =
        crate::endpoint_store::tenant_members::accept_pending_invites(client, &email).await?
    {
        client
            .execute(
                "UPDATE user_preferences SET default_tenant_id = $1 WHERE LOWER(email) = LOWER($2)",
                &[&invited_id, &email],
            )
            .await
            .to_store_error()?;
        let row = client
            .query_one(
                "SELECT id, name, credit_balance, created_at FROM tenants WHERE id = $1",
                &[&invited_id],
            )
            .await
            .to_store_error()?;
        return Ok(Tenant {
            id: row.get(0),
            name: row.get(1),
            credit_balance: row.get(2),
            created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
        });
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

/// The caller's tenant, or `None` — never creating one.
///
/// [`get_default_tenant`] creates a tenant when none exists, which is right when
/// somebody is signing up and wrong everywhere else: reading a credit balance
/// should not bring an account into being.
pub async fn find_default_tenant(
    store: &EndpointStore,
    email: &str,
) -> Result<Option<Tenant>, StoreError> {
    let email = email.to_lowercase();
    let client = store.get_admin_conn().await?;
    let row = client
        .query_opt(
            "SELECT t.id, t.name, t.credit_balance, t.created_at
             FROM user_preferences up
             JOIN tenants t ON up.default_tenant_id = t.id
             WHERE LOWER(up.email) = LOWER($1)",
            &[&email],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| Tenant {
        id: r.get(0),
        name: r.get(1),
        credit_balance: r.get(2),
        created_at: r.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
    }))
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
    // `None` leaves the column alone; `Some(None)` clears it.
    mcp_client_id: Option<Option<&str>>,
    google_client_id: Option<Option<&str>>,
    allow_api0_signin: Option<bool>,
) -> Result<(), StoreError> {
    let tenant = get_default_tenant(store, email).await?;
    let client = store.get_admin_conn().await?;

    // Absent must mean "leave it", not "clear it". A caller that only means to
    // turn on a sign-in method would otherwise blank the client id, and a
    // workspace would lose the identifier its connector is wired to.
    client
        .execute(
            "UPDATE tenants
             SET mcp_client_id     = CASE WHEN $1 THEN $2 ELSE mcp_client_id END,
                 google_client_id  = CASE WHEN $3 THEN $4 ELSE google_client_id END,
                 allow_api0_signin = COALESCE($5, allow_api0_signin)
             WHERE id = $6",
            &[
                &mcp_client_id.is_some(),
                &mcp_client_id.flatten(),
                &google_client_id.is_some(),
                &google_client_id.flatten(),
                &allow_api0_signin,
                &tenant.id,
            ],
        )
        .await
        .to_store_error()?;

    Ok(())
}

/// Record that `email` reaches `tenant_id`'s tools through a connector.
///
/// Written when a consumer key is issued, so a provider can see who uses it.
/// The role is `consumer`, which grants nothing — see [`MEMBER_ROLES`]. An
/// existing row is left alone so this never demotes a real owner or member.
pub async fn link_consumer_to_tenant(
    store: &EndpointStore,
    email: &str,
    tenant_id: &str,
) -> Result<(), StoreError> {
    let client = store.get_admin_conn().await?;
    // Take the address as user_preferences stores it: tenant_users.email is a
    // foreign key onto that column and matches by value, so a lowercased copy of
    // a differently-cased row would be rejected.
    client
        .execute(
            "INSERT INTO tenant_users (tenant_id, email, role)
             SELECT $1, up.email, 'consumer'
               FROM user_preferences up
              WHERE LOWER(up.email) = LOWER($2)
             ON CONFLICT (tenant_id, email) DO NOTHING",
            &[&tenant_id, &email],
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
    let new_name = new_name.trim();
    if !is_valid_tenant_name(new_name) {
        return Err(StoreError::InvalidInput(
            "A workspace name is required, and cannot be an email address".to_string(),
        ));
    }

    let tenant = get_default_tenant(store, email).await?;
    let client = store.get_admin_conn().await?;

    let taken = client
        .query_opt(NAME_TAKEN_SQL, &[&new_name, &Some(&tenant.id)])
        .await
        .to_store_error()?;
    if taken.is_some() {
        return Err(StoreError::InvalidInput(NAME_TAKEN.to_string()));
    }

    match client
        .execute("UPDATE tenants SET name = $1 WHERE id = $2", &[&new_name, &tenant.id])
        .await
    {
        Ok(_) => Ok(()),
        // Lost a race with another rename to the same name.
        Err(e) if e.to_string().contains(NAME_INDEX) => {
            Err(StoreError::InvalidInput(NAME_TAKEN.to_string()))
        }
        Err(e) => Err(StoreError::Database(e.to_string())),
    }
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
            // Allowlist, not denylist: a role added later should have to be
            // granted authority deliberately, not inherit it by not being
            // 'consumer'.
            "SELECT 1 FROM tenant_users
              WHERE tenant_id = $1 AND LOWER(email) = LOWER($2)
                AND role = ANY($3)",
            &[&tenant_id, &email, &MEMBER_ROLES],
        )
        .await
        .to_store_error()?;

    Ok(row.is_some())
}

/// A workspace as one of its members sees it: which role they hold there, and
/// whether it is the one everything they do currently acts on.
#[derive(Debug, Clone, serde::Serialize)]
pub struct UserTenant {
    #[serde(flatten)]
    pub tenant: Tenant,
    pub role: String,
    /// The account's default tenant — the one keys, credits, tools, settings and
    /// uploads resolve to. Exactly one entry is active, unless the default
    /// points at a workspace the user has since lost access to.
    pub active: bool,
}

pub async fn list_user_tenants_with_conn(
    client: &PgConnection,
    email: &str,
) -> Result<Vec<UserTenant>, StoreError> {
    let email = email.to_lowercase();
    let rows = client
        .query(
            "SELECT t.id, t.name, t.credit_balance, t.created_at, tu.role,
                    COALESCE(up.default_tenant_id = t.id, false)
             FROM tenants t
             JOIN tenant_users tu ON t.id = tu.tenant_id
             LEFT JOIN user_preferences up ON LOWER(up.email) = LOWER(tu.email)
             WHERE LOWER(tu.email) = LOWER($1) AND tu.role = ANY($2)
             ORDER BY t.created_at ASC",
            &[&email, &MEMBER_ROLES],
        )
        .await
        .to_store_error()?;

    Ok(rows
        .iter()
        .map(|row| UserTenant {
            tenant: Tenant {
                id: row.get(0),
                name: row.get(1),
                credit_balance: row.get(2),
                created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
            },
            role: row.get(4),
            active: row.get(5),
        })
        .collect())
}

/// Make `tenant_id` the workspace everything `email` does acts on.
///
/// Switching is just moving `default_tenant_id`: every store path already
/// resolves the caller's tenant through it, so none of them has to learn about
/// switching. `None` when the caller is not a member there — a consumer, in
/// particular, must never be able to step into a provider's workspace, since
/// that would hand them its settings, keys and credits.
pub async fn set_active_tenant(
    store: &EndpointStore,
    email: &str,
    tenant_id: &str,
) -> Result<Option<Tenant>, StoreError> {
    let client = store.get_admin_conn().await?;
    if !verify_tenant_access_with_conn(&client, email, tenant_id).await? {
        return Ok(None);
    }

    client
        .execute(
            "UPDATE user_preferences SET default_tenant_id = $1 WHERE LOWER(email) = LOWER($2)",
            &[&tenant_id, &email],
        )
        .await
        .to_store_error()?;

    let row = client
        .query_one(
            "SELECT id, name, credit_balance, created_at FROM tenants WHERE id = $1",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    Ok(Some(Tenant {
        id: row.get(0),
        name: row.get(1),
        credit_balance: row.get(2),
        created_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
    }))
}

/// Against a real database — see tenant_members::db_tests for how to run.
#[cfg(test)]
mod db_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL"]
    async fn concurrent_first_requests_create_one_workspace() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = std::sync::Arc::new(EndpointStore::new(&url).await.expect("store"));
        let email = format!("race-{}@example.com", Uuid::new_v4().simple());

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (store, email) = (store.clone(), email.clone());
                tokio::spawn(async move { get_default_tenant(&store, &email).await.unwrap().id })
            })
            .collect();
        let mut ids = Vec::new();
        for h in handles {
            ids.push(h.await.unwrap());
        }
        ids.dedup();
        assert_eq!(ids.len(), 1, "every caller got the same workspace");

        let c = store.get_admin_conn().await.unwrap();
        let memberships: i64 = c
            .query_one("SELECT count(*) FROM tenant_users WHERE email = $1", &[&email])
            .await.unwrap().get(0);
        assert_eq!(memberships, 1);
        let orphans: i64 = c
            .query_one(
                "SELECT count(*) FROM tenants t WHERE t.name = $1
                   AND NOT EXISTS (SELECT 1 FROM tenant_users tu WHERE tu.tenant_id = t.id)",
                &[&personal_tenant_name(&email)],
            )
            .await.unwrap().get(0);
        assert_eq!(orphans, 0, "no member-less workspace left behind");
    }
}
