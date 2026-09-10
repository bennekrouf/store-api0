// src/admin/tenant_overview.rs
//
// The platform-wide view of how every tenant is wired up.
//
// Internal (X-Internal-Secret):
//   GET /api/internal/tenants/overview
//
// This exists because the settings that decide whether a tenant actually works
// are spread across four tables, and until now the only way to see them together
// was a hand-written SQL join against production. Every column here is one that
// has silently broken a connector: a missing mcp_client_id means Claude has no
// client to name, neither sign-in method configured means the OAuth step is
// refused outright, and groups sitting in a different tenant than the connector
// reads from means the tools simply are not there.
//
// Read-only, and it deliberately carries no secrets: whether a downstream
// credential exists, never the credential.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use std::sync::Arc;

fn check_internal_secret(req: &HttpRequest) -> bool {
    let expected = match std::env::var("API0_INTERNAL_SECRET") {
        Ok(s) if !s.is_empty() => s,
        _ => return false,
    };
    req.headers()
        .get("X-Internal-Secret")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == expected)
        .unwrap_or(false)
}

/// GET /api/internal/tenants/overview
pub async fn tenants_overview(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "tenants_overview: no connection");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    // One row per tenant, with the counts that answer "is anything actually
    // there?" — subqueries rather than joins so a tenant with no groups, no
    // members or no tools still appears rather than dropping out.
    let rows = client
        .query(
            "SELECT
                 t.id,
                 t.name,
                 t.credit_balance,
                 t.created_at,
                 t.mcp_client_id,
                 t.google_client_id,
                 t.allow_api0_signin,
                 (SELECT count(*) FROM tenant_users tu
                   WHERE tu.tenant_id = t.id AND tu.role <> 'consumer'),
                 (SELECT count(*) FROM api_groups g WHERE g.tenant_id = t.id),
                 (SELECT count(*) FROM endpoints e
                    JOIN api_groups g2 ON e.group_id = g2.id
                   WHERE g2.tenant_id = t.id),
                 (SELECT count(*) FROM mcp_tools m
                   WHERE m.tenant_id = t.id AND m.is_active = true),
                 (SELECT count(*) FROM api_keys k WHERE k.tenant_id = t.id),
                 EXISTS (SELECT 1 FROM tenant_downstream_auth d WHERE d.tenant_id = t.id),
                 (SELECT string_agg(tu2.email, ', ' ORDER BY tu2.email)
                    FROM tenant_users tu2
                   WHERE tu2.tenant_id = t.id AND tu2.role = 'owner'),
                 -- Security profile. Never the credential itself, only its shape:
                 -- which mode, and whether the pieces that mode needs are present.
                 (SELECT d.auth_mode FROM tenant_downstream_auth d WHERE d.tenant_id = t.id),
                 (SELECT d.per_user_scheme FROM tenant_downstream_auth d WHERE d.tenant_id = t.id),
                 (SELECT d.per_user_header FROM tenant_downstream_auth d WHERE d.tenant_id = t.id),
                 (SELECT d.per_user_verify_url FROM tenant_downstream_auth d WHERE d.tenant_id = t.id),
                 (SELECT d.bearer_token IS NOT NULL FROM tenant_downstream_auth d WHERE d.tenant_id = t.id),
                 (SELECT d.service_account_json IS NOT NULL FROM tenant_downstream_auth d WHERE d.tenant_id = t.id),
                 (SELECT d.custom_headers IS NOT NULL FROM tenant_downstream_auth d WHERE d.tenant_id = t.id),
                 (SELECT d.updated_at FROM tenant_downstream_auth d WHERE d.tenant_id = t.id),
                 -- Members, as a JSON array so roles survive the trip.
                 (SELECT COALESCE(
                     json_agg(json_build_object('email', tu3.email, 'role', tu3.role)
                              ORDER BY tu3.role, tu3.email),
                     '[]'::json)
                    FROM tenant_users tu3 WHERE tu3.tenant_id = t.id),
                 -- Consumers reach this tenant's tools through a connector but
                 -- have no authority over it, so they are counted separately.
                 (SELECT count(*) FROM tenant_users tu4
                   WHERE tu4.tenant_id = t.id AND tu4.role = 'consumer')
             FROM tenants t
             ORDER BY t.created_at ASC",
            &[],
        )
        .await
        .to_store_error();

    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, "tenants_overview: query failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    let tenants: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let mcp_client_id: Option<String> = r.get(4);
            let google_client_id: Option<String> = r.get(5);
            let allow_api0_signin: bool = r.get(6);

            let has_google = google_client_id
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);

            serde_json::json!({
                "id": r.get::<_, String>(0),
                "name": r.get::<_, String>(1),
                "credit_balance": r.get::<_, i64>(2),
                "created_at": r.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
                "mcp_client_id": mcp_client_id,
                "google_client_id": google_client_id,
                "allow_api0_signin": allow_api0_signin,
                // The gateway refuses to mint an OAuth code unless one of the two
                // sign-in methods is set. Precomputed so the dashboard shows the
                // same verdict the gateway will reach, rather than its own guess.
                "can_sign_in": has_google || allow_api0_signin,
                "member_count": r.get::<_, i64>(7),
                "group_count": r.get::<_, i64>(8),
                "endpoint_count": r.get::<_, i64>(9),
                "mcp_tool_count": r.get::<_, i64>(10),
                "api_key_count": r.get::<_, i64>(11),
                // Whether a shared downstream credential exists — never its value.
                "has_downstream_auth": r.get::<_, bool>(12),
                "owners": r.get::<_, Option<String>>(13),
                // 'none' when the tenant has no row at all, which is what the
                // gateway falls back to anyway.
                "auth_mode": r.get::<_, Option<String>>(14).unwrap_or_else(|| "none".to_string()),
                "per_user_scheme": r.get::<_, Option<String>>(15),
                "per_user_header": r.get::<_, Option<String>>(16),
                "per_user_verify_url": r.get::<_, Option<String>>(17),
                "has_bearer_token": r.get::<_, Option<bool>>(18).unwrap_or(false),
                "has_service_account": r.get::<_, Option<bool>>(19).unwrap_or(false),
                "has_custom_headers": r.get::<_, Option<bool>>(20).unwrap_or(false),
                "auth_updated_at": r
                    .get::<_, Option<chrono::DateTime<chrono::Utc>>>(21)
                    .map(|t| t.to_rfc3339()),
                "members": r.get::<_, serde_json::Value>(22),
                "consumer_count": r.get::<_, i64>(23),
            })
        })
        .collect();

    app_log!(info, count = tenants.len(), "Served the tenant overview");

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tenants": tenants,
    }))
}

