// src/mcp/link_info.rs
//
// What a workspace's branded linking page and its bot need to know about it:
//
//   GET /api/internal/link-info/{ref}   (X-Internal-Secret)
//
// `ref` is the workspace's OAuth client id or its tenant id, so every workspace
// has a page even before it chose a client id. `link_ref` is what its page
// address should use: the client id when set (readable), else the tenant id.
// Nothing here is secret — a bot's username is public — but it is internal so
// that only the gateway and the bridge shape what is shown.

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use std::sync::Arc;

pub async fn link_info_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
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
            "SELECT id, name, mcp_client_id FROM tenants WHERE mcp_client_id = $1 OR id = $1 LIMIT 1",
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
    let client_id: Option<String> = tenant.get(2);

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

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tenant_id": tenant_id,
        "name": name,
        "link_ref": client_id.filter(|c| !c.trim().is_empty()).unwrap_or_else(|| tenant_id.clone()),
        "telegram_bot": telegram_bot,
        "whatsapp": whatsapp,
    }))
}
