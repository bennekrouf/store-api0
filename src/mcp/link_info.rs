// src/mcp/link_info.rs
//
// What a workspace's branded linking page and its bot need to know about it:
//
//   GET /api/internal/link-info/{ref}[?email=]   (X-Internal-Secret)
//
// `ref` is the workspace's OAuth client id or its tenant id, so every workspace
// has a page even before it chose a client id. `link_ref` is what its page
// address should use: the client id when set (readable), else the tenant id.
// Nothing here is secret — a bot's username is public — but it is internal so
// that only the gateway and the bridge shape what is shown.
//
// It is also the workspace's "Get started" page: how to reach its tools from
// Claude (`mcp_client_id`, `claude_ready`) as well as from its bots.
//
// With `email` (the gateway passes the signed-in person, never a caller-chosen
// one), `can_use` says whether they may use the workspace — the same rule a
// link code is redeemed under, so consumers count. Someone invited before they
// had an account becomes a member here, as they would at their first sign-in
// to the dashboard: the invitation link must work on its own.

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct LinkInfoQuery {
    pub email: Option<String>,
}

pub async fn link_info_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
    query: web::Query<LinkInfoQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let reference = path.into_inner();
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => {
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Database unavailable"}))
        }
    };

    let tenant = match client
        .query_opt(
            "SELECT id, name, mcp_client_id,
                    -- What the gateway checks before Claude may connect: a
                    -- sign-in method, and something to list once connected.
                    (NULLIF(trim(google_client_id), '') IS NOT NULL OR allow_api0_signin)
                    AND (EXISTS (SELECT 1 FROM mcp_tools m WHERE m.tenant_id = t.id AND m.is_active)
                         OR EXISTS (SELECT 1 FROM api_groups g JOIN endpoints e ON e.group_id = g.id
                                     WHERE g.tenant_id = t.id))
               FROM tenants t WHERE mcp_client_id = $1 OR id = $1 LIMIT 1",
            &[&reference],
        )
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => {
            return HttpResponse::NotFound()
                .json(serde_json::json!({"success": false, "error": "No such workspace"}))
        }
        Err(e) => {
            app_log!(error, error = %e, "link-info lookup failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Lookup failed"}));
        }
    };
    let tenant_id: String = tenant.get(0);
    let name: String = tenant.get(1);
    let client_id: Option<String> = tenant.get::<_, Option<String>>(2).filter(|c| !c.trim().is_empty());
    let claude_ready = client_id.is_some() && tenant.get::<_, bool>(3);

    let telegram_bot: Option<String> = client
        .query_opt(
            "SELECT display_ref FROM messaging_channels WHERE tenant_id = $1 AND channel = 'telegram'",
            &[&tenant_id],
        )
        .await
        .ok()
        .flatten()
        .map(|r| r.get::<_, String>(0))
        .filter(|b| !b.is_empty());
    let whatsapp = client
        .query_opt("SELECT 1 FROM whatsapp_channels WHERE tenant_id = $1", &[&tenant_id])
        .await
        .ok()
        .flatten()
        .is_some();

    let can_use = match query.email.as_deref().map(str::trim).filter(|e| !e.is_empty()) {
        Some(email) => Some(can_use(&store, &client, email, &tenant_id).await),
        None => None,
    };

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tenant_id": tenant_id,
        "name": name,
        "link_ref": client_id.clone().unwrap_or_else(|| tenant_id.clone()),
        "mcp_client_id": client_id,
        "claude_ready": claude_ready,
        "can_use": can_use,
        "telegram_bot": telegram_bot,
        "whatsapp": whatsapp,
    }))
}

/// Whether `email` may use the workspace — any role, consumers included.
///
/// A pending invitation is accepted first. Only someone invited here is set up:
/// a stranger who signs in on the page gets no account out of it.
async fn can_use(
    store: &EndpointStore,
    client: &deadpool_postgres::Object,
    email: &str,
    tenant_id: &str,
) -> bool {
    let invited = client
        .query_opt(
            "SELECT 1 FROM tenant_invites WHERE tenant_id = $1 AND email = LOWER($2)",
            &[&tenant_id, &email],
        )
        .await
        .ok()
        .flatten()
        .is_some();
    if invited {
        if let Err(e) = crate::endpoint_store::tenant_management::get_default_tenant(store, email).await {
            app_log!(warn, error = %e, tenant_id = %tenant_id, "Could not accept an invitation from the link page");
        }
    }
    crate::endpoint_store::user_credentials::require_tenant_access(store, email, tenant_id)
        .await
        .is_ok()
}
