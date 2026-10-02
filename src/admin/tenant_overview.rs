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
use crate::endpoint_store::tenant_management::{NAME_INDEX, NAME_TAKEN, NAME_TAKEN_SQL};
use slug::slugify;
use std::collections::{HashMap, HashSet};
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

    let own_hosts = first_party_hosts();
    let mut leaks = match identity_leaks(&client, &own_hosts).await {
        Ok(l) => l,
        Err(e) => {
            app_log!(error, error = %e, "tenants_overview: identity leak query failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    let tenants: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let id: String = r.get(0);
            let tenant_leaks = leaks.remove(&id).unwrap_or_default();
            let mcp_client_id: Option<String> = r.get(4);
            let google_client_id: Option<String> = r.get(5);
            let allow_api0_signin: bool = r.get(6);

            let has_google = google_client_id
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);

            serde_json::json!({
                "id": id,
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
                // Tools that send api0's identity headers (internal secret, the
                // caller's email) to a host outside `first_party_hosts`.
                "identity_leaks": tenant_leaks,
            })
        })
        .collect();

    app_log!(info, count = tenants.len(), "Served the tenant overview");

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tenants": tenants,
        "first_party_hosts": own_hosts,
    }))
}

// ── Identity leaks ────────────────────────────────────────────────────────────
//
// `forward_identity` defaults to true on both tools and endpoints, which is right
// for a first-party backend and wrong for anything else: a tool pointed at Azure
// DevOps that nobody remembered to switch off sends X-Internal-Secret and the
// caller's email to Microsoft on every call. Nothing fails, so nothing notices.

/// The domains trusted with api0's identity headers, subdomains included.
///
/// `API0_FIRST_PARTY_HOSTS` is a comma-separated list. The defaults are the
/// platform's own domains; a host on the private network is always trusted
/// (see [`is_first_party`]), so internal service names need no entry.
fn first_party_hosts() -> Vec<String> {
    let raw = std::env::var("API0_FIRST_PARTY_HOSTS")
        .ok()
        .filter(|s| !s.trim().is_empty())
        // api0's own domain only; a deployment adds the products it runs
        // first-party (e.g. API0_FIRST_PARTY_HOSTS=api0.ai,example.com).
        .unwrap_or_else(|| "api0.ai".to_string());
    raw.split(',')
        .map(|h| h.trim().trim_start_matches('.').to_lowercase())
        .filter(|h| !h.is_empty())
        .collect()
}

/// The host of a backend URL, lowercased, without port or credentials.
/// `None` when there is no host to speak of (relative or placeholder-only).
fn host_of(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://").map(|(_, r)| r).unwrap_or(url.trim());
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let authority = authority.rsplit_once('@').map(|(_, h)| h).unwrap_or(authority);
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next().unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or_default()
    };
    let host = host.to_lowercase();
    if host.is_empty() || host.contains('{') {
        return None;
    }
    Some(host)
}

/// Whether `host` may receive api0's identity headers.
///
/// Besides the listed domains: loopback, a bare service name (no dot — only
/// resolvable inside the deployment's own network) and the RFC 1918 ranges.
fn is_first_party(host: &str, own: &[String]) -> bool {
    if host == "localhost" || host == "::1" || !host.contains('.') {
        return true;
    }
    if let Ok(std::net::IpAddr::V4(ip)) = host.parse::<std::net::IpAddr>() {
        return ip.is_loopback() || ip.is_private();
    }
    own.iter()
        .any(|d| host == d || host.ends_with(&format!(".{d}")))
}