#[derive(serde::Deserialize)]
pub struct UpdateTenantConfig {
    /// `None` leaves the field alone; `Some(None)` clears it. Distinguishing the
    /// two matters: a panel that edits one field must not blank the others.
    #[serde(default, deserialize_with = "double_option")]
    pub mcp_client_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    pub google_client_id: Option<Option<String>>,
    pub allow_api0_signin: Option<bool>,
    pub name: Option<String>,
}

fn double_option<'de, D>(de: D) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(de).map(Some)
}

/// PUT /api/internal/tenants/{tenant_id}/config
///
/// The admin counterpart to `set_mcp_client_id`. That one resolves its target as
/// the caller's *default* tenant, which is right for a tenant editing itself and
/// useless for an operator fixing someone else's — so this one takes the tenant
/// id explicitly and touches only the fields present in the body.
pub async fn update_tenant_config(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
    body: web::Json<UpdateTenantConfig>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let tenant_id = path.into_inner();

    // A rename must produce a real name. Empty is meaningless, and an address is
    // the confusion this validation exists to stop.
    if let Some(new_name) = body.name.as_deref() {
        if !crate::endpoint_store::tenant_management::is_valid_tenant_name(new_name) {
            return HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "error": "A tenant name is required, and cannot be an email address"
            }));
        }
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "update_tenant_config: no connection");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    // Empty string means "clear it", so a blanked input does not store "".
    let blank_to_null = |v: Option<Option<String>>| -> Option<Option<String>> {
        v.map(|inner| inner.filter(|s| !s.trim().is_empty()).map(|s| s.trim().to_string()))
    };

    let mcp = blank_to_null(body.mcp_client_id.clone());
    let google = blank_to_null(body.google_client_id.clone());

    let result = client
        .execute(
            "UPDATE tenants
                SET mcp_client_id     = CASE WHEN $2 THEN $3 ELSE mcp_client_id END,
                    google_client_id  = CASE WHEN $4 THEN $5 ELSE google_client_id END,
                    allow_api0_signin = COALESCE($6, allow_api0_signin),
                    name              = COALESCE(NULLIF(btrim($7), ''), name)
              WHERE id = $1",
            &[
                &tenant_id,
                &mcp.is_some(),
                &mcp.clone().flatten(),
                &google.is_some(),
                &google.clone().flatten(),
                &body.allow_api0_signin,
                &body.name,
            ],
        )
        .await
        .to_store_error();

    match result {
        Ok(0) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "No such tenant"})),
        Ok(_) => {
            app_log!(
                info,
                tenant_id = %tenant_id,
                allow_api0_signin = ?body.allow_api0_signin,
                "Admin updated tenant configuration"
            );
            HttpResponse::Ok().json(serde_json::json!({"success": true}))
        }
        Err(e) => {
            // A duplicate mcp_client_id is the caller's mistake, not a server fault:
            // the column is UNIQUE because it must resolve to exactly one tenant.
            let msg = e.to_string();
            if msg.contains("duplicate") || msg.contains("unique") {
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "success": false,
                    "error": "That Client ID is already taken by another tenant"
                }));
            }
            app_log!(error, error = %e, tenant_id = %tenant_id, "update_tenant_config failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}))
        }
    }
}

