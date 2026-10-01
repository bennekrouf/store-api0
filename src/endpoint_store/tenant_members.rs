// src/endpoint_store/tenant_members.rs
//
// Who belongs to a workspace, and how people are added to and removed from it.
//
// Every operation acts on the caller's *active* workspace (their
// `default_tenant_id`), the same one every other store path resolves to, so a
// member switches workspace first and then manages it.
//
// Roles, highest first: owner, admin, member. Owners and admins manage members;
// an admin can neither touch an owner nor make one. `consumer` is not a member
// role at all (see `tenant_management::MEMBER_ROLES`) and nothing here grants it.
//
// Every write locks the tenant row first, so two concurrent changes to the same
// workspace run one after the other — which is what keeps "a workspace always
// has an owner" true when two owners demote each other at the same moment.

use crate::app_log;
use crate::endpoint_store::tenant_management::{get_default_tenant, MEMBER_ROLES};
use crate::endpoint_store::{EndpointStore, StoreError};
use deadpool_postgres::Transaction;

/// Roles a member can be given. Highest first.
pub const ASSIGNABLE_ROLES: &[&str] = &["owner", "admin", "member"];

/// How many invitations may be waiting on one workspace. Each one sends an
/// email to an address of the inviter's choosing, so this is also a spam cap.
const MAX_PENDING_INVITES: i64 = 50;

fn rank(role: &str) -> u8 {
    match role {
        "owner" => 3,
        "admin" => 2,
        "member" => 1,
        _ => 0,
    }
}

fn can_manage(role: &str) -> bool {
    rank(role) >= rank("admin")
}

#[derive(Debug)]
pub enum MemberError {
    /// The caller's role does not allow this.
    Forbidden(String),
    /// The request itself is wrong: unknown role, bad address, not a member.
    Invalid(String),
    /// Allowed in principle, refused by the workspace's current state.
    Conflict(String),
    Store(StoreError),
}

impl std::fmt::Display for MemberError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Forbidden(m) | Self::Invalid(m) | Self::Conflict(m) => f.write_str(m),
            Self::Store(e) => write!(f, "{}", e),
        }
    }
}

impl From<StoreError> for MemberError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

impl From<tokio_postgres::Error> for MemberError {
    fn from(e: tokio_postgres::Error) -> Self {
        Self::Store(StoreError::Database(e.to_string()))
    }
}

impl From<deadpool_postgres::PoolError> for MemberError {
    fn from(e: deadpool_postgres::PoolError) -> Self {
        Self::Store(StoreError::from(e))
    }
}

const LAST_OWNER: &str =
    "A workspace needs at least one owner — make someone else owner first";

/// The caller's active workspace and their role in it.
struct Seat {
    tenant_id: String,
    tenant_name: String,
    role: String,
}

/// Resolve the caller's seat inside `tx`, locking the workspace row so that
/// every membership change to one workspace is serialised.
async fn seat(tx: &Transaction<'_>, email: &str) -> Result<Seat, MemberError> {
    let row = tx
        .query_opt(
            "SELECT t.id, t.name, tu.role
               FROM user_preferences up
               JOIN tenants t       ON t.id = up.default_tenant_id
               JOIN tenant_users tu ON tu.tenant_id = t.id AND tu.email = up.email
              WHERE LOWER(up.email) = LOWER($1) AND tu.role = ANY($2)
                FOR UPDATE OF t",
            &[&email, &MEMBER_ROLES],
        )
        .await?;
    let row = row.ok_or_else(|| {
        MemberError::Forbidden("You are not a member of your active workspace".into())
    })?;
    Ok(Seat { tenant_id: row.get(0), tenant_name: row.get(1), role: row.get(2) })
}

/// A member's current role in a workspace, or `None` when they hold none.
async fn role_of(
    tx: &Transaction<'_>,
    tenant_id: &str,
    email: &str,
) -> Result<Option<String>, MemberError> {
    let row = tx
        .query_opt(
            "SELECT role FROM tenant_users
              WHERE tenant_id = $1 AND LOWER(email) = LOWER($2) AND role = ANY($3)",
            &[&tenant_id, &email, &MEMBER_ROLES],
        )
        .await?;
    Ok(row.map(|r| r.get(0)))
}

