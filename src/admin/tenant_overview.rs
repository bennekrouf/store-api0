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
                 (SELECT count(*) FROM tenant_users tu WHERE tu.tenant_id = t.id),
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
                    FROM tenant_users tu3 WHERE tu3.tenant_id = t.id)
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
                    name              = COALESCE($7, name)
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
