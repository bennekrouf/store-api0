// src/admin/connectors_overview.rs
//
// Which doors into each tenant exist, and whether anything is walking through them.
//
// Internal (X-Internal-Secret):
//   GET /api/internal/connectors/overview
//   GET /api/internal/connectors/{tenant_id}/messaging-channels
//
// Three connectors, three different traces:
//
//   claude    — MCP tool calls in api_usage_logs, minus the ones the bridge makes
//               with a linked person's key (those belong to WhatsApp/Telegram).
//               Only tool calls are logged, so a client that connects and lists
//               tools but never calls one looks idle here.
//   whatsapp  — whatsapp_channels, sessions keyed "whatsapp:<phone>" (or a bare
//               phone, from before keys were namespaced), dead letters.
//   telegram  — messaging_channels, sessions keyed "telegram:<id>", dead letters.
//
// A session is written only at the end of a turn that succeeded, and a dead
// letter only when one failed, so between them they date the last success and
// the last failure. Read-only, and no credentials: configured or not, never the
// token.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::messaging_channels::list_channels;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use chrono::{DateTime, Duration, Utc};
use std::sync::Arc;

/// How recent a success has to be for a connector to count as in use.
const ACTIVE_WINDOW_DAYS: i64 = 7;
/// How far back activity is read at all. Sessions are cleaned up after 30 days,
/// so looking further would only see MCP calls and be inconsistent.
const LOOKBACK_DAYS: i64 = 30;

/// One verdict per connector, so the dashboard and anything else reading this
/// agree on what "dead" means rather than each guessing.
///
/// Order matters: a failure more recent than the last success outranks any
/// amount of earlier activity, because that is the state a user hits right now.
pub fn connector_status(
    configured: bool,
    last_success: Option<DateTime<Utc>>,
    last_failure: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> &'static str {
    let failing_now = match (last_failure, last_success) {
        (Some(f), Some(s)) => f > s && now - f < Duration::hours(24),
        (Some(f), None) => now - f < Duration::hours(24),
        _ => false,
    };

    if failing_now {
        "failing"
    } else if last_success.is_some_and(|s| now - s < Duration::days(ACTIVE_WINDOW_DAYS)) {
        "active"
    } else if configured {
        "idle"
    } else {
        "not_configured"
    }
}

/// A messaging channel that no longer exists cannot receive anything, so its
/// leftover sessions and dead letters describe a bot that was removed — not a
/// connector that is failing now. Unlike Claude, where an API-key client works
/// with no OAuth configuration at all, there is no unconfigured way in.
pub fn messaging_status(
    configured: bool,
    last_success: Option<DateTime<Utc>>,
    last_failure: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> &'static str {
    if configured {
        connector_status(true, last_success, last_failure, now)
    } else {
        "not_configured"
    }
}