async fn owner_count(tx: &Transaction<'_>, tenant_id: &str) -> Result<i64, MemberError> {
    let row = tx
        .query_one(
            "SELECT count(*) FROM tenant_users WHERE tenant_id = $1 AND role = 'owner'",
            &[&tenant_id],
        )
        .await?;
    Ok(row.get(0))
}

fn normalise_email(raw: &str) -> Result<String, MemberError> {
    let email = raw.trim().to_lowercase();
    let well_formed = email
        .split_once('@')
        .map(|(local, domain)| !local.is_empty() && domain.contains('.'))
        .unwrap_or(false);
    if !well_formed || email.contains(char::is_whitespace) {
        return Err(MemberError::Invalid(format!("'{}' is not an email address", raw.trim())));
    }
    Ok(email)
}

fn assignable(role: &str) -> Result<&'static str, MemberError> {
    ASSIGNABLE_ROLES
        .iter()
        .find(|r| **r == role)
        .copied()
        .ok_or_else(|| MemberError::Invalid(format!("Unknown role '{}'", role)))
}

// ── Listing ───────────────────────────────────────────────────────────────────

/// The active workspace's members and pending invitations. Any member may look.
pub async fn list_members(
    store: &EndpointStore,
    email: &str,
) -> Result<serde_json::Value, MemberError> {
    get_default_tenant(store, email).await?;
    let mut client = store.get_admin_conn().await?;
    let tx = client.transaction().await?;
    let seat = seat(&tx, email).await?;

    let members = tx
        .query(
            "SELECT email, role FROM tenant_users
              WHERE tenant_id = $1 AND role = ANY($2)
              ORDER BY CASE role WHEN 'owner' THEN 0 WHEN 'admin' THEN 1 ELSE 2 END, email",
            &[&seat.tenant_id, &MEMBER_ROLES],
        )
        .await?;
    let invites = tx
        .query(
            "SELECT email, role, invited_by, created_at FROM tenant_invites
              WHERE tenant_id = $1 ORDER BY created_at",
            &[&seat.tenant_id],
        )
        .await?;
    tx.commit().await?;

    Ok(serde_json::json!({
        "tenant": { "id": seat.tenant_id, "name": seat.tenant_name },
        "your_role": seat.role,
        "can_manage": can_manage(&seat.role),
        "members": members.iter().map(|r| serde_json::json!({
            "email": r.get::<_, String>(0),
            "role": r.get::<_, String>(1),
        })).collect::<Vec<_>>(),
        "invites": invites.iter().map(|r| serde_json::json!({
            "email": r.get::<_, String>(0),
            "role": r.get::<_, String>(1),
            "invited_by": r.get::<_, String>(2),
            "created_at": r.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
        })).collect::<Vec<_>>(),
    }))
}

// ── Adding people ─────────────────────────────────────────────────────────────

/// What an invitation turned into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InviteOutcome {
    /// They already had an api0 account and are a member now.
    Added,
    /// No account yet: they join when they first sign in with that address.
    Invited,
}

