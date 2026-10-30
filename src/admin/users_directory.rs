// src/admin/users_directory.rs
//
// The super admin's directory of every user, the workspaces they belong to and
// the licences they hold. Built for thousands: paged and filtered in SQL, never
// "load everything".
//
// A person is an api0 account (user_preferences) or an email with a licence
// (subscriptions): a subscriber needs no account, so licence holders who never
// signed in to the dashboard are listed too, with has_account false.
//
//   GET /api/admin/users?q=&tenant_id=&role=&licence=&sort=&page=&page_size=
//       q          matches the email or the name of any workspace they are in
//       tenant_id  only members of that workspace (with `role`, only that role)
//       role       owner | admin | member | consumer
//       licence    live (any plan that gives access now) | paid | free | none
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

/// A licence that gives access now: the statuses payment::subscription treats
/// as live, in their paid month. A free one past its month is still live: the
/// store starts its next month on the next read.
const LIVE: &str = "s.licence IS NOT NULL AND s.status IN ('active', 'trialing') AND (s.period_end > NOW() OR s.is_free)";

#[derive(Debug, Deserialize)]
pub struct DirectoryQuery {
    pub q: Option<String>,
    pub tenant_id: Option<String>,
    pub role: Option<String>,
    pub licence: Option<String>,
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

/// The WHERE clause for a licence filter, None for an unknown one. A whitelist
/// too: the value is never interpolated.
fn licence_filter(licence: &str) -> Option<String> {
    let has = |extra: &str| {
        format!("EXISTS (SELECT 1 FROM subscriptions s WHERE s.email = lower(up.email) AND {LIVE}{extra})")
    };
    match licence {
        "" => Some("TRUE".into()),
        "live" => Some(has("")),
        "paid" => Some(has(" AND NOT s.is_free")),
        "free" => Some(has(" AND s.is_free")),
        "none" => Some(format!("NOT {}", has(""))),
        _ => None,
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
    let Some(licence_where) = licence_filter(query.licence.as_deref().map(str::trim).unwrap_or("")) else {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"success": false, "error": "Unknown licence filter"}));
    };
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

    // One row per person. The filters run in SQL so the page is all that leaves
    // the database, whatever the platform's size. Subscriptions keep emails in
    // lower case; accounts may not, hence lower(up.email) on that side.
    let sql = format!(
        "WITH people AS (
             SELECT email, default_tenant_id, created_at, TRUE AS has_account FROM user_preferences
             UNION ALL
             SELECT s.email, NULL, min(s.created_at), FALSE
               FROM subscriptions s
              WHERE s.licence IS NOT NULL
                AND NOT EXISTS (SELECT 1 FROM user_preferences a WHERE lower(a.email) = s.email)
              GROUP BY s.email
         )
         SELECT
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
             -- Every tool call, from Claude or a messaging bot, is logged with
             -- the key owner's email, and with the consumer's when a provider
             -- calls for one. api_keys.last_used is kept in case it is ever written.
             GREATEST(
                 (SELECT max(l.timestamp) FROM api_usage_logs l WHERE l.email = up.email),
                 (SELECT max(l.timestamp) FROM api_usage_logs l WHERE l.consumer_id = up.email),
                 (SELECT max(k.last_used) FROM api_keys k WHERE k.email = up.email),
                 -- The apps' AI calls, made with a licence rather than a key.
                 (SELECT max(u.created_at) FROM llm_usage u WHERE u.email = lower(up.email))
             ) AS last_active,
             (SELECT count(*) FROM channel_identities ci WHERE ci.user_email = up.email) AS linked_channels,
             up.has_account,
             -- Every licence the email holds, live ones first. The licence
             -- number itself stays out: it is what unlocks the apps.
             (SELECT COALESCE(json_agg(json_build_object(
                        'id', s.id, 'plan_id', s.plan_id, 'plan', p.name, 'is_free', s.is_free,
                        'on_stripe', s.stripe_customer_id <> '',
                        'status', s.status, 'live', ({LIVE}),
                        'cancel_at_period_end', s.cancel_at_period_end,
                        'period_end', s.period_end,
                        'credits_used', s.credits_used, 'credits_granted', s.credits_granted)
                        ORDER BY ({LIVE}) DESC, s.is_free, s.created_at DESC), '[]'::json)
                FROM subscriptions s JOIN plans p ON p.id = s.plan_id
               WHERE s.email = lower(up.email) AND s.licence IS NOT NULL) AS licences
         FROM people up
         WHERE ($1 = '' OR up.email ILIKE '%' || $1 || '%'
                OR EXISTS (SELECT 1 FROM tenant_users tu JOIN tenants t ON t.id = tu.tenant_id
                            WHERE tu.email = up.email AND t.name ILIKE '%' || $1 || '%'))
           AND ($2 = '' OR EXISTS (SELECT 1 FROM tenant_users tu
                                    WHERE tu.email = up.email AND tu.tenant_id = $2
                                      AND ($3 = '' OR tu.role = $3)))
           AND ($3 = '' OR $2 <> '' OR EXISTS (SELECT 1 FROM tenant_users tu
                                                WHERE tu.email = up.email AND tu.role = $3))
           AND {licence_where}
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
                "has_account": r.get::<_, bool>(8),
                "licences": r.get::<_, serde_json::Value>(9),
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

    #[test]
    fn licence_filter_is_a_whitelist() {
        assert_eq!(licence_filter("").as_deref(), Some("TRUE"));
        assert!(licence_filter("paid").unwrap().contains("NOT s.is_free"));
        assert!(licence_filter("none").unwrap().starts_with("NOT EXISTS"));
        assert!(licence_filter("x' OR 1=1 --").is_none());
    }

}