const OVERVIEW_SQL: &str = "
WITH mcp AS (
    SELECT l.tenant_id,
           max(l.timestamp) FILTER (WHERE NOT l.failed)                                   AS last_success,
           max(l.timestamp) FILTER (WHERE l.failed)                                       AS last_failure,
           count(*) FILTER (WHERE l.timestamp > now() - interval '24 hours')              AS calls_24h,
           count(*) FILTER (WHERE l.timestamp > now() - interval '24 hours' AND l.failed) AS errors_24h,
           count(*) FILTER (WHERE l.timestamp > now() - interval '7 days')                AS calls_7d,
           count(DISTINCT l.email) FILTER (WHERE l.timestamp > now() - interval '7 days') AS users_7d
      FROM (SELECT tenant_id, timestamp, email,
                   (COALESCE(response_status, 0) >= 400
                    OR COALESCE(metadata->>'is_error', 'false') = 'true') AS failed
              FROM api_usage_logs ul
             WHERE ul.method = 'MCP'
               AND ul.timestamp > now() - make_interval(days => $1)
               AND ul.tenant_id IS NOT NULL
               -- Calls the bridge makes on a linked person's behalf.
               AND NOT EXISTS (SELECT 1 FROM channel_identities ci WHERE ci.api_key_id = ul.key_id)
           ) l
     GROUP BY l.tenant_id
),
bridge_calls AS (
    SELECT ul.tenant_id, ci.channel, count(*) AS tool_calls_7d
      FROM api_usage_logs ul
      JOIN channel_identities ci ON ci.api_key_id = ul.key_id
     WHERE ul.method = 'MCP' AND ul.timestamp > now() - interval '7 days'
     GROUP BY ul.tenant_id, ci.channel
),
sessions AS (
    SELECT tenant_id,
           CASE WHEN customer_phone LIKE 'telegram:%' THEN 'telegram' ELSE 'whatsapp' END AS channel,
           max(last_active)                                                    AS last_success,
           count(*) FILTER (WHERE last_active > now() - interval '7 days')     AS conversations_7d
      FROM whatsapp_sessions
     GROUP BY 1, 2
),
failures AS (
    -- Rows from before the channel column existed are WhatsApp's: the table
    -- predates every other channel.
    SELECT tenant_id,
           COALESCE(channel, 'whatsapp')                                       AS channel,
           max(created_at)                                                     AS last_failure,
           count(*) FILTER (WHERE created_at > now() - interval '24 hours')    AS failures_24h,
           (array_agg(error_type ORDER BY created_at DESC))[1]                 AS last_failure_type,
           (array_agg(left(error_detail, 300) ORDER BY created_at DESC))[1]    AS last_failure_detail
      FROM whatsapp_failed_messages
     WHERE created_at > now() - make_interval(days => $1)
     GROUP BY 1, 2
),
identities AS (
    SELECT tenant_id, channel, count(*) AS linked
      FROM channel_identities
     GROUP BY 1, 2
),
tools AS (
    SELECT t.id AS tenant_id,
           (SELECT count(*) FROM endpoints e JOIN api_groups g ON e.group_id = g.id
             WHERE g.tenant_id = t.id)
         + (SELECT count(*) FROM mcp_tools m WHERE m.tenant_id = t.id AND m.is_active) AS tool_count
      FROM tenants t
)
SELECT t.id,
       t.name,
       -- claude (MCP)
       t.mcp_client_id,
       (COALESCE(btrim(t.google_client_id), '') <> '' OR t.allow_api0_signin) AS can_sign_in,
       tools.tool_count,
       mcp.last_success, mcp.last_failure,
       COALESCE(mcp.calls_24h, 0), COALESCE(mcp.errors_24h, 0),
       COALESCE(mcp.calls_7d, 0),  COALESCE(mcp.users_7d, 0),
       -- whatsapp
       wa.phone_number_id, wa.created_at,
       ws.last_success, COALESCE(ws.conversations_7d, 0),
       wf.last_failure, COALESCE(wf.failures_24h, 0), wf.last_failure_type, wf.last_failure_detail,
       COALESCE(wi.linked, 0), COALESCE(wb.tool_calls_7d, 0),
       -- telegram
       tg.channel_ref, tg.display_ref, tg.created_at,
       ts.last_success, COALESCE(ts.conversations_7d, 0),
       tf.last_failure, COALESCE(tf.failures_24h, 0), tf.last_failure_type, tf.last_failure_detail,
       COALESCE(ti.linked, 0), COALESCE(tb.tool_calls_7d, 0)
  FROM tenants t
  JOIN tools              ON tools.tenant_id = t.id
  LEFT JOIN mcp           ON mcp.tenant_id = t.id
  LEFT JOIN whatsapp_channels  wa ON wa.tenant_id = t.id
  LEFT JOIN sessions      ws ON ws.tenant_id = t.id AND ws.channel = 'whatsapp'
  LEFT JOIN failures      wf ON wf.tenant_id = t.id AND wf.channel = 'whatsapp'
  LEFT JOIN identities    wi ON wi.tenant_id = t.id AND wi.channel = 'whatsapp'
  LEFT JOIN bridge_calls  wb ON wb.tenant_id = t.id AND wb.channel = 'whatsapp'
  LEFT JOIN messaging_channels tg ON tg.tenant_id = t.id AND tg.channel = 'telegram'
  LEFT JOIN sessions      ts ON ts.tenant_id = t.id AND ts.channel = 'telegram'
  LEFT JOIN failures      tf ON tf.tenant_id = t.id AND tf.channel = 'telegram'
  LEFT JOIN identities    ti ON ti.tenant_id = t.id AND ti.channel = 'telegram'
  LEFT JOIN bridge_calls  tb ON tb.tenant_id = t.id AND tb.channel = 'telegram'
 -- Only tenants with a door, or traffic through one. Every personal tenant
 -- nobody has wired up would otherwise bury the handful that matter.
 WHERE COALESCE(btrim(t.mcp_client_id), '') <> ''
    OR mcp.tenant_id IS NOT NULL
    OR wa.tenant_id IS NOT NULL
    OR tg.tenant_id IS NOT NULL
    OR ws.tenant_id IS NOT NULL
    OR ts.tenant_id IS NOT NULL
    OR wf.tenant_id IS NOT NULL
    OR tf.tenant_id IS NOT NULL
 ORDER BY t.name";