/// Put `invitee` into `tenant_id` with `role`, without any check on who asks.
///
/// Someone api0 already knows becomes a member at once — `tenant_users.email`
/// is a foreign key onto `user_preferences`, so only then *can* they be one.
/// Anyone else gets a pending invitation, accepted at their first sign-in. A
/// consumer of the workspace is promoted; an existing member is a conflict.
async fn add_or_invite(
    tx: &Transaction<'_>,
    tenant_id: &str,
    invitee: &str,
    role: &str,
    invited_by: &str,
) -> Result<InviteOutcome, MemberError> {
    if role_of(tx, tenant_id, invitee).await?.is_some() {
        return Err(MemberError::Conflict(format!("{} is already a member", invitee)));
    }

    // The address exactly as user_preferences stores it: the foreign key
    // matches by value, so a lowercased copy of a mixed-case row is rejected.
    let account = tx
        .query_opt("SELECT email FROM user_preferences WHERE LOWER(email) = $1", &[&invitee])
        .await?;

    if let Some(row) = account {
        let stored: String = row.get(0);
        tx.execute(
            "INSERT INTO tenant_users (tenant_id, email, role) VALUES ($1, $2, $3)
             ON CONFLICT (tenant_id, email) DO UPDATE SET role = EXCLUDED.role",
            &[&tenant_id, &stored, &role],
        )
        .await?;
        tx.execute(
            "DELETE FROM tenant_invites WHERE tenant_id = $1 AND email = $2",
            &[&tenant_id, &invitee],
        )
        .await?;
        return Ok(InviteOutcome::Added);
    }

    let pending: i64 = tx
        .query_one(
            "SELECT count(*) FROM tenant_invites WHERE tenant_id = $1 AND email <> $2",
            &[&tenant_id, &invitee],
        )
        .await?
        .get(0);
    if pending >= MAX_PENDING_INVITES {
        return Err(MemberError::Conflict(format!(
            "This workspace already has {} pending invitations — cancel some first",
            MAX_PENDING_INVITES
        )));
    }

    tx.execute(
        "INSERT INTO tenant_invites (tenant_id, email, role, invited_by) VALUES ($1, $2, $3, $4)
         ON CONFLICT (tenant_id, email)
         DO UPDATE SET role = EXCLUDED.role, invited_by = EXCLUDED.invited_by, created_at = NOW()",
        &[&tenant_id, &invitee, &role, &invited_by],
    )
    .await?;
    Ok(InviteOutcome::Invited)
}

fn notify(store: &std::sync::Arc<EndpointStore>, to: &str, workspace: &str, role: &str, invited_by: &str, outcome: InviteOutcome) {
    crate::email::send_async(
        store.clone(),
        to.to_string(),
        crate::email::EmailKind::WorkspaceInvite {
            workspace: workspace.to_string(),
            role: role.to_string(),
            invited_by: invited_by.to_string(),
            has_account: outcome == InviteOutcome::Added,
        },
    );
}

/// Invite `invitee` into the caller's active workspace.
///
/// Owners and admins only, and nobody can hand out a role above their own.
pub async fn invite(
    store: &std::sync::Arc<EndpointStore>,
    caller: &str,
    invitee: &str,
    role: &str,
) -> Result<InviteOutcome, MemberError> {
    let invitee = normalise_email(invitee)?;
    let role = assignable(role)?;

    get_default_tenant(store, caller).await?;
    let mut client = store.get_admin_conn().await?;
    let tx = client.transaction().await?;
    let seat = seat(&tx, caller).await?;

    if !can_manage(&seat.role) {
        return Err(MemberError::Forbidden("Only owners and admins can invite people".into()));
    }
    if rank(role) > rank(&seat.role) {
        return Err(MemberError::Forbidden(format!(
            "An {} cannot make someone {}",
            seat.role, role
        )));
    }

    let outcome = add_or_invite(&tx, &seat.tenant_id, &invitee, role, caller).await?;
    tx.commit().await?;

    app_log!(info, tenant_id = %seat.tenant_id, by = %caller, invitee = %invitee, role = %role,
        outcome = ?outcome, "Workspace invitation");
    notify(store, &invitee, &seat.tenant_name, role, caller, outcome);
    Ok(outcome)
}

/// The platform admin's way in: add someone to any workspace, no caller role
/// involved. Replaces hand-written SQL for "make X owner of Y".
pub async fn admin_add_member(
    store: &std::sync::Arc<EndpointStore>,
    tenant_id: &str,
    invitee: &str,
    role: &str,
    admin_email: &str,
) -> Result<InviteOutcome, MemberError> {
    let invitee = normalise_email(invitee)?;
    let role = assignable(role)?;

    let mut client = store.get_admin_conn().await?;
    let tx = client.transaction().await?;
    let name = tx
        .query_opt("SELECT name FROM tenants WHERE id = $1 FOR UPDATE", &[&tenant_id])
        .await?
        .map(|r| r.get::<_, String>(0))
        .ok_or_else(|| MemberError::Invalid("No such workspace".into()))?;

    let outcome = add_or_invite(&tx, tenant_id, &invitee, role, admin_email).await?;
    tx.commit().await?;

    app_log!(warn, tenant_id = %tenant_id, by = %admin_email, invitee = %invitee, role = %role,
        outcome = ?outcome, "Platform admin added a workspace member");
    notify(store, &invitee, &name, role, admin_email, outcome);
    Ok(outcome)
}

