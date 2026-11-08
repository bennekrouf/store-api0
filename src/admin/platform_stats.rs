// src/admin/platform_stats.rs
//
// The super admin's figures at a glance: counts for the strip at the top of
// the admin view, and the AI calls of today, the last 7 days and this year for
// the LLM tab. Counted in SQL; nothing but the numbers leaves the database.
//
//   GET /api/admin/stats
//   → { users: { total, accounts, new_7d }, workspaces: { total, new_7d, on_plan },
//       licences: { live, paid, free } }
//       users       people as the Users tab lists them: an api0 account, or an
//                   email holding a licence without one
//       on_plan     workspaces with a live workspace plan
//       licences    live ones only: active or trialing, in their paid month
//                   (a free one is live whatever its month, as in users_directory.rs)
//
//   GET /api/admin/llm-stats?tz=Europe/Zurich
//   → { tz, periods: [{ period: today | 7d | year, since, entries, calls, failed,
//                       input_tokens, output_tokens, credits, users,
//                       by_kind: { licence, platform, assistant } }] }
//       today and year start at midnight in `tz` (UTC when absent or unknown);
//       7d is the last 168 hours. `entries` are ledger rows as the LLM tab lists
//       them; `calls` counts each completion of a bot turn.
//
// Both need X-Internal-Secret; the gateway restricts them to the super admin.

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

/// A licence that gives access now (users_directory.rs LIVE).
const LIVE: &str = "s.licence IS NOT NULL AND s.status IN ('active', 'trialing') AND (s.period_end > NOW() OR s.is_free)";

pub async fn platform_stats(req: HttpRequest, store: web::Data<Arc<EndpointStore>>) -> impl Responder {
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

    let sql = format!(
        "WITH people AS (
             SELECT created_at FROM user_preferences
             UNION ALL
             SELECT min(s.created_at)
               FROM subscriptions s
              WHERE s.licence IS NOT NULL
                AND NOT EXISTS (SELECT 1 FROM user_preferences a WHERE lower(a.email) = s.email)
              GROUP BY s.email
         )
         SELECT
             (SELECT count(*) FROM people),
             (SELECT count(*) FROM user_preferences),
             (SELECT count(*) FROM people WHERE created_at > NOW() - INTERVAL '7 days'),
             (SELECT count(*) FROM tenants),
             (SELECT count(*) FROM tenants WHERE created_at > NOW() - INTERVAL '7 days'),
             (SELECT count(DISTINCT ws.tenant_id) FROM workspace_subscriptions ws
               WHERE ws.status IN ('active', 'trialing') AND ws.period_end > NOW()),
             (SELECT count(*) FROM subscriptions s WHERE {LIVE}),
             (SELECT count(*) FROM subscriptions s WHERE {LIVE} AND NOT s.is_free)"
    );

    let row = match client.query_one(&sql, &[]).await {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, "Admin stats query failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Query failed"}));
        }
    };
    let n = |i: usize| row.get::<_, i64>(i);
    let (live, paid) = (n(6), n(7));

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "users": { "total": n(0), "accounts": n(1), "new_7d": n(2) },
        "workspaces": { "total": n(3), "new_7d": n(4), "on_plan": n(5) },
        "licences": { "live": live, "paid": paid, "free": live - paid },
    }))
}

#[derive(Debug, Deserialize)]
pub struct LlmStatsQuery {
    pub tz: Option<String>,
}

/// An IANA zone name the browser sent, or None for anything that is not shaped
/// like one. Postgres still has the last word on whether it exists.
fn zone(tz: Option<&str>) -> Option<String> {
    let tz = tz?.trim();
    let shaped = !tz.is_empty()
        && tz.len() <= 64
        && tz.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '+' | '-'));
    shaped.then(|| tz.to_string())
}

