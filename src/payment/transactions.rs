use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

pub async fn get_credit_transactions_handler(
    store: web::Data<Arc<EndpointStore>>,
    tenant_id: web::Path<String>,
) -> impl Responder {
    let tenant_id = tenant_id.into_inner();
    app_log!(info, tenant_id = %tenant_id, "Received HTTP get credit transactions request");

    match store.get_credit_transactions(&tenant_id, 50).await {
        Ok(transactions) => {
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "transactions": transactions,
            }))
        }
        Err(e) => {
            app_log!(error, error = %e, tenant_id = %tenant_id, "Failed to retrieve credit transactions");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error retrieving credit transactions: {}", e),
            }))
        }
    }
}
