use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for getting credit balance
pub async fn get_credit_balance_handler(
    store: web::Data<Arc<EndpointStore>>,
    tenant_id: web::Path<String>,
) -> impl Responder {
    let tenant_id = tenant_id.into_inner();
    app_log!(info, tenant_id = %tenant_id, "Received HTTP get credit balance request");

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
