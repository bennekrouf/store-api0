// src/get_api_usage_logs.rs
use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

/// Handler for getting detailed API usage logs with token information
pub async fn get_api_usage_logs(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>, // (tenant_id, key_id)
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let (mut tenant_id, key_id) = path_params.into_inner();

    // If tenant_id looks like an email, resolve it to the actual tenant ID
    if tenant_id.contains('@') {
        use crate::endpoint_store::tenant_management;
        match tenant_management::get_default_tenant(&store, &tenant_id).await {
            Ok(t) => {
                app_log!(info, email = %tenant_id, resolved_tenant_id = %t.id, "Resolved email to tenant ID for usage logs");
                tenant_id = t.id;
            },
            Err(e) => {
                app_log!(error, email = %tenant_id, error = %e, "Failed to resolve tenant for usage logs lookup");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": "Account resolution failed"
                }));
            }
        }
    }

    // Extract limit from query parameters if provided
    let limit = query.get("limit").and_then(|l| l.parse::<i64>().ok());

    app_log!(info,
        tenant_id = %tenant_id,
        key_id = %key_id,
        limit = limit,
        "Received HTTP get API usage logs request"
    );

    match store.get_api_usage_logs(&key_id, &tenant_id, limit).await {
        Ok(logs) => {
            app_log!(info,
                tenant_id = %tenant_id,
                key_id = %key_id,
                log_count = logs.len(),
                "Successfully retrieved API usage logs with token data"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "logs": logs,
                "count": logs.len(),
            }))
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                tenant_id = %tenant_id,
                key_id = %key_id,
                "Failed to retrieve API usage logs"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to retrieve API usage logs: {}", e),
            }))
        }
    }
}
