// src/api/tenant_usage.rs
//
// GET /api/tenant/stats/{email}?hours=24&limit=50&offset=0
//
// Returns governance stats for the tenant that owns `email`:
//   - MCP summary KPIs (total MCP calls, success/fail, avg latency, unique consumers/tools)
//   - total_credit: Real financial/top-up events in the same window
//   - unified paginated `events` array: MCP tool calls UNION ALL important Tenant events,
//     each row tagged with source = "mcp" | "event"
//
// MCP rows come from api_usage_logs.
// Event rows come from credit_transactions WHERE action_type = 'topup' or 'stripe_topup'
// (Internal deductions like 'cv_generation' are excluded to avoid noise in the protocol monitor).
//
// hours = 0  → all time (no timestamp filter)
// hours > 0  → last N hours
// limit max 200, default 50

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// ── Shared SELECT fragments ────────────────────────────────────────────────────
//
// Both halves of the UNION ALL expose 14 columns in the same order and types:
//   0  id            TEXT
//   1  timestamp     TIMESTAMPTZ
//   2  source        TEXT  ('mcp' | 'web')
//   3  name          TEXT  (endpoint_path or action_type)
//   4  consumer_id   TEXT  (nullable — MCP only)
//   5  key_prefix    TEXT  (nullable — MCP only)
//   6  key_name      TEXT  (nullable — MCP only)
//   7  status_code   INT4  (nullable — MCP only)
//   8  latency_ms    INT8  (nullable — MCP only)
//   9  tokens        INT8  (nullable — MCP only)
//  10  amount        INT8  (nullable — Web only)
//  11  balance_after INT8  (nullable — Web only)
//  12  web_user      TEXT  (nullable — Web only)
//  13  description   TEXT  (nullable — Web only)

const MCP_SEL: &str =
    "SELECT
        l.id,
        l.timestamp,
        'mcp'                          AS source,
        l.endpoint_path                AS name,
        l.consumer_id,
        k.key_prefix,
        k.key_name,
        l.response_status              AS status_code,
        l.response_time_ms             AS latency_ms,
        l.total_tokens                 AS tokens,
        NULL::BIGINT                   AS amount,
        NULL::BIGINT                   AS balance_after,
        NULL::TEXT                     AS web_user,
        NULL::TEXT                     AS description
     FROM api_usage_logs l
     LEFT JOIN api_keys k ON l.key_id = k.id
     WHERE l.tenant_id = $1";

const WEB_SEL: &str =
    "SELECT
        ct.id::TEXT,
        ct.created_at                  AS timestamp,
        'event'                        AS source,
        ct.action_type                 AS name,
        NULL::TEXT                     AS consumer_id,
        NULL::TEXT                     AS key_prefix,
        NULL::TEXT                     AS key_name,
        NULL::INT                      AS status_code,
        NULL::BIGINT                   AS latency_ms,
        NULL::BIGINT                   AS tokens,
        ct.amount,
        ct.balance_after,
        ct.email                       AS web_user,
        ct.description
     FROM credit_transactions ct
     WHERE ct.tenant_id = $1
       AND ct.action_type IN ('topup', 'stripe_topup', 'welcome')";

