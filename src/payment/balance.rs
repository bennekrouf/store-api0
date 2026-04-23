use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for getting credit balance
pub async fn get_credit_balance_handler(
    store: web::Data<Arc<EndpointStore>>,
    tenant_id: web::Path<String>,
) -> impl Responder {
    let mut tenant_id = tenant_id.into_inner();
    app_log!(info, tenant_id = %tenant_id, "Received HTTP get credit balance request");

    // If tenant_id looks like an email, resolve it to the actual tenant ID
    if tenant_id.contains('@') {
        use crate::endpoint_store::tenant_management;
        match tenant_management::get_default_tenant(&store, &tenant_id).await {
            Ok(t) => {
                app_log!(info, email = %tenant_id, resolved_tenant_id = %t.id, "Resolved email to tenant ID");
                tenant_id = t.id;
            },
            Err(e) => {
                app_log!(error, email = %tenant_id, error = %e, "Failed to resolve tenant for balance lookup");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": "Account resolution failed"
                }));
            }
        }
    }

    match store.get_credit_balance(&tenant_id).await {
        Ok(balance) => {
            app_log!(info,
                tenant_id = %tenant_id,
                balance = balance,
                "Successfully retrieved credit balance"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "balance": balance,
                "message": "Credit balance retrieved successfully",
            }))
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                tenant_id = %tenant_id,
                "Failed to retrieve credit balance"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error retrieving credit balance: {}", e),
            }))
        }
    }
}