/// Per tenant, every tool that forwards identity to a host outside `own`.
///
/// Mirrors how the gateway assembles a tenant's tools: the active `mcp_tools`
/// rows, plus an endpoint only when no row already carries its name. Checking
/// the endpoint behind an explicit row would report a setting nobody uses.
async fn identity_leaks(
    client: &deadpool_postgres::Object,
    own: &[String],
) -> Result<HashMap<String, Vec<serde_json::Value>>, tokio_postgres::Error> {
    let explicit = client
        .query(
            "SELECT tenant_id, tool_name, backend_url, forward_identity
               FROM mcp_tools WHERE is_active = true",
            &[],
        )
        .await?;
    let endpoints = client
        .query(
            "SELECT g.tenant_id, g.name, e.text, e.base, g.base, e.path,
                    COALESCE(e.forward_identity, true)
               FROM api_groups g JOIN endpoints e ON g.id = e.group_id
              WHERE g.tenant_id IS NOT NULL",
            &[],
        )
        .await?;

    let mut named: HashSet<(String, String)> = HashSet::new();
    let mut tools: Vec<(String, String, String, bool)> = Vec::new();

    for r in &explicit {
        let tenant: String = r.get(0);
        let name: String = r.get(1);
        named.insert((tenant.clone(), name.clone()));
        tools.push((tenant, name, r.get(2), r.get(3)));
    }
    for r in &endpoints {
        let tenant: String = r.get(0);
        let group: String = r.get(1);
        let text: String = r.get(2);
        let e_base: String = r.get(3);
        let g_base: String = r.get(4);
        let path: String = r.get(5);
        let name = slugify(format!("{} {}", group, text));
        if name.is_empty() || named.contains(&(tenant.clone(), name.clone())) {
            continue;
        }
        let base = if e_base.is_empty() { &g_base } else { &e_base };
        let url = format!("{}{}", base.trim_end_matches('/'), path);
        tools.push((tenant, name, url, r.get(6)));
    }

    let mut out: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    for (tenant, name, url, forward) in tools {
        if !forward {
            continue;
        }
        let Some(host) = host_of(&url) else { continue };
        if is_first_party(&host, own) {
            continue;
        }
        out.entry(tenant)
            .or_default()
            .push(serde_json::json!({"tool": name, "host": host}));
    }
    for list in out.values_mut() {
        list.sort_by(|a, b| a["tool"].as_str().cmp(&b["tool"].as_str()));
    }
    Ok(out)
}

#[cfg(test)]
mod identity_leak_tests {
    use super::*;

    fn own() -> Vec<String> {
        vec!["api0.ai".to_string(), "example.com".to_string()]
    }

    #[test]
    fn host_of_strips_scheme_port_path_and_credentials() {
        assert_eq!(host_of("https://dev.azure.com/org/_apis/wit").as_deref(), Some("dev.azure.com"));
        assert_eq!(host_of("http://user:pw@API.example.com:8443/x?y").as_deref(), Some("api.example.com"));
        assert_eq!(host_of("http://[::1]:5007/x").as_deref(), Some("::1"));
        assert_eq!(host_of("/relative/path"), None);
        assert_eq!(host_of("https://{host}/x"), None);
    }

    #[test]
    fn own_domains_and_their_subdomains_are_first_party() {
        assert!(is_first_party("example.com", &own()));
        assert!(is_first_party("api.example.com", &own()));
        assert!(!is_first_party("notexample.com", &own()));
        assert!(!is_first_party("dev.azure.com", &own()));
    }

    #[test]
    fn the_private_network_is_first_party() {
        assert!(is_first_party("localhost", &own()));
        assert!(is_first_party("backend-service", &own()));
        assert!(is_first_party("127.0.0.1", &own()));
        assert!(is_first_party("10.0.0.7", &own()));
        assert!(is_first_party("192.168.1.20", &own()));
        assert!(!is_first_party("8.8.8.8", &own()));
    }
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

pub fn double_option<'de, D>(de: D) -> Result<Option<Option<String>>, D::Error>
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