pub async fn get_tenant_stats(
    store: web::Data<Arc<EndpointStore>>,
    path_email: web::Path<String>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let email = path_email.into_inner();
    // hours: i32 (INT4) — make_interval(hours => $n) expects INT4.
    // limit / offset: i64 (INT8) — PostgreSQL infers LIMIT $n / OFFSET $n as INT8.
    // Mixing these up causes "cannot convert i32 ↔ int8" at bind time.
    let hours: i32 = query
        .get("hours")
        .and_then(|h| h.parse().ok())
        .unwrap_or(24);
    let limit: i64 = query
        .get("limit")
        .and_then(|l| l.parse().ok())
        .unwrap_or(50)
        .min(200);
    let offset: i64 = query
        .get("offset")
        .and_then(|o| o.parse().ok())
        .unwrap_or(0);

    app_log!(info,
        email = %email,
        hours = hours,
        limit = limit,
        offset = offset,
        "Tenant stats request"
    );

    // Resolve tenant_id from email
    let tenant_id = match crate::endpoint_store::tenant_management::get_default_tenant(
        &store, &email,
    )
    .await
    {
        Ok(t) => t.id,
        Err(e) => {
            app_log!(error, error = %e, email = %email, "Failed to resolve tenant for stats");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Tenant resolution failed: {}", e)
            }));
        }
    };

    let client = match store.get_conn(Some(&tenant_id)).await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "DB connection failed for stats");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": "Database error"
            }));
        }
    };

    // ── MCP Summary KPIs ──────────────────────────────────────────────────────
    let summary_sql_base =
        "SELECT
            COUNT(*)::BIGINT                                                     AS total_mcp,
            COALESCE(SUM(CASE WHEN response_status < 400 THEN 1 ELSE 0 END), 0)::BIGINT
                                                                                 AS successful,
            COALESCE(SUM(CASE WHEN response_status >= 400
                               OR  response_status IS NULL THEN 1 ELSE 0 END), 0)::BIGINT
                                                                                 AS failed,
            ROUND(COALESCE(AVG(response_time_ms), 0))::BIGINT                   AS avg_latency_ms,
            COUNT(DISTINCT NULLIF(consumer_id, ''))::BIGINT                     AS unique_consumers,
            COUNT(DISTINCT endpoint_path)::BIGINT                               AS unique_tools
         FROM api_usage_logs
         WHERE tenant_id = $1";

    let summary_row = if hours == 0 {
        client.query_one(summary_sql_base, &[&tenant_id]).await
    } else {
        let sql = format!("{} AND timestamp >= NOW() - make_interval(hours => $2)", summary_sql_base);
        client.query_one(&sql, &[&tenant_id, &hours]).await
    };

    let summary_row = match summary_row {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, tenant_id = %tenant_id, "Stats summary query failed");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Summary query error: {}", e)
            }));
        }
    };

    let total_mcp: i64        = summary_row.get(0);
    let successful: i64       = summary_row.get(1);
    let failed: i64           = summary_row.get(2);
    let avg_latency_ms: i64   = summary_row.get(3);
    let unique_consumers: i64 = summary_row.get(4);
    let unique_tools: i64     = summary_row.get(5);

    let success_rate: f64 = if total_mcp > 0 {
        ((successful as f64 / total_mcp as f64) * 1000.0).round() / 10.0
    } else {
        0.0
    };

    // ── Web credit events count ───────────────────────────────────────────────
    let credit_count: i64 = {
        let res = if hours == 0 {
            client.query_one(
                "SELECT COUNT(*)::BIGINT FROM credit_transactions \
                 WHERE tenant_id = $1 AND action_type IN ('topup', 'stripe_topup', 'welcome')",
                &[&tenant_id],
            ).await
        } else {
            client.query_one(
                "SELECT COUNT(*)::BIGINT FROM credit_transactions \
                 WHERE tenant_id = $1 AND action_type IN ('topup', 'stripe_topup', 'welcome') \
                   AND created_at >= NOW() - make_interval(hours => $2)",
                &[&tenant_id, &hours],
            ).await
        };
        match res {
            Ok(r) => r.get(0),
            Err(e) => {
                app_log!(warn, error = %e, "Credit count query failed, defaulting 0");
                0i64
            }
        }
    };

    let total_count = total_mcp + credit_count;

    // ── Unified activity log (UNION ALL, sorted, paginated) ──────────────────
    //
    // $1 = tenant_id (shared by both halves)
    // $2 = limit, $3 = offset
    // $4 = hours (only in the time-filtered branch, shared by both halves)
    let events_rows = if hours == 0 {
        let sql = format!(
            "{} UNION ALL {} ORDER BY timestamp DESC LIMIT $2 OFFSET $3",
            MCP_SEL, WEB_SEL
        );
        client.query(&sql, &[&tenant_id, &limit, &offset]).await
    } else {
        let sql = format!(
            "{mcp} AND l.timestamp  >= NOW() - make_interval(hours => $4) \
             UNION ALL \
             {web} AND ct.created_at >= NOW() - make_interval(hours => $4) \
             ORDER BY timestamp DESC LIMIT $2 OFFSET $3",
            mcp = MCP_SEL,
            web = WEB_SEL,
        );
        client.query(&sql, &[&tenant_id, &limit, &offset, &hours]).await
    };

    let events_rows = match events_rows {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, tenant_id = %tenant_id, "Activity log query failed");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Activity query error: {}", e)
            }));
        }
    };

    let events: Vec<serde_json::Value> = events_rows
        .iter()
        .map(|row| {
            let ts: chrono::DateTime<chrono::Utc> = row.get(1);
            serde_json::json!({
                "id":           row.get::<_, String>(0),
                "timestamp":    ts.to_rfc3339(),
                "source":       row.get::<_, String>(2),
                "name":         row.get::<_, Option<String>>(3),
                "consumer_id":  row.get::<_, Option<String>>(4),
                "key_prefix":   row.get::<_, Option<String>>(5),
                "key_name":     row.get::<_, Option<String>>(6),
                "status_code":  row.get::<_, Option<i32>>(7),
                "latency_ms":   row.get::<_, Option<i64>>(8),
                "tokens":       row.get::<_, Option<i64>>(9),
                "amount":       row.get::<_, Option<i64>>(10),
                "balance_after":row.get::<_, Option<i64>>(11),
                "web_user":     row.get::<_, Option<String>>(12),
                "description":  row.get::<_, Option<String>>(13),
            })
        })
        .collect();

    app_log!(info,
        email = %email,
        tenant_id = %tenant_id,
        total_mcp = total_mcp,
        total_credit = credit_count,
        events_returned = events.len(),
        "Tenant stats returned"
    );

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "summary": {
            "total_mcp":        total_mcp,
            "successful":       successful,
            "failed":           failed,
            "success_rate":     success_rate,
            "avg_latency_ms":   avg_latency_ms,
            "unique_consumers": unique_consumers,
            "unique_tools":     unique_tools,
            "total_credit":     credit_count,
            "period_hours":     hours,
        },
        "events":      events,
        "total_count": total_count,
    }))
}
