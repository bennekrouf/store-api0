// src/admin/llm_calls.rs
//
// The super admin's ledger of every call api0 makes to an AI provider: which
// model, how many tokens went in and came out. Three kinds, from two tables:
//
//   licence    a desktop app through the gateway's /v1/chat/completions,
//              paid from a plan (llm_usage, with a licence)
//   platform   api0's own calls — YAML and reference-data imports
//              (llm_usage, no licence, no credits)
//   assistant  one Telegram/WhatsApp/Teams bot turn, which may take several
//              completions; `calls` says how many (api_usage_logs, method LLM)
//
//   GET /api/admin/llm-calls?q=&kind=&tenant_id=&page=&page_size=
//       q          matches the email, the model or the app
//       kind       licence | platform | assistant
//   → { calls: [...], total, input_tokens, output_tokens } — the sums cover
//     the whole filtered set, not just the page.
//
// Needs X-Internal-Secret; the gateway restricts it to the super admin.

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

const MAX_PAGE_SIZE: i64 = 200;
const KINDS: &[&str] = &["licence", "platform", "assistant"];

#[derive(Debug, Deserialize)]
pub struct LlmCallsQuery {
    pub q: Option<String>,
    pub kind: Option<String>,
    pub tenant_id: Option<String>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

// Both tables in one shape, newest first. The filters and the sums run in SQL
// so only the page leaves the database.
const SQL: &str = "
    WITH calls AS (
        SELECT id, created_at AS at,
               CASE WHEN license_id IS NULL THEN 'platform' ELSE 'licence' END AS kind,
               product AS app, email, tenant_id, provider, model,
               input_tokens, output_tokens, total_tokens, estimated,
               credits, status_code, response_time_ms, 1 AS calls, region
          FROM llm_usage
        UNION ALL
        SELECT id, timestamp, 'assistant', endpoint_path, email, tenant_id,
               COALESCE(metadata->>'provider', ''), COALESCE(model_used, ''),
               COALESCE(input_tokens, 0), COALESCE(output_tokens, 0), COALESCE(total_tokens, 0),
               COALESCE(usage_estimated, false), NULL::BIGINT, COALESCE(response_status, 0),
               response_time_ms, COALESCE((metadata->>'llm_calls')::INT, 1), NULL::VARCHAR
          FROM api_usage_logs
         WHERE method = 'LLM'
    )
    SELECT count(*) OVER (), (sum(c.input_tokens) OVER ())::BIGINT, (sum(c.output_tokens) OVER ())::BIGINT,
           c.id, c.at, c.kind, c.app, c.email, c.tenant_id, t.name, c.provider, c.model,
           c.input_tokens, c.output_tokens, c.total_tokens, c.estimated, c.credits,
           c.status_code, c.response_time_ms, c.calls, c.region
      FROM calls c
      LEFT JOIN tenants t ON t.id = c.tenant_id
     WHERE ($1 = '' OR c.email ILIKE '%' || $1 || '%' OR c.model ILIKE '%' || $1 || '%'
                    OR c.app ILIKE '%' || $1 || '%')
       AND ($2 = '' OR c.kind = $2)
       AND ($3 = '' OR c.tenant_id = $3)
     ORDER BY c.at DESC
     LIMIT $4 OFFSET $5";

pub async fn list_llm_calls(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<LlmCallsQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let trimmed = |v: &Option<String>| v.as_deref().map(str::trim).unwrap_or("").to_string();
    let search = trimmed(&query.q);
    let kind = trimmed(&query.kind);
    let tenant_id = trimmed(&query.tenant_id);
    if !kind.is_empty() && !KINDS.contains(&kind.as_str()) {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"success": false, "error": format!("Unknown kind '{}'", kind)}));
    }
    let page_size = query.page_size.unwrap_or(50).clamp(1, MAX_PAGE_SIZE);
    let offset = (query.page.unwrap_or(1).max(1) - 1) * page_size;

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Database unavailable"}))
        }
    };
    let rows = match client.query(SQL, &[&search, &kind, &tenant_id, &page_size, &offset]).await {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, "LLM call ledger query failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Query failed"}));
        }
    };

    let first = rows.first();
    let total: i64 = first.map(|r| r.get(0)).unwrap_or(0);
    let input_tokens: i64 = first.and_then(|r| r.get::<_, Option<i64>>(1)).unwrap_or(0);
    let output_tokens: i64 = first.and_then(|r| r.get::<_, Option<i64>>(2)).unwrap_or(0);
    let calls: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let at: chrono::DateTime<chrono::Utc> = r.get(4);
            let status: i32 = r.get(17);
            serde_json::json!({
                "id": r.get::<_, String>(3),
                "at": at.to_rfc3339(),
                "kind": r.get::<_, String>(5),
                "app": r.get::<_, String>(6),
                "email": r.get::<_, String>(7),
                "tenant_id": r.get::<_, Option<String>>(8),
                "tenant": r.get::<_, Option<String>>(9),
                "provider": r.get::<_, String>(10),
                "model": r.get::<_, String>(11),
                "input_tokens": r.get::<_, i64>(12),
                "output_tokens": r.get::<_, i64>(13),
                "total_tokens": r.get::<_, i64>(14),
                "estimated": r.get::<_, bool>(15),
                "credits": r.get::<_, Option<i64>>(16),
                "ok": (200..300).contains(&status),
                "response_time_ms": r.get::<_, Option<i64>>(18),
                "calls": r.get::<_, i32>(19),
                // The region a licence call chose: what its user saw instead of the model.
                "region": r.get::<_, Option<String>>(20),
            })
        })
        .collect();

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "calls": calls,
        "total": total,
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
    }))
}
