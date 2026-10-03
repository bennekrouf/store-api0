// src/admin/users_directory.rs
//
// The super admin's directory of every user and the workspaces they belong to.
// Built for thousands: paged and filtered in SQL, never "load everything".
//
//   GET /api/admin/users?q=&tenant_id=&role=&sort=&page=&page_size=
//       q          matches the email or the name of any workspace they are in
//       tenant_id  only members of that workspace (with `role`, only that role)
//       role       owner | admin | member | consumer
//       sort       email (default) | last_active | newest
//       page       1-based; page_size up to 200
//       export=1   the whole filtered list (up to 10 000) in one page, for a CSV
//                  the dashboard builds
//   GET /api/admin/tenant-names   id, name and member count of every workspace,
//                                 for the directory's workspace filter
//
// Both need X-Internal-Secret; the gateway restricts them to the super admin.

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

const MAX_PAGE_SIZE: i64 = 200;
const MAX_EXPORT_ROWS: i64 = 10_000;
const ROLES: &[&str] = &["owner", "admin", "member", "consumer"];

#[derive(Debug, Deserialize)]
pub struct DirectoryQuery {
    pub q: Option<String>,
    pub tenant_id: Option<String>,
    pub role: Option<String>,
    pub sort: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    /// Set to 1 for the whole filtered list in one page (an export).
    pub export: Option<u8>,
}

/// The ORDER BY for a sort name. A whitelist: the value is never interpolated.
fn order_by(sort: Option<&str>) -> &'static str {
    match sort {
        Some("last_active") => "last_active DESC NULLS LAST, up.email ASC",
        Some("newest") => "up.created_at DESC NULLS LAST, up.email ASC",
        _ => "up.email ASC",
    }
}

pub async fn list_users(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<DirectoryQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let search = query.q.as_deref().map(str::trim).unwrap_or("").to_string();
    let tenant_id = query.tenant_id.as_deref().map(str::trim).unwrap_or("").to_string();
    let role = query.role.as_deref().map(str::trim).unwrap_or("").to_string();
    if !role.is_empty() && !ROLES.contains(&role.as_str()) {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"success": false, "error": format!("Unknown role '{}'", role)}));
    }
    let export = query.export == Some(1);
    let page_size = if export {
        MAX_EXPORT_ROWS
    } else {
        query.page_size.unwrap_or(50).clamp(1, MAX_PAGE_SIZE)
    };
    let page = if export { 1 } else { query.page.unwrap_or(1).max(1) };
    let offset = (page - 1) * page_size;

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Database unavailable"}))
        }
    };

    // One row per user. The filters run in SQL so the page is all that leaves
    // the database, whatever the platform's size.
    let sql = format!(
        "SELECT
             count(*) OVER () AS total,
             up.email,
             up.default_tenant_id,
             up.created_at,
             (SELECT COALESCE(json_agg(json_build_object(
                        'tenant_id', t.id, 'tenant', t.name, 'role', tu.role)
                        ORDER BY t.name), '[]'::json)
                FROM tenant_users tu JOIN tenants t ON t.id = tu.tenant_id
               WHERE tu.email = up.email) AS workspaces,
             (SELECT count(*) FROM api_keys k WHERE k.email = up.email AND k.is_active) AS active_keys,
             (SELECT max(k.last_used) FROM api_keys k WHERE k.email = up.email) AS last_active,
             (SELECT count(*) FROM channel_identities ci WHERE ci.user_email = up.email) AS linked_channels
         FROM user_preferences up
         WHERE ($1 = '' OR up.email ILIKE '%' || $1 || '%'
                OR EXISTS (SELECT 1 FROM tenant_users tu JOIN tenants t ON t.id = tu.tenant_id
                            WHERE tu.email = up.email AND t.name ILIKE '%' || $1 || '%'))
           AND ($2 = '' OR EXISTS (SELECT 1 FROM tenant_users tu
                                    WHERE tu.email = up.email AND tu.tenant_id = $2
                                      AND ($3 = '' OR tu.role = $3)))
           AND ($3 = '' OR $2 <> '' OR EXISTS (SELECT 1 FROM tenant_users tu
                                                WHERE tu.email = up.email AND tu.role = $3))
         ORDER BY {}
         LIMIT $4 OFFSET $5",
        order_by(query.sort.as_deref())
    );

    let rows = match client
        .query(&sql, &[&search, &tenant_id, &role, &page_size, &offset])
        .await
    {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, "User directory query failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Query failed"}));
        }
    };

    let total: i64 = rows.first().map(|r| r.get(0)).unwrap_or(0);
    let users: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let created: Option<chrono::DateTime<chrono::Utc>> = r.get(3);
            let last: Option<chrono::DateTime<chrono::Utc>> = r.get(6);
            serde_json::json!({
                "email": r.get::<_, String>(1),
                "default_tenant_id": r.get::<_, Option<String>>(2),
                "created_at": created.map(|t| t.to_rfc3339()),
                "workspaces": r.get::<_, serde_json::Value>(4),
                "active_keys": r.get::<_, i64>(5),
                "last_active": last.map(|t| t.to_rfc3339()),
                "linked_channels": r.get::<_, i64>(7),
            })
        })
        .collect();


    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "total": total,
        "page": page,
        "page_size": page_size,
        "users": users,
    }))
}

pub async fn tenant_names(req: HttpRequest, store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Database unavailable"}))
        }
    };
    match client
        .query(
            "SELECT t.id, t.name, (SELECT count(*) FROM tenant_users tu WHERE tu.tenant_id = t.id)
               FROM tenants t ORDER BY lower(t.name), t.id",
            &[],
        )
        .await
    {
        Ok(rows) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "tenants": rows.iter().map(|r| serde_json::json!({
                "id": r.get::<_, String>(0),
                "name": r.get::<_, String>(1),
                "users": r.get::<_, i64>(2),
            })).collect::<Vec<_>>(),
        })),
        Err(e) => {
            app_log!(error, error = %e, "Tenant names query failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Query failed"}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorting_is_a_whitelist() {
        assert_eq!(order_by(Some("email; DROP TABLE x")), "up.email ASC");
        assert!(order_by(Some("last_active")).starts_with("last_active DESC"));
    }

}
