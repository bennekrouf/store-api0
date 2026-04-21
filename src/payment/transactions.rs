use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

pub async fn get_credit_transactions_handler(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    app_log!(info, email = %email, "Received HTTP get credit transactions request");

    match store.get_credit_transactions(&email, 50).await {
        Ok(transactions) => {
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "transactions": transactions,
            }))
        }
        Err(e) => {
            app_log!(error, error = %e, email = %email, "Failed to retrieve credit transactions");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error retrieving credit transactions: {}", e),
            }))
        }
    }
}