#[derive(serde::Deserialize)]
pub struct DeleteTenantBody {
    /// The tenant's exact name, retyped. Deleting a tenant destroys its groups,
    /// endpoints and keys, and there is no undo — so the caller has to name the
    /// thing they mean, not just click the row they happened to have open.
    pub confirm_name: String,
}

/// DELETE /api/internal/tenants/{tenant_id}
///
/// Removing a tenant is not one statement. Some children cascade
/// (`mcp_tools`, `tenant_downstream_auth`, `whatsapp_channels`,
/// `user_downstream_credentials`, `idp_auth_requests`); `tenant_users` and
/// `api_keys.provider_tenant_id` hold foreign keys *without* cascade and would
/// block the delete; and `api_groups.tenant_id` has no foreign key at all, so a
/// plain delete would silently orphan its groups, endpoints and parameters.
///
/// Hence the explicit order below, in one transaction: either the tenant and
/// everything under it goes, or nothing does.
///
/// `api_usage_logs` and `credit_transactions` are deliberately left in place.
/// They are financial and audit history; losing them to a tidy-up would be worse
/// than carrying rows that point at a tenant that no longer exists.
pub async fn delete_tenant(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
    body: web::Json<DeleteTenantBody>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let tenant_id = path.into_inner();

    let mut client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "delete_tenant: no connection");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    let name: String = match client
        .query_opt("SELECT name FROM tenants WHERE id = $1", &[&tenant_id])
        .await
    {
        Ok(Some(row)) => row.get(0),
        Ok(None) => {
            return HttpResponse::NotFound()
                .json(serde_json::json!({"success": false, "error": "No such tenant"}))
        }
        Err(e) => {
            app_log!(error, error = %e, "delete_tenant: lookup failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    if body.confirm_name.trim() != name.trim() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "Confirmation does not match the tenant name"
        }));
    }

    let tx = match client.transaction().await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, error = %e, "delete_tenant: could not open transaction");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    // Endpoints of this tenant's groups, named once and reused: every child of an
    // endpoint has to go before the endpoint itself.
    const OWNED_ENDPOINTS: &str =
        "SELECT e.id FROM endpoints e JOIN api_groups g ON e.group_id = g.id
          WHERE g.tenant_id = $1";

    let steps: Vec<(&str, String)> = vec![
        // Anyone defaulting to this tenant falls back to a fresh personal one.
        (
            "user_preferences.default_tenant_id",
            "UPDATE user_preferences SET default_tenant_id = NULL WHERE default_tenant_id = $1"
                .to_string(),
        ),
        (
            "parameter_alternatives",
            format!("DELETE FROM parameter_alternatives WHERE endpoint_id IN ({OWNED_ENDPOINTS})"),
        ),
        (
            "parameters",
            format!("DELETE FROM parameters WHERE endpoint_id IN ({OWNED_ENDPOINTS})"),
        ),
        (
            "user_endpoints",
            format!("DELETE FROM user_endpoints WHERE endpoint_id IN ({OWNED_ENDPOINTS})"),
        ),
        (
            "endpoints",
            "DELETE FROM endpoints WHERE group_id IN (SELECT id FROM api_groups WHERE tenant_id = $1)"
                .to_string(),
        ),
        (
            "user_groups",
            "DELETE FROM user_groups WHERE group_id IN (SELECT id FROM api_groups WHERE tenant_id = $1)"
                .to_string(),
        ),
        ("api_groups", "DELETE FROM api_groups WHERE tenant_id = $1".to_string()),
        (
            "api_keys",
            "DELETE FROM api_keys WHERE tenant_id = $1 OR provider_tenant_id = $1".to_string(),
        ),
        ("tenant_users", "DELETE FROM tenant_users WHERE tenant_id = $1".to_string()),
        // Last: takes mcp_tools, downstream auth, whatsapp channels, per-user
        // credentials and idp requests with it by cascade.
        ("tenants", "DELETE FROM tenants WHERE id = $1".to_string()),
    ];

    let mut removed = serde_json::Map::new();
    for (label, sql) in &steps {
        match tx.execute(sql.as_str(), &[&tenant_id]).await {
            Ok(n) => {
                removed.insert((*label).to_string(), serde_json::json!(n));
            }
            Err(e) => {
                app_log!(error, error = %e, tenant_id = %tenant_id, step = %label,
                    "delete_tenant: step failed, rolling back");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "error": format!("Failed while clearing {}: {}", label, e)
                }));
            }
        }
    }

    if let Err(e) = tx.commit().await {
        app_log!(error, error = %e, tenant_id = %tenant_id, "delete_tenant: commit failed");
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "Commit failed"}));
    }

    app_log!(warn, tenant_id = %tenant_id, tenant_name = %name, removed = ?removed,
        "Tenant deleted by an administrator");

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "deleted": name,
        "removed": removed,
    }))
}