// The ledger of llm_calls.rs, cut to what the sums need and to this year (or
// the last 7 days, early in January). Each period then sums the rows since its
// start; a period with none still comes back, with zeros.
const LLM_SQL: &str = "
    WITH b AS (
        SELECT date_trunc('day', NOW() AT TIME ZONE $1) AT TIME ZONE $1 AS day0,
               NOW() - INTERVAL '7 days' AS week0,
               date_trunc('year', NOW() AT TIME ZONE $1) AT TIME ZONE $1 AS year0
    ),
    calls AS (
        SELECT u.created_at AS at,
               CASE WHEN u.license_id IS NULL THEN 'platform' ELSE 'licence' END AS kind,
               u.email, u.input_tokens, u.output_tokens, u.credits, u.status_code AS status, 1 AS calls
          FROM llm_usage u, b
         WHERE u.created_at >= LEAST(b.day0, b.week0, b.year0)
        UNION ALL
        SELECT l.timestamp, 'assistant', l.email,
               COALESCE(l.input_tokens, 0), COALESCE(l.output_tokens, 0), NULL::BIGINT,
               COALESCE(l.response_status, 0), COALESCE((l.metadata->>'llm_calls')::INT, 1)
          FROM api_usage_logs l, b
         WHERE l.method = 'LLM' AND l.timestamp >= LEAST(b.day0, b.week0, b.year0)
    ),
    periods AS (
        SELECT 'today' AS period, 1 AS ord, day0 AS since FROM b
        UNION ALL SELECT '7d', 2, week0 FROM b
        UNION ALL SELECT 'year', 3, year0 FROM b
    )
    SELECT p.period, p.since,
           count(c.at),
           COALESCE(sum(c.calls), 0)::BIGINT,
           count(c.at) FILTER (WHERE c.status NOT BETWEEN 200 AND 299),
           COALESCE(sum(c.input_tokens), 0)::BIGINT,
           COALESCE(sum(c.output_tokens), 0)::BIGINT,
           COALESCE(sum(c.credits), 0)::BIGINT,
           count(DISTINCT c.email) FILTER (WHERE c.email <> ''),
           count(c.at) FILTER (WHERE c.kind = 'licence'),
           count(c.at) FILTER (WHERE c.kind = 'platform'),
           count(c.at) FILTER (WHERE c.kind = 'assistant')
      FROM periods p
      LEFT JOIN calls c ON c.at >= p.since
     GROUP BY p.period, p.ord, p.since
     ORDER BY p.ord";

pub async fn llm_stats(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<LlmStatsQuery>,
) -> impl Responder {
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

    // A zone Postgres does not know fails the query: count in UTC instead.
    let mut tz = zone(query.tz.as_deref()).unwrap_or_else(|| "UTC".to_string());
    let mut result = client.query(LLM_SQL, &[&tz]).await;
    if result.is_err() && tz != "UTC" {
        tz = "UTC".to_string();
        result = client.query(LLM_SQL, &[&tz]).await;
    }
    let rows = match result {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, "LLM stats query failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Query failed"}));
        }
    };

    let periods: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let since: chrono::DateTime<chrono::Utc> = r.get(1);
            serde_json::json!({
                "period": r.get::<_, String>(0),
                "since": since.to_rfc3339(),
                "entries": r.get::<_, i64>(2),
                "calls": r.get::<_, i64>(3),
                "failed": r.get::<_, i64>(4),
                "input_tokens": r.get::<_, i64>(5),
                "output_tokens": r.get::<_, i64>(6),
                "credits": r.get::<_, i64>(7),
                "users": r.get::<_, i64>(8),
                "by_kind": {
                    "licence": r.get::<_, i64>(9),
                    "platform": r.get::<_, i64>(10),
                    "assistant": r.get::<_, i64>(11),
                },
            })
        })
        .collect();

    HttpResponse::Ok().json(serde_json::json!({ "success": true, "tz": tz, "periods": periods }))
}

#[cfg(test)]
mod tests {
    use super::zone;

    #[test]
    fn zone_accepts_iana_names_only() {
        assert_eq!(zone(Some("Europe/Zurich")).as_deref(), Some("Europe/Zurich"));
        assert_eq!(zone(Some(" America/Argentina/Buenos_Aires ")).as_deref(), Some("America/Argentina/Buenos_Aires"));
        assert_eq!(zone(Some("Etc/GMT+1")).as_deref(), Some("Etc/GMT+1"));
        assert_eq!(zone(None), None);
        assert_eq!(zone(Some("")), None);
        assert_eq!(zone(Some("UTC'; DROP TABLE x")), None);
    }
}