    if let Some(new_name) = body.name.as_deref() {
        match client.query_opt(NAME_TAKEN_SQL, &[&new_name, &Some(&tenant_id)]).await {
            Ok(None) => {}
            Ok(Some(_)) => {
                return HttpResponse::Conflict()
                    .json(serde_json::json!({"success": false, "error": NAME_TAKEN}))
            }
            Err(e) => {
                app_log!(error, error = %e, "update_tenant_config: name check failed");
                return HttpResponse::InternalServerError()
                    .json(serde_json::json!({"success": false, "error": "DB error"}));
            }
        }
    }

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
            if msg.contains(NAME_INDEX) {
                return HttpResponse::Conflict()
                    .json(serde_json::json!({"success": false, "error": NAME_TAKEN}));
            }
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

/// The ordered statements that remove a tenant and everything under it.
///
/// Shared by the single delete and the bulk prune so the two can never disagree
/// about what "delete a tenant" means. Order matters: some children cascade,
/// `tenant_users` and `api_keys.provider_tenant_id` hold foreign keys that would
/// block, and `api_groups.tenant_id` has no foreign key at all and would be
/// orphaned silently. Every statement takes the tenant id as `$1`.
fn cascade_steps() -> Vec<(&'static str, String)> {
    // Endpoints of this tenant's groups, named once and reused: every child of an
    // endpoint has to go before the endpoint itself.
    const OWNED_ENDPOINTS: &str =
        "SELECT e.id FROM endpoints e JOIN api_groups g ON e.group_id = g.id
          WHERE g.tenant_id = $1";

    vec![
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
    ]
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

    let steps = cascade_steps();

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
                 SELECT $1, up.email, 'consumer'
                   FROM user_preferences up
                  WHERE LOWER(up.email) = LOWER($2)
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

#[derive(serde::Deserialize)]
pub struct PruneBody {
    /// When true (the default), report what would go without touching anything.
    #[serde(default = "default_true")]
    pub dry_run: bool,
}

fn default_true() -> bool {
    true
}

/// Tenants that hold nothing anyone would miss.
///
/// "Empty" deliberately ignores credits: a balance is not content, and an
/// account nobody reaches is not worth keeping for it. It does *not* ignore a
/// client id — a tenant somebody has wired a connector to is in use even before
/// anything is imported into it, and deleting it would break that connector.
///
/// Consumer links count as content too: a tenant whose users were attached to a
/// provider still records a relationship worth keeping.
const EMPTY_TENANTS: &str = "
    SELECT t.id, t.name, t.credit_balance
      FROM tenants t
     WHERE t.mcp_client_id IS NULL
       AND NOT EXISTS (SELECT 1 FROM api_groups g WHERE g.tenant_id = t.id)
       AND NOT EXISTS (SELECT 1 FROM mcp_tools m WHERE m.tenant_id = t.id)
       AND NOT EXISTS (SELECT 1 FROM api_keys k
                        WHERE k.tenant_id = t.id OR k.provider_tenant_id = t.id)
       AND NOT EXISTS (SELECT 1 FROM tenant_users tu
                        WHERE tu.tenant_id = t.id AND tu.role = 'consumer')
       AND NOT EXISTS (SELECT 1 FROM tenant_downstream_auth d WHERE d.tenant_id = t.id)
     ORDER BY t.created_at ASC";

/// POST /api/internal/tenants/prune-empty
///
/// Removes every tenant holding no groups, endpoints, tools, keys, consumers,
/// downstream auth or client id. Defaults to a dry run: the caller sees the list
/// first and has to ask again to actually delete.
pub async fn prune_empty_tenants(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<PruneBody>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let mut client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "prune_empty_tenants: no connection");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    let rows = match client.query(EMPTY_TENANTS, &[]).await {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, "prune_empty_tenants: query failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    let candidates: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<_, String>(0),
                "name": r.get::<_, String>(1),
                "credit_balance": r.get::<_, i64>(2),
            })
        })
        .collect();

    if body.dry_run {
        return HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "dry_run": true,
            "candidates": candidates,
        }));
    }

    let ids: Vec<String> = rows.iter().map(|r| r.get::<_, String>(0)).collect();
    let steps = cascade_steps();

    // One transaction for the whole prune: a half-finished sweep would leave
    // tenants stripped of their rows but still listed.
    let tx = match client.transaction().await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, error = %e, "prune_empty_tenants: could not open transaction");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    for id in &ids {
        for (label, sql) in &steps {
            if let Err(e) = tx.execute(sql.as_str(), &[id]).await {
                app_log!(error, error = %e, tenant_id = %id, step = %label,
                    "prune_empty_tenants: step failed, rolling back everything");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "error": format!("Failed clearing {} for {}: {}", label, id, e)
                }));
            }
        }
    }

    if let Err(e) = tx.commit().await {
        app_log!(error, error = %e, "prune_empty_tenants: commit failed");
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "Commit failed"}));
    }

    app_log!(warn, count = ids.len(), "Pruned empty tenants");

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "dry_run": false,
        "deleted": candidates,
    }))
}

