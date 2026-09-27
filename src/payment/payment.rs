use crate::app_log;
use crate::email::{send_async, EmailKind};
use crate::endpoint_store::EndpointStore;
use crate::payment::service::PaymentService;
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::topup::{self, TopUp, TopUpError, MAX_AMOUNT, MIN_AMOUNT};
use actix_web::{web, HttpRequest, HttpResponse, Responder};
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
    /// Still sent by the dashboard, and ignored: the credits come from the
    /// amount Stripe charged. See payment::topup.
    #[allow(dead_code)]
    pub amount: Option<i64>,
}

/// POST /api/payments/intent
/// Creates a Stripe PaymentIntent and returns the client_secret to the frontend.
pub async fn create_payment_intent_handler(
    req: HttpRequest,
    payment_service: web::Data<Arc<PaymentService>>,
    request: web::Json<CreateIntentRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    if !(MIN_AMOUNT..=MAX_AMOUNT).contains(&request.amount) {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": format!(
                "Amount must be between {} and {}",
                MIN_AMOUNT / 100,
                MAX_AMOUNT / 100
            ),
        }));
    }
    let email = request.email.to_lowercase();
    app_log!(info,
        email = %email,
        amount = request.amount,
        currency = %request.currency,
        "Creating Stripe payment intent"
    );

    let email = request.email.to_lowercase();
    match payment_service
        .create_payment_intent(request.amount, &request.currency, &email)
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
/// Called by the browser once Stripe.js reports the payment succeeded. Credits
/// the account the intent was created for, from the amount Stripe charged, once
/// per intent: the payment_intent.succeeded webhook may already have done it.
pub async fn confirm_payment_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    payment_service: web::Data<Arc<PaymentService>>,
    request: web::Json<ConfirmRequest>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let email = request.email.to_lowercase();
    let payment_intent_id = &request.payment_intent_id;

    app_log!(info,
        email = %email,
        payment_intent_id = %payment_intent_id,
        "Confirming payment"
    );

    let intent = match payment_service.confirm_payment(payment_intent_id).await {
        Ok(intent) => intent,
        Err(e) => {
            app_log!(error, error = %e, "Failed to verify payment intent with Stripe");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to verify payment: {}", e),
            }));
        }
    };

    match topup::credit_intent(&store, &intent, Some(&email)).await {
        Ok((email, TopUp::Credited { credits, new_balance })) => {
            send_async(store.as_ref().clone(), email, EmailKind::PaymentReceipt {
                amount_dollars: intent.amount_received as f64 / 100.0,
                credits_added: credits,
                new_balance,
            });
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": format!("Payment confirmed. {} credits added.", credits),
                "new_balance": new_balance,
            }))
        }
        Ok((_, TopUp::AlreadyCredited { balance })) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "message": "Payment confirmed. Credits were already added.",
            "new_balance": balance,
        })),
        Err(e @ TopUpError::Store(_)) => {
            app_log!(error, error = %e, email = %email, payment_intent_id = %payment_intent_id,
                "Payment confirmed but credit update failed");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": e.to_string(),
            }))
        }
        Err(e) => {
            app_log!(warn, error = %e, email = %email, payment_intent_id = %payment_intent_id,
                "Payment not credited");
            HttpResponse::BadRequest().json(serde_json::json!({
                "success": false,
                "message": e.to_string(),
            }))
        }
    }
}

/// GET /api/payments/history/{email}
/// Returns the user's Stripe top-up history from credit_transactions.
pub async fn get_payment_history_handler(
    store: web::Data<Arc<EndpointStore>>,
    tenant_id_or_email: web::Path<String>,
) -> impl Responder {
    let mut tenant_id = tenant_id_or_email.into_inner();
    app_log!(info, input = %tenant_id, "Fetching payment history");

    // If it looks like an email, resolve it
    if tenant_id.contains('@') {
        match crate::endpoint_store::tenant_management::get_default_tenant(&store, &tenant_id).await {
            Ok(t) => {
                app_log!(info, email = %tenant_id, resolved_tenant_id = %t.id, "Resolved email to tenant ID");
                tenant_id = t.id;
            },
            Err(e) => {
                app_log!(error, error = %e, email = %tenant_id, "Payment history: tenant lookup failed");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": "Account resolution failed"
                }));
            }
        }
    }

    match store.get_payment_history(&tenant_id).await {
        Ok(payments) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "payments": payments,
        })),
        Err(e) => {
            app_log!(error, error = %e, tenant_id = %tenant_id, "Failed to fetch payment history");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "payments": serde_json::json!([]),
                "message": format!("Failed to fetch payment history: {}", e),
            }))
        }
    }
}
