// src/mcp/oauth_codes.rs
//
// Internal HTTP surface for spending an OAuth authorization code. Requires
// X-Internal-Secret: only the gateway's token endpoint calls it.
//
//   POST /internal/oauth-code/redeem   { "jti": "…" }
//        200 → first redemption, go ahead
//        409 → already redeemed, refuse the exchange

use crate::app_log;
use crate::endpoint_store::oauth_codes::redeem_code;
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::middleware::internal_secret::require_internal_secret;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct RedeemRequest {
    pub jti: String,
}

pub async fn redeem_code_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<RedeemRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }

    match redeem_code(&store, &body.jti).await {
        Ok(true) => HttpResponse::Ok().json(serde_json::json!({"success": true})),
        Ok(false) => {
            app_log!(warn, "An OAuth authorization code was presented a second time");
            HttpResponse::Conflict()
                .json(serde_json::json!({"success": false, "error": "Code already redeemed"}))
        }
        Err(StoreError::InvalidInput(msg)) => {
            HttpResponse::BadRequest().json(serde_json::json!({"success": false, "error": msg}))
        }
        Err(e) => {
            app_log!(error, error = %e, "Could not record an OAuth code redemption");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": e.to_string()}))
        }
    }
}
