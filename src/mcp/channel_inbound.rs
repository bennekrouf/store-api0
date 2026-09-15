// src/mcp/channel_inbound.rs
//
// The last time a platform actually delivered a message to the bridge, per
// tenant and channel. All behind X-Internal-Secret.
//
//   POST /internal/channel-inbound                    — the bridge, on every message
//   GET  /internal/connectors/{tenant_id}/inbound     — connector tests
//
// Every other trace is written only after something *else* worked: a session
// once a turn succeeded, a dead letter once one failed past identity. A message
// from someone who has not linked yet, or one rate limited, leaves neither — so
// "nobody has messaged this bot" and "the platform never delivered" looked the
// same. This row is written at the door, before any of that, and is the only
// proof the last step of a setup guide worked.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct InboundBody {
    pub tenant_id: String,
    pub channel: String,
    /// Whether the sender was a linked person. `None` when it is not known —
    /// rate limited, or the identity lookup itself failed.
    pub linked: Option<bool>,
}

pub async fn record_inbound_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<InboundBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "record_inbound: no connection");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    let unlinked = body.linked == Some(false);
    let result = client
        .execute(
            "INSERT INTO channel_inbound (tenant_id, channel, last_inbound_at, last_unlinked_at)
             VALUES ($1, $2, NOW(), CASE WHEN $3 THEN NOW() END)
             ON CONFLICT (tenant_id, channel) DO UPDATE SET
                last_inbound_at  = NOW(),
                last_unlinked_at = CASE WHEN $3 THEN NOW() ELSE channel_inbound.last_unlinked_at END",
            &[&body.tenant_id, &body.channel, &unlinked],
        )
        .await
        .to_store_error();

    match result {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({"success": true})),
        Err(e) => {
            // A tenant deleted between delivery and this write lands here on the
            // foreign key; nothing to do about it but say so.
            app_log!(warn, error = %e, tenant_id = %body.tenant_id, "record_inbound failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}))
        }
    }
}

pub async fn tenant_inbound_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant_id = path.into_inner();

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "tenant_inbound: no connection");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}));
        }
    };

    match client
        .query(
            "SELECT channel, last_inbound_at, last_unlinked_at
               FROM channel_inbound WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await
        .to_store_error()
    {
        Ok(rows) => {
            let mut by_channel = serde_json::Map::new();
            for r in rows {
                let channel: String = r.get(0);
                by_channel.insert(
                    channel,
                    serde_json::json!({
                        "last_inbound_at": r.get::<_, chrono::DateTime<chrono::Utc>>(1).to_rfc3339(),
                        "last_unlinked_at": r
                            .get::<_, Option<chrono::DateTime<chrono::Utc>>>(2)
                            .map(|t| t.to_rfc3339()),
                    }),
                );
            }
            HttpResponse::Ok().json(serde_json::json!({"success": true, "inbound": by_channel}))
        }
        Err(e) => {
            app_log!(error, error = %e, tenant_id = %tenant_id, "tenant_inbound failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}))
        }
    }
}
