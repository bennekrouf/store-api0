// src/admin/tenant_region.rs
//
// The region a workspace's messaging assistant (WhatsApp, Telegram, Teams)
// answers from: the same three a desktop app's user chooses between
// (llm_regions.rs). The region runs its `chat` task's model, and the workspace
// only ever sees the region. A workspace that picks none stays on the
// platform's assistant provider (assistant_config.rs), as before.
//
//   GET /api/user/assistant-region?email=…  → { region, live, regions: [{ id, label, flag }], can_edit }
//                                             for the caller's workspace; `live` is
//                                             false when its region is off offer
//   PUT /api/user/assistant-region          { email, region } — "" or null for the
//                                             platform's; owners and admins
//
// Stored in tenants.llm_region. A region taken off offer (no provider behind
// it, or its key removed) answers with the platform's provider until it is
// back; the choice is kept. Both need X-Internal-Secret; the gateway binds
// `email` to the signed-in caller.

use crate::admin::assistant_config;
use crate::admin::llm_regions::{self, Region};
use crate::app_log;
use crate::endpoint_store::tenant_management::get_default_tenant;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::tenant_billing::can_edit;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

/// The task a messaging assistant's calls do (llm_models::TASKS).
const TASK: &str = "chat";

/// The region a tenant chose, if any.
pub(crate) async fn tenant_region(store: &EndpointStore, tenant_id: &str) -> Result<Option<&'static Region>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let row = client
        .query_opt("SELECT llm_region FROM tenants WHERE id = $1", &[&tenant_id])
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.and_then(|r| r.get::<_, Option<String>>(0)).and_then(|id| llm_regions::region(&id)))
}

/// The region, provider and model a tenant's assistant runs on, when it chose
/// a region that is on offer now.
pub(crate) async fn resolve(store: &EndpointStore, tenant_id: &str) -> Result<Option<(&'static Region, String, String)>, String> {
    let Some(region) = tenant_region(store, tenant_id).await? else {
        return Ok(None);
    };
    let (regions, keyed) = (
        llm_regions::read_regions(store).await?,
        assistant_config::providers_with_key(store).await?,
    );
    Ok(llm_regions::live(&regions, &keyed)
        .into_iter()
        .find(|(r, _)| r.id == region.id)
        .map(|(r, b)| {
            let (p, m) = b.for_task(Some(TASK), &keyed);
            (r, p.to_string(), m.to_string())
        }))
}

fn failed(what: &str, e: String) -> HttpResponse {
    app_log!(error, error = %e, "Assistant region: {}", what);
    HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": format!("Could not {}", what) }))
}

/// The regions on offer now, in the order the apps list them.
async fn live_regions(store: &EndpointStore) -> Result<Vec<&'static Region>, String> {
    let (regions, keyed) = (
        llm_regions::read_regions(store).await?,
        assistant_config::providers_with_key(store).await?,
    );
    Ok(llm_regions::live(&regions, &keyed).into_iter().map(|(r, _)| r).collect())
}

#[derive(Deserialize)]
pub struct EmailQuery {
    pub email: String,
}

/// GET /api/user/assistant-region?email=…
pub async fn get_my_region_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant = match get_default_tenant(&store, &query.email).await {
        Ok(t) => t,
        Err(e) => return failed("find the workspace", e.to_string()),
    };
    let (chosen, offered, editable) = match (
        tenant_region(&store, &tenant.id).await,
        live_regions(&store).await,
        can_edit(&store, &tenant.id, &query.email).await,
    ) {
        (Ok(c), Ok(o), Ok(e)) => (c, o, e),
        (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => return failed("read the region", e),
    };
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tenant_id": tenant.id,
        "region": chosen.map(|r| r.id),
        "live": chosen.is_none_or(|c| offered.iter().any(|r| r.id == c.id)),
        "regions": offered
            .iter()
            .map(|r| serde_json::json!({ "id": r.id, "label": r.label, "flag": r.flag }))
            .collect::<Vec<_>>(),
        "can_edit": editable,
    }))
}

#[derive(Deserialize)]
pub struct SetRegionBody {
    pub email: String,
    #[serde(default)]
    pub region: Option<String>,
}

/// PUT /api/user/assistant-region
pub async fn set_my_region_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SetRegionBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant = match get_default_tenant(&store, &body.email).await {
        Ok(t) => t,
        Err(e) => return failed("find the workspace", e.to_string()),
    };
    match can_edit(&store, &tenant.id, &body.email).await {
        Ok(true) => {}
        Ok(false) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "success": false,
                "error": "Only the workspace's owners and admins can change the assistant's region",
            }))
        }
        Err(e) => return failed("read the caller's role", e),
    }
    let wanted = body.region.as_deref().map(str::trim).filter(|r| !r.is_empty());
    let region = match wanted {
        None => None,
        Some(id) => {
            let offered = match live_regions(&store).await {
                Ok(o) => o,
                Err(e) => return failed("read the regions", e),
            };
            match offered.into_iter().find(|r| r.id.eq_ignore_ascii_case(id)) {
                Some(r) => Some(r),
                None => {
                    return HttpResponse::BadRequest().json(serde_json::json!({
                        "success": false,
                        "error": format!("'{}' is not a region on offer", id),
                    }))
                }
            }
        }
    };
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => return failed("save the region", e.to_string()),
    };
    if let Err(e) = client
        .execute("UPDATE tenants SET llm_region = $1 WHERE id = $2", &[&region.map(|r| r.id), &tenant.id])
        .await
    {
        return failed("save the region", e.to_string());
    }
    app_log!(info, tenant_id = %tenant.id, email = %body.email, region = ?region.map(|r| r.id), "Assistant region set");
    HttpResponse::Ok().json(serde_json::json!({ "success": true, "region": region.map(|r| r.id) }))
}
