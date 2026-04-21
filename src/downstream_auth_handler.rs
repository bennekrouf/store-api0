// src/downstream_auth_handler.rs
// HTTP handlers for GET/PUT /api/user/downstream-auth
// and GET /api/tenant/downstream-auth/{tenant_id} (internal, used by gateway)

use crate::app_log;
use crate::endpoint_store::downstream_auth_management::{
    get_downstream_auth, save_downstream_auth, SaveDownstreamAuthRequest,
};
use crate::endpoint_store::tenant_management::get_default_tenant;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct EmailQuery {
    pub email: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveAuthBody {
    pub email: String,
    pub auth_mode: String,
    pub service_account_json: Option<String>,
    pub target_audience: Option<String>,
    pub bearer_token: Option<String>,
    pub custom_headers: Option<serde_json::Value>,
}

pub async fn get_downstream_auth_handler(
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    let tenant = match get_default_tenant(&store, &query.email).await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, error = %e, "get_downstream_auth: tenant lookup failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Tenant not found"}));
        }
    };

    // Also surface the tenant's OAuth client IDs so the dashboard can display them.
    let (mcp_client_id, google_client_id): (Option<String>, Option<String>) =
        match store.get_conn().await {
            Ok(client) => client
                .query_opt(
                    "SELECT mcp_client_id, google_client_id FROM tenants WHERE id = $1",
                    &[&tenant.id],
                )
                .await
                .ok()
                .flatten()
                .map(|row| (row.get(0), row.get(1)))
                .unwrap_or((None, None)),
            Err(_) => (None, None),
        };

    match get_downstream_auth(&store, &tenant.id).await {
        Ok(Some(auth)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "auth": auth,
            "mcp_client_id": mcp_client_id,
            "google_client_id": google_client_id
        })),
        Ok(None) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "auth": {
                "tenant_id": tenant.id,
                "auth_mode": "none",
                "service_account_json": null,
                "target_audience": null,
                "bearer_token": null,
                "custom_headers": null,
                "updated_at": null
            },
            "mcp_client_id": mcp_client_id,
            "google_client_id": google_client_id
        })),
        Err(e) => {
            app_log!(error, error = %e, "get_downstream_auth: DB error");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}))
        }
    }
}

pub async fn save_downstream_auth_handler(
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SaveAuthBody>,
) -> impl Responder {
    let tenant = match get_default_tenant(&store, &body.email).await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, error = %e, "save_downstream_auth: tenant lookup failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Tenant not found"}));
        }
    };

    let req = SaveDownstreamAuthRequest {
        auth_mode:            body.auth_mode.clone(),
        service_account_json: body.service_account_json.clone(),
        target_audience:      body.target_audience.clone(),
        bearer_token:         body.bearer_token.clone(),
        custom_headers:       body.custom_headers.clone(),
    };

    match save_downstream_auth(&store, &tenant.id, &req).await {
        Ok(auth) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "auth": auth
        })),
        Err(e) => {
            app_log!(error, error = %e, "save_downstream_auth: DB error");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "DB error"}))
        }
    }
}

/// Internal handler — called by the gateway with a direct tenant_id
/// (avoids going through the email→tenant lookup).
pub async fn get_downstream_auth_by_id_handler(
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    let tenant_id = path.into_inner();
    match get_downstream_auth(&store, &tenant_id).await {
        Ok(Some(auth)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "auth": auth
        })),
        Ok(None) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "auth": { "auth_mode": "none" }
        })),
        Err(e) => {
            app_log!(error, error = %e, "get_downstream_auth_by_id: DB error");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false}))
        }
    }
}
