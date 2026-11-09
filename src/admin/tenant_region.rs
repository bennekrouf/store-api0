// src/admin/tenant_region.rs
//
// The region a workspace's messaging assistant (WhatsApp, Telegram, Teams)
// answers from: the same three a desktop app's user chooses between
// (llm_regions.rs). The region runs its `chat` task's model, and the workspace
// only ever sees the region. A workspace that has picked none gets no answers
// until it does: nothing chooses where its data goes on its behalf.
//
//   GET /api/user/assistant-region?email=…  → { region, live, regions: [{ id, label, flag, available }], can_edit }
//                                             for the caller's workspace; `live` is
//                                             false when its region is off offer;
//                                             `regions` is all of them, `available`
//                                             false for one not open yet
//   PUT /api/user/assistant-region          { email, region } — one of the regions;
//                                             owners and admins
//
// Stored in tenants.llm_region. A region taken off offer (no provider behind
// it, or its key removed) stops the assistant until it is back, and the choice
// is kept: a workspace that chose where its data goes is never moved elsewhere. Both need X-Internal-Secret; the gateway binds
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

/// What a tenant's choice of region means for its assistant now.
pub(crate) enum AssistantRegion {
    /// None chosen: the assistant does not answer.
    NotChosen,
    Live { region: &'static Region, provider: String, model: String },
    /// Chosen but off offer: nothing may answer in its place.
    Unavailable(&'static Region),
}

pub(crate) async fn resolve(store: &EndpointStore, tenant_id: &str) -> Result<AssistantRegion, String> {
    let Some(region) = tenant_region(store, tenant_id).await? else {
        return Ok(AssistantRegion::NotChosen);
    };
    let (regions, keyed) = (
        llm_regions::read_offered(store).await?,
        assistant_config::providers_with_key(store).await?,
    );
    Ok(llm_regions::live(&regions, &keyed)
        .into_iter()
        .find(|(r, _)| r.id == region.id)
        .map(|(r, b)| {
            let (p, m) = b.for_task(Some(TASK), &keyed);
            AssistantRegion::Live { region: r, provider: p.to_string(), model: m.to_string() }
        })
        .unwrap_or(AssistantRegion::Unavailable(region)))
}

fn failed(what: &str, e: String) -> HttpResponse {
    app_log!(error, error = %e, "Assistant region: {}", what);
    HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": format!("Could not {}", what) }))
}

/// The regions on offer now, in the order the apps list them.
async fn live_regions(store: &EndpointStore) -> Result<Vec<&'static Region>, String> {
    let (regions, keyed) = (
        llm_regions::read_offered(store).await?,
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
        Err(e) => return failed("find the tenant", e.to_string()),
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
        "regions": llm_regions::REGIONS
            .iter()
            .map(|r| serde_json::json!({
                "id": r.id,
                "label": r.label,
                "flag": r.flag,
                "available": offered.iter().any(|o| o.id == r.id),
            }))
            .collect::<Vec<_>>(),
        "can_edit": editable,
    }))
}

#[derive(Deserialize)]
pub struct SetRegionBody {
    pub email: String,
    #[serde(default)]
    pub region: String,
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
        Err(e) => return failed("find the tenant", e.to_string()),
    };
    match can_edit(&store, &tenant.id, &body.email).await {
        Ok(true) => {}
        Ok(false) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "success": false,
                "error": "Only the tenant's owners and admins can change the assistant's region",
            }))
        }
        Err(e) => return failed("read the caller's role", e),
    }
    let wanted = body.region.trim();
    let offered = match live_regions(&store).await {
        Ok(o) => o,
        Err(e) => return failed("read the regions", e),
    };
    let Some(region) = offered.into_iter().find(|r| r.id.eq_ignore_ascii_case(wanted)) else {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": format!("'{}' is not a region on offer", wanted),
        }));
    };
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => return failed("save the region", e.to_string()),
    };
    if let Err(e) = client
        .execute("UPDATE tenants SET llm_region = $1 WHERE id = $2", &[&region.id, &tenant.id])
        .await
    {
        return failed("save the region", e.to_string());
    }
    app_log!(info, tenant_id = %tenant.id, email = %body.email, region = region.id, "Assistant region set");
    HttpResponse::Ok().json(serde_json::json!({ "success": true, "region": region.id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL"]
    async fn a_region_off_offer_is_unavailable_and_no_choice_is_not() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = Arc::new(EndpointStore::new(&url).await.expect("store"));
        let run = uuid::Uuid::new_v4().simple().to_string();
        let tenant = get_default_tenant(&store, &format!("region-{run}@example.com")).await.unwrap();
        let t = tenant.id.as_str();

        assert!(matches!(resolve(&store, t).await.unwrap(), AssistantRegion::NotChosen));

        // No provider has a key on a fresh database, so no region is on offer.
        let c = store.get_admin_conn().await.unwrap();
        c.execute("UPDATE tenants SET llm_region = 'europe' WHERE id = $1", &[&t]).await.unwrap();
        match resolve(&store, t).await.unwrap() {
            AssistantRegion::Unavailable(r) => assert_eq!(r.id, "europe"),
            _ => panic!("a region off offer must not resolve to a provider"),
        }
    }
}