#[derive(serde::Deserialize)]
pub struct AddConsumersBody {
    pub emails: Vec<String>,
}

/// POST /api/internal/tenants/{tenant_id}/consumers
///
/// Record that these people reach this tenant's tools, without giving them any
/// authority over it — the role is `consumer`, which every authorization path
/// excludes.
///
/// It exists because a provider's relationship with its users is not always
/// visible to api0. A partner whose users hold consumer API keys is linked
/// automatically at key issue; one that uses api0 only as a credit ledger — its
/// users never holding a key — leaves no trace to derive the link from. This is
/// how those get attached.
pub async fn add_consumers(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
    body: web::Json<AddConsumersBody>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let tenant_id = path.into_inner();

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "add_consumers: no connection");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    match client
        .query_opt("SELECT 1 FROM tenants WHERE id = $1", &[&tenant_id])
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => {
            return HttpResponse::NotFound()
                .json(serde_json::json!({"success": false, "error": "No such tenant"}))
        }
        Err(e) => {
            app_log!(error, error = %e, "add_consumers: tenant lookup failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    }

    let mut linked: Vec<String> = Vec::new();
    let mut skipped: Vec<serde_json::Value> = Vec::new();

    for raw in &body.emails {
        let email = raw.trim().to_lowercase();
        if email.is_empty() {
            continue;
        }
        if !email.contains('@') {
            skipped.push(serde_json::json!({"email": email, "reason": "not an email address"}));
            continue;
        }

        // tenant_users.email is a foreign key into user_preferences, so someone
        // api0 has never seen cannot be linked. Say so rather than failing the
        // whole batch on one unknown address.
        let known = client
            .query_opt(
                "SELECT 1 FROM user_preferences WHERE LOWER(email) = LOWER($1)",
                &[&email],
            )
            .await;

        match known {
            Ok(Some(_)) => {}
            Ok(None) => {
                skipped.push(serde_json::json!({
                    "email": email,
                    "reason": "unknown to api0 — no account with that address"
                }));
                continue;
            }
            Err(e) => {
                skipped.push(serde_json::json!({"email": email, "reason": e.to_string()}));
                continue;
            }
        }

        match client
            .execute(
                "INSERT INTO tenant_users (tenant_id, email, role)
                 VALUES ($1, $2, 'consumer')
                 ON CONFLICT (tenant_id, email) DO NOTHING",
                &[&tenant_id, &email],
            )
            .await
        {
            Ok(_) => linked.push(email),
            Err(e) => {
                skipped.push(serde_json::json!({"email": email, "reason": e.to_string()}));
            }
        }
    }

    app_log!(
        info,
        tenant_id = %tenant_id,
        linked = linked.len(),
        skipped = skipped.len(),
        "Linked consumers to a tenant"
    );

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "linked": linked,
        "skipped": skipped,
    }))
}
