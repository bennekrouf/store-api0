use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for getting credit balance
pub async fn get_credit_balance_handler(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    app_log!(info, email = %email, "Received HTTP get credit balance request");

    match store.get_credit_balance(&email).await {
        Ok(balance) => {
            app_log!(info,
                email = %email,
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
                email = %email,
                "Failed to retrieve credit balance"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error retrieving credit balance: {}", e),
            }))
        }
    }
}