/// Withdraw a pending invitation to the caller's active workspace.
pub async fn cancel_invite(
    store: &EndpointStore,
    caller: &str,
    invitee: &str,
) -> Result<(), MemberError> {
    let invitee = invitee.trim().to_lowercase();
    get_default_tenant(store, caller).await?;
    let mut client = store.get_admin_conn().await?;
    let tx = client.transaction().await?;
    let seat = seat(&tx, caller).await?;

    if !can_manage(&seat.role) {
        return Err(MemberError::Forbidden("Only owners and admins can cancel invitations".into()));
    }
    let n = tx
        .execute(
            "DELETE FROM tenant_invites WHERE tenant_id = $1 AND email = $2",
            &[&seat.tenant_id, &invitee],
        )
        .await?;
    if n == 0 {
        return Err(MemberError::Invalid(format!("No pending invitation for {}", invitee)));
    }
    tx.commit().await?;
    Ok(())
}

/// Accept every invitation waiting for `email`. Called while their account is
/// being set up, inside that transaction; returns the workspace to make their
/// default (the oldest invitation), if there was any.
pub async fn accept_pending_invites(
    tx: &Transaction<'_>,
    email: &str,
) -> Result<Option<String>, StoreError> {
    let accepted = tx
        .query(
            "DELETE FROM tenant_invites WHERE email = LOWER($1)
              RETURNING tenant_id, role, created_at",
            &[&email],
        )
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;

    let mut first: Option<(String, chrono::DateTime<chrono::Utc>)> = None;
    for row in &accepted {
        let tenant_id: String = row.get(0);
        let role: String = row.get(1);
        let at: chrono::DateTime<chrono::Utc> = row.get(2);
        // A consumer row is promoted; a real member keeps their role.
        tx.execute(
            "INSERT INTO tenant_users (tenant_id, email, role)
             SELECT $1, up.email, $3 FROM user_preferences up WHERE LOWER(up.email) = LOWER($2)
             ON CONFLICT (tenant_id, email) DO UPDATE SET role = EXCLUDED.role
              WHERE tenant_users.role = 'consumer'",
            &[&tenant_id, &email, &role],
        )
        .await
        .map_err(|e| StoreError::Database(e.to_string()))?;
        if first.as_ref().map(|(_, t)| at < *t).unwrap_or(true) {
            first = Some((tenant_id, at));
        }
    }

    if !accepted.is_empty() {
        app_log!(info, email = %email, count = accepted.len(), "Accepted workspace invitations at sign-in");
    }
    Ok(first.map(|(id, _)| id))
}

// ── Changing and removing ─────────────────────────────────────────────────────

/// Change a member's role in the caller's active workspace.
pub async fn change_role(
    store: &EndpointStore,
    caller: &str,
    member: &str,
    new_role: &str,
) -> Result<(), MemberError> {
    let new_role = assignable(new_role)?;
    get_default_tenant(store, caller).await?;
    let mut client = store.get_admin_conn().await?;
    let tx = client.transaction().await?;
    let seat = seat(&tx, caller).await?;

    if !can_manage(&seat.role) {
        return Err(MemberError::Forbidden("Only owners and admins can change roles".into()));
    }
    let current = role_of(&tx, &seat.tenant_id, member)
        .await?
        .ok_or_else(|| MemberError::Invalid(format!("{} is not a member", member)))?;

    if seat.role != "owner" && (current == "owner" || new_role == "owner") {
        return Err(MemberError::Forbidden("Only an owner can make or unmake owners".into()));
    }
    if current == "owner" && new_role != "owner" && owner_count(&tx, &seat.tenant_id).await? <= 1 {
        return Err(MemberError::Conflict(LAST_OWNER.into()));
    }

    tx.execute(
        "UPDATE tenant_users SET role = $3 WHERE tenant_id = $1 AND LOWER(email) = LOWER($2)",
        &[&seat.tenant_id, &member, &new_role],
    )
    .await?;
    tx.commit().await?;
    app_log!(info, tenant_id = %seat.tenant_id, by = %caller, member = %member,
        from = %current, to = %new_role, "Changed a member's role");
    Ok(())
}

