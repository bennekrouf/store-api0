use crate::app_log;
use crate::endpoint_store::{EndpointStore, UpdateCreditRequest};
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;
pub async fn update_credit_balance_handler(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<UpdateCreditRequest>,
) -> impl Responder {
    let email = &request.email;
    let amount = request.amount;

    app_log!(info,
        email = %email,
        amount = amount,
        "Received HTTP update credit balance request"
    );

    match store.update_credit_balance(email, amount).await {
        Ok(new_balance) => {
            app_log!(info,
                email = %email,
                amount = amount,
                new_balance = new_balance,
                "Successfully updated credit balance"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": format!("Credit balance updated by {}", amount),
                "balance": new_balance,
            }))
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                email = %email,
                amount = amount,
                "Failed to update credit balance"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to update credit balance: {}", e),
            }))
        }
    }
}
