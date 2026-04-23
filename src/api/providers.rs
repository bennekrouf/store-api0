// src/api/providers.rs
//
// GET /api/providers
//
// Returns the list of api0 tenants that are acting as MCP providers —
// i.e. tenants that have at least one api_group with an external base URL.
//
// Public endpoint (no auth required): callers need this to pick a provider
// before they have a consumer key.
//
// Response:
//   { "providers": [ { "id", "name", "tool_count" } ] }

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

pub async fn list_providers_handler(
    store: web::Data<Arc<EndpointStore>>,
) -> impl Responder {
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, error = %e, "DB connection failed for list_providers");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": "Database error"
            }));
        }
    };

    // A "provider" is any tenant with at least one api_group that points
    // to an external HTTPS URL.
    let rows = match client
        .query(
            "SELECT t.id, t.name, COUNT(e.id)::BIGINT AS tool_count
             FROM tenants t
             JOIN api_groups g ON g.tenant_id = t.id
             JOIN endpoints  e ON e.group_id  = g.id
             WHERE g.base LIKE 'https://%'
             GROUP BY t.id, t.name
             ORDER BY t.name",
            &[],
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            app_log!(error, error = %e, "list_providers query failed");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": "Query failed"
            }));
        }
    };

    let providers: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "id":         row.get::<_, String>(0),
                "name":       row.get::<_, String>(1),
                "tool_count": row.get::<_, i64>(2),
            })
        })
        .collect();

    app_log!(info, count = providers.len(), "Listed providers");

    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "providers": providers
    }))
}