/// Take `member` out of `tenant_id`, and everything that let them act there.
///
/// Their own API keys on the workspace are deactivated — a key carries its
/// tenant, so it would otherwise keep reaching the workspace's tools — and the
/// personal credentials they stored for its downstream API are dropped. If it
/// was their active workspace, their default moves to another one they belong
/// to; with none left it is cleared, and they get a fresh personal workspace
/// at their next sign-in.
async fn detach(tx: &Transaction<'_>, tenant_id: &str, member: &str) -> Result<(), MemberError> {
    tx.execute(
        "DELETE FROM tenant_users WHERE tenant_id = $1 AND LOWER(email) = LOWER($2)",
        &[&tenant_id, &member],
    )
    .await?;
    let keys = tx
        .execute(
            "UPDATE api_keys SET is_active = false
              WHERE tenant_id = $1 AND LOWER(email) = LOWER($2)
                AND provider_tenant_id IS NULL AND is_active",
            &[&tenant_id, &member],
        )
        .await?;
    tx.execute(
        "DELETE FROM user_downstream_credentials
          WHERE tenant_id = $1 AND LOWER(user_email) = LOWER($2)",
        &[&tenant_id, &member],
    )
    .await?;
    tx.execute(
        "UPDATE user_preferences up SET default_tenant_id = (
             SELECT tu.tenant_id FROM tenant_users tu JOIN tenants t ON t.id = tu.tenant_id
              WHERE tu.email = up.email AND tu.role = ANY($3)
              ORDER BY t.created_at LIMIT 1)
          WHERE LOWER(up.email) = LOWER($2) AND up.default_tenant_id = $1",
        &[&tenant_id, &member, &MEMBER_ROLES],
    )
    .await?;
    app_log!(info, tenant_id = %tenant_id, member = %member, keys_deactivated = keys,
        "Removed a member from a workspace");
    Ok(())
}

/// Remove someone from the caller's active workspace.
pub async fn remove_member(
    store: &EndpointStore,
    caller: &str,
    member: &str,
) -> Result<(), MemberError> {
    get_default_tenant(store, caller).await?;
    let mut client = store.get_admin_conn().await?;
    let tx = client.transaction().await?;
    let seat = seat(&tx, caller).await?;

    if !can_manage(&seat.role) {
        return Err(MemberError::Forbidden("Only owners and admins can remove members".into()));
    }
    let current = role_of(&tx, &seat.tenant_id, member)
        .await?
        .ok_or_else(|| MemberError::Invalid(format!("{} is not a member", member)))?;
    if current == "owner" && seat.role != "owner" {
        return Err(MemberError::Forbidden("Only an owner can remove an owner".into()));
    }
    if current == "owner" && owner_count(&tx, &seat.tenant_id).await? <= 1 {
        return Err(MemberError::Conflict(LAST_OWNER.into()));
    }

    detach(&tx, &seat.tenant_id, member).await?;
    tx.commit().await?;
    Ok(())
}

/// Leave the caller's active workspace. The last owner cannot.
pub async fn leave(store: &EndpointStore, caller: &str) -> Result<(), MemberError> {
    get_default_tenant(store, caller).await?;
    let mut client = store.get_admin_conn().await?;
    let tx = client.transaction().await?;
    let seat = seat(&tx, caller).await?;

    if seat.role == "owner" && owner_count(&tx, &seat.tenant_id).await? <= 1 {
        return Err(MemberError::Conflict(
            "You are this workspace's only owner — make someone else owner before leaving".into(),
        ));
    }

    detach(&tx, &seat.tenant_id, caller).await?;
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_rank_owner_over_admin_over_member() {
        assert!(rank("owner") > rank("admin"));
        assert!(rank("admin") > rank("member"));
        assert_eq!(rank("consumer"), 0);
        assert!(can_manage("owner") && can_manage("admin"));
        assert!(!can_manage("member") && !can_manage("consumer"));
    }

    #[test]
    fn only_member_roles_are_assignable() {
        assert!(assignable("owner").is_ok());
        assert!(assignable("member").is_ok());
        assert!(assignable("consumer").is_err());
        assert!(assignable("Owner").is_err());
    }

    #[test]
    fn addresses_are_normalised_and_checked() {
        assert_eq!(normalise_email("  Bob@Example.COM ").unwrap(), "bob@example.com");
        assert!(normalise_email("bob").is_err());
        assert!(normalise_email("@example.com").is_err());
        assert!(normalise_email("bob@localhost").is_err());
        assert!(normalise_email("bob smith@example.com").is_err());
    }
}

