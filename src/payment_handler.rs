use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::payment_service::PaymentService;
use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Deserialize)]
pub struct CreateIntentRequest {
    pub email: String,
    pub amount: i64,   // in cents (frontend sends amount * 100)
    pub currency: String,
}

#[derive(Deserialize)]
pub struct ConfirmRequest {
    pub email: String,
    pub payment_intent_id: String,
    pub amount: i64,   // dollar amount (frontend sends the selected $ value)
}

/// POST /api/payments/intent
/// Creates a Stripe PaymentIntent and returns the client_secret to the frontend.
pub async fn create_payment_intent_handler(
    payment_service: web::Data<Arc<PaymentService>>,
    request: web::Json<CreateIntentRequest>,
) -> impl Responder {
    app_log!(info,
        email = %request.email,
        amount = request.amount,
        currency = %request.currency,
        "Creating Stripe payment intent"
    );

    match payment_service
        .create_payment_intent(request.amount, &request.currency, &request.email)
        .await
    {
        Ok(intent) => {
            let client_secret = intent.client_secret.clone().unwrap_or_default();
            let intent_id = intent.id.to_string();
            app_log!(info, intent_id = %intent_id, "Payment intent created");
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "client_secret": client_secret,
                "payment_intent_id": intent_id,
            }))
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to create payment intent");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to create payment intent: {}", e),
            }))
        }
    }
}

/// POST /api/payments/confirm
/// Verifies the PaymentIntent succeeded with Stripe, then credits the user's account.
pub async fn confirm_payment_handler(
    store: web::Data<Arc<EndpointStore>>,
    payment_service: web::Data<Arc<PaymentService>>,
    request: web::Json<ConfirmRequest>,
) -> impl Responder {
    let email = &request.email;
    let payment_intent_id = &request.payment_intent_id;
    let amount = request.amount; // dollar amount, e.g. 10 for a $10 payment

    app_log!(info,
        email = %email,
        payment_intent_id = %payment_intent_id,
        amount = amount,
        "Confirming payment"
    );

    // 1. Verify Stripe intent status
    match payment_service.confirm_payment(payment_intent_id).await {
        Ok(intent) => {
            use stripe::PaymentIntentStatus;
            if intent.status != PaymentIntentStatus::Succeeded {
                app_log!(warn,
                    payment_intent_id = %payment_intent_id,
                    status = ?intent.status,
                    "Payment intent not succeeded"
                );
                return HttpResponse::BadRequest().json(serde_json::json!({
                    "success": false,
                    "message": format!("Payment not completed. Status: {:?}", intent.status),
                }));
            }

            // 2. Credit the user account
            let description = format!("Stripe payment – ${}", amount);
            match store
                .update_credit_balance(email, amount, "stripe_topup", Some(&description))
                .await
            {
                Ok(new_balance) => {
                    app_log!(info,
                        email = %email,
                        amount = amount,
                        new_balance = new_balance,
                        "Credits added after successful payment"
                    );
                    HttpResponse::Ok().json(serde_json::json!({
                        "success": true,
                        "message": format!("Payment confirmed. {} credits added.", amount),
                        "new_balance": new_balance,
                    }))
                }
                Err(e) => {
                    app_log!(error,
                        error = %e,
                        email = %email,
                        "Payment confirmed but credit update failed"
                    );
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "success": false,
                        "message": format!("Payment confirmed but credits could not be added: {}", e),
                    }))
                }
            }
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to verify payment intent with Stripe");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to verify payment: {}", e),
            }))
        }
    }
}

/// GET /api/payments/history/{email}
/// Returns the user's Stripe top-up history from credit_transactions.
pub async fn get_payment_history_handler(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    app_log!(info, email = %email, "Fetching payment history");

    match store.get_payment_history(&email).await {
        Ok(payments) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "payments": payments,
        })),
        Err(e) => {
            app_log!(error, error = %e, email = %email, "Failed to fetch payment history");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "payments": serde_json::json!([]),
                "message": format!("Failed to fetch payment history: {}", e),
            }))
        }
    }
}
