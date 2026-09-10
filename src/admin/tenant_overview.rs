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
                   WHERE tu2.tenant_id = t.id AND tu2.role = 'owner')
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
            })
        })
        .collect();

    app_log!(info, count = tenants.len(), "Served the tenant overview");

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tenants": tenants,
    }))
}