/// End-to-end against a real database. Every rule above lives in SQL, so this
/// is the test that matters. Run with a throwaway Postgres:
///   TEST_DATABASE_URL=postgres://app:app@localhost:55432/app \
///     cargo test --bin store -- --ignored membership_lifecycle
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::endpoint_store::tenant_management::{
        list_user_tenants_with_conn, set_active_tenant, update_tenant_name, NAME_TAKEN,
    };
    use std::sync::Arc;

    fn addr(tag: &str, run: &str) -> String {
        format!("{tag}-{run}@example.com")
    }

    async fn default_of(store: &EndpointStore, email: &str) -> Option<String> {
        let c = store.get_admin_conn().await.unwrap();
        c.query_one("SELECT default_tenant_id FROM user_preferences WHERE email = $1", &[&email])
            .await
            .unwrap()
            .get(0)
    }

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL"]
    async fn membership_lifecycle() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = Arc::new(EndpointStore::new(&url).await.expect("store"));
        // Schema must be re-runnable: the store applies it at every boot.
        EndpointStore::new(&url).await.expect("schema applied twice");

        let run = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        let (owner, bob, carol, dave, eve) = (
            addr("owner", &run), addr("bob", &run), addr("carol", &run), addr("dave", &run), addr("eve", &run),
        );

        // ── an owner and their workspace ─────────────────────────────────────
        let ws = get_default_tenant(&store, &owner).await.unwrap();
        let ws_id = ws.id.clone();

        // ── existing account: added at once ──────────────────────────────────
        let bob_personal = get_default_tenant(&store, &bob).await.unwrap();
        assert_eq!(invite(&store, &owner, &bob, "member").await.unwrap(), InviteOutcome::Added);
        assert!(matches!(invite(&store, &owner, &bob, "member").await, Err(MemberError::Conflict(_))),
            "inviting an existing member is a conflict");
        {
            let c = store.get_admin_conn().await.unwrap();
            let list = list_user_tenants_with_conn(&c, &bob).await.unwrap();
            assert_eq!(list.len(), 2, "bob: personal + invited");
            assert!(list.iter().any(|t| t.tenant.id == bob_personal.id && t.active));
            assert!(list.iter().any(|t| t.tenant.id == ws_id && t.role == "member" && !t.active));
        }
        assert!(set_active_tenant(&store, &bob, &ws_id).await.unwrap().is_some());
        assert_eq!(default_of(&store, &bob).await.as_deref(), Some(ws_id.as_str()));

        // A member cannot manage.
        assert!(matches!(invite(&store, &bob, &dave, "member").await, Err(MemberError::Forbidden(_))));
        assert!(matches!(remove_member(&store, &bob, &owner).await, Err(MemberError::Forbidden(_))));

        // ── no account yet: invited, accepted at first sign-in ───────────────
        assert_eq!(invite(&store, &owner, &carol, "admin").await.unwrap(), InviteOutcome::Invited);
        let listed = list_members(&store, &owner).await.unwrap();
        assert_eq!(listed["invites"].as_array().unwrap().len(), 1);
        assert_eq!(listed["can_manage"], true);

        let carol_ws = get_default_tenant(&store, &carol).await.unwrap();
        assert_eq!(carol_ws.id, ws_id, "carol lands in the workspace she was invited to");
        {
            let c = store.get_admin_conn().await.unwrap();
            let list = list_user_tenants_with_conn(&c, &carol).await.unwrap();
            assert_eq!(list.len(), 1, "no personal workspace was created for carol");
            assert_eq!(list[0].role, "admin");
            let left: i64 = c.query_one("SELECT count(*) FROM tenant_invites WHERE email = $1", &[&carol])
                .await.unwrap().get(0);
            assert_eq!(left, 0, "the invitation is consumed");
        }

        // ── admin limits ─────────────────────────────────────────────────────
        assert!(matches!(invite(&store, &carol, &dave, "owner").await, Err(MemberError::Forbidden(_))),
            "an admin cannot make an owner");
        assert_eq!(invite(&store, &carol, &dave, "member").await.unwrap(), InviteOutcome::Invited);
        cancel_invite(&store, &carol, &dave).await.unwrap();
        assert!(matches!(cancel_invite(&store, &carol, &dave).await, Err(MemberError::Invalid(_))));
        assert!(matches!(change_role(&store, &carol, &owner, "member").await, Err(MemberError::Forbidden(_))),
            "an admin cannot demote an owner");
        assert!(matches!(remove_member(&store, &carol, &owner).await, Err(MemberError::Forbidden(_))));
        change_role(&store, &carol, &bob, "admin").await.unwrap();
        assert!(matches!(change_role(&store, &carol, &bob, "boss").await, Err(MemberError::Invalid(_))));

        // ── the last owner ───────────────────────────────────────────────────
        assert!(matches!(leave(&store, &owner).await, Err(MemberError::Conflict(_))));
        assert!(matches!(change_role(&store, &owner, &owner, "member").await, Err(MemberError::Conflict(_))));

        change_role(&store, &owner, &carol, "owner").await.unwrap();
        leave(&store, &owner).await.unwrap();
        assert_eq!(default_of(&store, &owner).await, None, "owner belongs nowhere now");
        let owner_new = get_default_tenant(&store, &owner).await.unwrap();
        assert_ne!(owner_new.id, ws_id, "and gets a fresh personal workspace");

        // ── removal deactivates keys and moves the default ───────────────────
        {
            let c = store.get_admin_conn().await.unwrap();
            c.execute(
                "INSERT INTO api_keys (id, email, key_hash, key_prefix, key_name, generated_at, tenant_id)
                 VALUES ($1, $2, 'h', 'sk_test', 'k', NOW(), $3)",
                &[&format!("key-{run}"), &bob, &ws_id],
            ).await.unwrap();
        }
        remove_member(&store, &carol, &bob).await.unwrap();
        {
            let c = store.get_admin_conn().await.unwrap();
            let active: bool = c.query_one("SELECT is_active FROM api_keys WHERE id = $1", &[&format!("key-{run}")])
                .await.unwrap().get(0);
            assert!(!active, "bob's key on the workspace is off");
        }
        assert_eq!(default_of(&store, &bob).await.as_deref(), Some(bob_personal.id.as_str()),
            "bob's default falls back to his own workspace");
        assert!(set_active_tenant(&store, &bob, &ws_id).await.unwrap().is_none(),
            "and he can no longer switch into it");

        // carol is now the only owner and cannot remove herself.
        assert!(matches!(remove_member(&store, &carol, &carol).await, Err(MemberError::Conflict(_))));

        // ── platform admin ───────────────────────────────────────────────────
        get_default_tenant(&store, &eve).await.unwrap();
        assert_eq!(
            admin_add_member(&store, &ws_id, &eve, "owner", "root@example.com").await.unwrap(),
            InviteOutcome::Added
        );
        assert!(matches!(admin_add_member(&store, "no-such-tenant", &eve, "owner", "root@example.com").await,
            Err(MemberError::Invalid(_))));

        // ── names are unique ignoring case ───────────────────────────────────
        update_tenant_name(&store, &carol, &format!("Acme {run}")).await.unwrap();
        match update_tenant_name(&store, &eve, &format!("  ACME {run} ")).await {
            Err(StoreError::InvalidInput(m)) => assert_eq!(m, NAME_TAKEN),
            other => panic!("expected NAME_TAKEN, got {:?}", other),
        }
        let c = store.get_admin_conn().await.unwrap();
        let indexed: i64 = c.query_one(
            "SELECT count(*) FROM pg_indexes WHERE indexname = 'tenants_name_ci_key'", &[],
        ).await.unwrap().get(0);
        assert_eq!(indexed, 1, "the case-insensitive name index exists");
        let raced = c.execute(
            "UPDATE tenants SET name = $1 WHERE id = $2",
            &[&format!("acme {run}"), &bob_personal.id],
        ).await;
        assert!(raced.is_err(), "the index refuses what the check would have");
    }
}