type Ts = Option<DateTime<Utc>>;

fn rfc(t: Ts) -> Option<String> {
    t.map(|t| t.to_rfc3339())
}

/// GET /api/internal/connectors/overview
pub async fn connectors_overview(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "connectors_overview: no connection");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    let lookback = LOOKBACK_DAYS as i32;
    let rows = match client.query(OVERVIEW_SQL, &[&lookback]).await.to_store_error() {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, "connectors_overview: query failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    let now = Utc::now();

    let tenants: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            // ── claude ──────────────────────────────────────────────────────
            let client_id: Option<String> = r.get(2);
            let has_client_id = client_id.as_deref().is_some_and(|s| !s.trim().is_empty());
            let can_sign_in: bool = r.get(3);
            let tool_count: i64 = r.get(4);
            let (mcp_ok, mcp_fail): (Ts, Ts) = (r.get(5), r.get(6));
            // OAuth is the only thing that makes Claude.ai able to connect, so a
            // client id without a sign-in method is a half-built door. An API-key
            // client (Claude Desktop, Cursor) needs neither, which is why traffic
            // still counts as active below regardless.
            let claude_ready = has_client_id && can_sign_in;
            let mut claude_status = connector_status(claude_ready, mcp_ok, mcp_fail, now);
            if claude_status == "not_configured" && has_client_id && !can_sign_in {
                claude_status = "misconfigured";
            }

            // ── whatsapp ────────────────────────────────────────────────────
            let wa_number: Option<String> = r.get(11);
            let (wa_ok, wa_fail): (Ts, Ts) = (r.get(13), r.get(15));
            let wa_status = messaging_status(wa_number.is_some(), wa_ok, wa_fail, now);

            // ── telegram ────────────────────────────────────────────────────
            let tg_bot: Option<String> = r.get(21);
            let (tg_ok, tg_fail): (Ts, Ts) = (r.get(24), r.get(26));
            let tg_status = messaging_status(tg_bot.is_some(), tg_ok, tg_fail, now);

            serde_json::json!({
                "tenant_id": r.get::<_, String>(0),
                "tenant_name": r.get::<_, String>(1),
                "claude": {
                    "status": claude_status,
                    "configured": claude_ready,
                    "client_id": client_id,
                    "can_sign_in": can_sign_in,
                    "tool_count": tool_count,
                    "last_success_at": rfc(mcp_ok),
                    "last_failure_at": rfc(mcp_fail),
                    "calls_24h": r.get::<_, i64>(7),
                    "errors_24h": r.get::<_, i64>(8),
                    "calls_7d": r.get::<_, i64>(9),
                    "users_7d": r.get::<_, i64>(10),
                },
                "whatsapp": {
                    "status": wa_status,
                    "configured": wa_number.is_some(),
                    "display_ref": wa_number,
                    "channel_ref": null,
                    "configured_at": rfc(r.get(12)),
                    "last_success_at": rfc(wa_ok),
                    "conversations_7d": r.get::<_, i64>(14),
                    "last_failure_at": rfc(wa_fail),
                    "failures_24h": r.get::<_, i64>(16),
                    "last_failure_type": r.get::<_, Option<String>>(17),
                    "last_failure_detail": r.get::<_, Option<String>>(18),
                    "linked_identities": r.get::<_, i64>(19),
                    "tool_calls_7d": r.get::<_, i64>(20),
                },
                "telegram": {
                    "status": tg_status,
                    "configured": tg_bot.is_some(),
                    "display_ref": r.get::<_, Option<String>>(22).filter(|s| !s.is_empty()).map(|u| format!("@{}", u)),
                    // The bot id is public — it is in the webhook URL.
                    "channel_ref": tg_bot,
                    "configured_at": rfc(r.get(23)),
                    "last_success_at": rfc(tg_ok),
                    "conversations_7d": r.get::<_, i64>(25),
                    "last_failure_at": rfc(tg_fail),
                    "failures_24h": r.get::<_, i64>(27),
                    "last_failure_type": r.get::<_, Option<String>>(28),
                    "last_failure_detail": r.get::<_, Option<String>>(29),
                    "linked_identities": r.get::<_, i64>(30),
                    "tool_calls_7d": r.get::<_, i64>(31),
                },
            })
        })
        .collect();

    app_log!(info, count = tenants.len(), "Served the connectors overview");

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "generated_at": now.to_rfc3339(),
        "active_window_days": ACTIVE_WINDOW_DAYS,
        "lookback_days": LOOKBACK_DAYS,
        "tenants": tenants,
    }))
}

