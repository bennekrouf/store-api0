// src/api/tenant_usage.rs
//
// GET /api/tenant/stats/{email}?hours=24&limit=50&offset=0
//
// Returns governance/security stats for the tenant that owns `email`:
//   - summary KPIs (total requests, success/fail, avg latency, unique consumers/tools)
//   - paginated request log joined with key_prefix from api_keys
//
// hours = 0  → all time (no timestamp filter)
// hours > 0  → last N hours
// limit max 200, default 50

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

pub async fn get_tenant_stats(
    store: web::Data<Arc<EndpointStore>>,
    path_email: web::Path<String>,
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let email = path_email.into_inner();
    let hours: i64 = query
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

    let client = match store.get_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "DB connection failed for stats");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": "Database error"
            }));
        }
    };

    // ── Summary KPIs ─────────────────────────────────────────────────────────
    // Cast $2 to int4 for make_interval — i64 maps to BIGINT but make_interval
    // takes INTEGER. hours is always small (≤ 720) so the cast is safe.
    //
    // COUNT(DISTINCT NULLIF(consumer_id,'')) treats empty-string as NULL,
    // which is cleaner than the FILTER trick that had a cast-placement bug.
    // SUM returns NULL when there are no matching rows — always COALESCE to 0.
    // COUNT returns 0 naturally. AVG returns NULL on empty → COALESCE handles it.
    // make_interval(hours => $2::INT) avoids the missing bigint×interval operator.
    let summary_sql_base =
        "SELECT
            COUNT(*)::BIGINT                                                     AS total_requests,
            COALESCE(SUM(CASE WHEN response_status < 400 THEN 1 ELSE 0 END), 0)::BIGINT
                                                                                 AS successful,
            COALESCE(SUM(CASE WHEN response_status >= 400
                               OR  response_status IS NULL THEN 1 ELSE 0 END), 0)::BIGINT
                                                                                 AS failed,
            ROUND(COALESCE(AVG(response_time_ms), 0))::BIGINT                   AS avg_latency_ms,
            COUNT(DISTINCT NULLIF(consumer_id, ''))::BIGINT                     AS unique_consumers,
            COUNT(DISTINCT endpoint_path)::BIGINT                                AS unique_tools
         FROM api_usage_logs
         WHERE tenant_id = $1";

    let summary_row = if hours == 0 {
        client.query_one(summary_sql_base, &[&tenant_id]).await
    } else {
        let sql = format!("{} AND timestamp >= NOW() - make_interval(hours => $2::INT)", summary_sql_base);
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

    // COUNT() returns i64, ROUND(AVG)::BIGINT returns i64
    let total_requests: i64 = summary_row.get(0);
    let successful: i64     = summary_row.get(1);
    let failed: i64         = summary_row.get(2);
    let avg_latency_ms: i64 = summary_row.get(3);
    let unique_consumers: i64 = summary_row.get(4);
    let unique_tools: i64   = summary_row.get(5);

    let success_rate: f64 = if total_requests > 0 {
        ((successful as f64 / total_requests as f64) * 1000.0).round() / 10.0
    } else {
        0.0
    };

    // ── Paginated request log ─────────────────────────────────────────────────
    let log_rows = if hours == 0 {
        client.query(
            "SELECT
                l.id,
                l.timestamp,
                l.endpoint_path,
                l.method,
                l.response_status,
                l.response_time_ms,
                l.consumer_id,
                l.total_tokens,
                l.metadata,
                COALESCE(k.key_prefix, '') AS key_prefix,
                COALESCE(k.key_name,   '') AS key_name
             FROM api_usage_logs l
             LEFT JOIN api_keys k ON l.key_id = k.id
             WHERE l.tenant_id = $1
             ORDER BY l.timestamp DESC
             LIMIT $2 OFFSET $3",
            &[&tenant_id, &limit, &offset],
        ).await
    } else {
        client.query(
            "SELECT
                l.id,
                l.timestamp,
                l.endpoint_path,
                l.method,
                l.response_status,
                l.response_time_ms,
                l.consumer_id,
                l.total_tokens,
                l.metadata,
                COALESCE(k.key_prefix, '') AS key_prefix,
                COALESCE(k.key_name,   '') AS key_name
             FROM api_usage_logs l
             LEFT JOIN api_keys k ON l.key_id = k.id
             WHERE l.tenant_id = $1
               AND l.timestamp >= NOW() - make_interval(hours => $4::INT)
             ORDER BY l.timestamp DESC
             LIMIT $2 OFFSET $3",
            &[&tenant_id, &limit, &offset, &hours],
        ).await
    };

    let log_rows = match log_rows {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, tenant_id = %tenant_id, "Stats log query failed");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Log query error: {}", e)
            }));
        }
    };

    let logs: Vec<serde_json::Value> = log_rows
        .iter()
        .map(|row| {
            let ts: chrono::DateTime<chrono::Utc> = row.get(1);
            // metadata is stored as JSONB — try get as Value first, else try String
            let metadata: Option<serde_json::Value> = row
                .try_get::<_, serde_json::Value>(8)
                .ok()
                .filter(|v| !v.is_null());

            serde_json::json!({
                "id":               row.get::<_, String>(0),
                "timestamp":        ts.to_rfc3339(),
                "endpoint_path":    row.get::<_, String>(2),
                "method":           row.get::<_, String>(3),
                "response_status":  row.get::<_, Option<i32>>(4),
                "response_time_ms": row.get::<_, Option<i64>>(5),
                "consumer_id":      row.get::<_, Option<String>>(6),
                "total_tokens":     row.get::<_, Option<i64>>(7),
                "metadata":         metadata,
                "key_prefix":       row.get::<_, String>(9),
                "key_name":         row.get::<_, String>(10),
            })
        })
        .collect();

    app_log!(info,
        email = %email,
        tenant_id = %tenant_id,
        total_requests = total_requests,
        logs_returned = logs.len(),
        "Tenant stats returned"
    );

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "summary": {
            "total_requests":   total_requests,
            "successful":       successful,
            "failed":           failed,
            "success_rate":     success_rate,
            "avg_latency_ms":   avg_latency_ms,
            "unique_consumers": unique_consumers,
            "unique_tools":     unique_tools,
            "period_hours":     hours,
        },
        "logs":        logs,
        "total_count": total_requests,
    }))
}