/// Against a real database — see tenant_members::db_tests for how to run.
#[cfg(test)]
mod db_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL"]
    async fn identity_leaks_follow_the_gateways_tool_list() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = EndpointStore::new(&url).await.expect("store");
        let run = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        let tenant = crate::endpoint_store::tenant_management::get_default_tenant(
            &store, &format!("leaks-{run}@example.com"),
        ).await.unwrap();
        let t = tenant.id.as_str();
        let c = store.get_admin_conn().await.unwrap();

        let g = format!("g-{run}");
        c.execute("INSERT INTO api_groups (id, name, description, base, tenant_id)
                   VALUES ($1, 'azure', '', 'https://dev.azure.com/org', $2)", &[&g, &t]).await.unwrap();
        for (id, text, fwd) in [("e1", "list items", None), ("e2", "list off", Some(false)), ("e3", "shadowed", None)] {
            c.execute("INSERT INTO endpoints (id, text, description, verb, base, path, suggested_sentence, group_id, forward_identity)
                       VALUES ($1, $2, '', 'GET', '', '/_apis/x', '', $3, $4)",
                &[&format!("{id}-{run}"), &text, &g, &fwd]).await.unwrap();
        }
        for (name, url, fwd, active) in [
            ("azure-shadowed", "https://dev.azure.com/org/x", false, true),   // explicit row wins over e3
            ("first-party", "https://api.example.com/x", true, true),
            ("internal", "http://backend-service:8080/x", true, true),
            ("jira", "https://acme.atlassian.net/rest", true, true),
            ("inactive", "https://acme.atlassian.net/rest", true, false),
        ] {
            c.execute("INSERT INTO mcp_tools (tenant_id, tool_name, backend_url, forward_identity, is_active)
                       VALUES ($1, $2, $3, $4, $5)", &[&t, &name, &url, &fwd, &active]).await.unwrap();
        }

        let own = vec!["api0.ai".to_string(), "example.com".to_string()];
        let mut leaks = identity_leaks(&c, &own).await.unwrap();
        let found: Vec<(String, String)> = leaks.remove(t).unwrap_or_default().iter()
            .map(|v| (v["tool"].as_str().unwrap().to_string(), v["host"].as_str().unwrap().to_string()))
            .collect();
        assert_eq!(found, vec![
            ("azure-list-items".to_string(), "dev.azure.com".to_string()),
            ("jira".to_string(), "acme.atlassian.net".to_string()),
        ]);
    }

    async fn has_index(c: &crate::infra::db::PgConnection) -> bool {
        c.query_one("SELECT count(*) FROM pg_indexes WHERE indexname = 'tenants_name_ci_key'", &[])
            .await
            .unwrap()
            .get::<_, i64>(0)
            == 1
    }

    /// Existing duplicates must cost the index, never the boot.
    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL; drops and recreates a shared index"]
    async fn colliding_names_warn_instead_of_stopping_the_store() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = EndpointStore::new(&url).await.expect("store");
        let run = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        let c = store.get_admin_conn().await.unwrap();

        c.execute("DROP INDEX tenants_name_ci_key", &[]).await.unwrap();
        let (a, b) = (format!("dup-a-{run}"), format!("dup-b-{run}"));
        for (id, name) in [(&a, format!("Dup {run}")), (&b, format!("dup {run}"))] {
            c.execute("INSERT INTO tenants (id, name, credit_balance, created_at) VALUES ($1, $2, 0, NOW())",
                &[id, &name]).await.unwrap();
        }

        EndpointStore::new(&url).await.expect("the store still starts");
        assert!(!has_index(&c).await, "the index is skipped while names collide");

        c.execute("DELETE FROM tenants WHERE id = $1", &[&b]).await.unwrap();
        EndpointStore::new(&url).await.expect("store");
        assert!(has_index(&c).await, "and created at the next start once they don't");
        c.execute("DELETE FROM tenants WHERE id = $1", &[&a]).await.unwrap();
    }
}
