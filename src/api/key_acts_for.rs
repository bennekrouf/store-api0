// src/api/key_acts_for.rs
//
// GET /api/internal/key/acts-for?tenant_id=…&email=…  → { success, allowed }
//
// Asked by the gateway when a request carries X-User-Email naming someone
// other than the key's owner: may a key of this tenant act for that person?
// The header decides whose plan pays and who the tenant's backend sees, so an
// unchecked one let any key holder spend anyone's credits and use their stored
// credentials. The rule is api_key_management::tenant_may_act_for.
//
// Auth: X-Internal-Secret; the gateway is the only caller.

use crate::app_log;
use crate::endpoint_store::api_key_management::tenant_may_act_for;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct ActsForQuery {
    pub tenant_id: String,
    pub email: String,
}

pub async fn acts_for_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<ActsForQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    if query.tenant_id.trim().is_empty() || !query.email.contains('@') {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Give a tenant_id and an email address"
        }));
    }
    match tenant_may_act_for(&store, &query.tenant_id, &query.email).await {
        Ok(allowed) => {
            if !allowed {
                app_log!(warn, tenant_id = %query.tenant_id, "A key named someone it may not act for");
            }
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "allowed": allowed }))
        }
        Err(e) => {
            app_log!(error, error = %e, tenant_id = %query.tenant_id, "Could not check who a key may act for");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": "Could not check who this key may act for"
            }))
        }
    }
}