/// GET /api/internal/connectors/{tenant_id}/messaging-channels
///
/// A tenant's bots by tenant id. The tenant-facing list resolves the caller's
/// default tenant from an email, which is no use to an operator testing someone
/// else's. Summaries only — the credential stays behind the bridge's own route.
pub async fn messaging_channels_for_tenant(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant_id = path.into_inner();
    match list_channels(&store, &tenant_id).await {
        Ok(channels) => {
            HttpResponse::Ok().json(serde_json::json!({"success": true, "channels": channels}))
        }
        Err(e) => {
            app_log!(error, error = %e, tenant_id = %tenant_id, "messaging_channels_for_tenant failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ago(h: i64) -> Option<DateTime<Utc>> {
        Some(Utc::now() - Duration::hours(h))
    }

    #[test]
    fn nothing_set_up_and_nothing_seen_is_not_configured() {
        assert_eq!(connector_status(false, None, None, Utc::now()), "not_configured");
    }

    #[test]
    fn configured_but_silent_is_idle() {
        assert_eq!(connector_status(true, None, None, Utc::now()), "idle");
        // A success older than the active window no longer counts.
        assert_eq!(connector_status(true, ago(24 * 10), None, Utc::now()), "idle");
    }

    #[test]
    fn a_recent_success_is_active_even_without_configuration() {
        // An API-key MCP client needs no OAuth client id and still works.
        assert_eq!(connector_status(false, ago(2), None, Utc::now()), "active");
    }

    #[test]
    fn a_failure_after_the_last_success_is_failing() {
        assert_eq!(connector_status(true, ago(5), ago(1), Utc::now()), "failing");
        assert_eq!(connector_status(true, None, ago(1), Utc::now()), "failing");
    }

    #[test]
    fn a_failure_followed_by_a_success_has_recovered() {
        assert_eq!(connector_status(true, ago(1), ago(5), Utc::now()), "active");
    }

    #[test]
    fn a_removed_messaging_channel_is_not_configured_whatever_it_left_behind() {
        assert_eq!(messaging_status(false, ago(1), ago(0), Utc::now()), "not_configured");
        assert_eq!(messaging_status(true, ago(5), ago(1), Utc::now()), "failing");
    }

    #[test]
    fn an_old_failure_does_not_keep_a_connector_failing() {
        assert_eq!(connector_status(true, None, ago(48), Utc::now()), "idle");
    }
}
