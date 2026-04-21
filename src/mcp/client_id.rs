// src/mcp_client_id_handler.rs
// GET  /api/tenant/by-client-id/{client_id}  — used by gateway to resolve provider
// PUT  /api/user/mcp-client-id               — used by dashboard to set client_id + Google OAuth config

use crate::app_log;
use crate::endpoint_store::tenant_management::{get_tenant_by_mcp_client_id, set_mcp_client_id};
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

/// Gateway calls GET /api/tenant/by-client-id/{client_id}
/// Returns { tenant_id, name, google_client_id? } or 404.
pub async fn get_by_client_id_handler(
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    let client_id = path.into_inner();
    match get_tenant_by_mcp_client_id(&store, &client_id).await {
        Ok(Some((tenant, google_client_id))) => {
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "tenant_id": tenant.id,
                "name": tenant.name,
                "google_client_id": google_client_id,
            }))
        }
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "success": false,
            "error": "No tenant registered with that client_id"
        })),
        Err(e) => {
            app_log!(error, error = %e, client_id = %client_id, "get_by_client_id failed");
            HttpResponse::InternalServerError().json(serde_json::json!({ "success": false }))
        }
    }
}

#[derive(Deserialize)]
pub struct SetClientIdBody {
    pub email: String,
    pub mcp_client_id: Option<String>,
    /// Google OAuth 2.0 Web Client ID — used by the api0 authorize page to sign in
    /// end-users via Google Identity Services (not Firebase-specific).
    pub google_client_id: Option<String>,
}

/// Dashboard calls PUT /api/user/mcp-client-id
pub async fn set_client_id_handler(
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SetClientIdBody>,
) -> impl Responder {
    let id_ref = body.mcp_client_id.as_deref();
    match set_mcp_client_id(
        &store,
        &body.email,
        id_ref,
        body.google_client_id.as_deref(),
    )
    .await
    {
        Ok(()) => {
            app_log!(
                info,
                email = %body.email,
                mcp_client_id = ?body.mcp_client_id,
                google_client_id = ?body.google_client_id,
                "mcp_client_id + Google client_id updated"
            );
            HttpResponse::Ok().json(serde_json::json!({ "success": true }))
        }
        Err(e) => {
            app_log!(error, error = %e, "set_mcp_client_id failed");
            HttpResponse::InternalServerError().json(serde_json::json!({ "success": false }))
        }
    }
}
