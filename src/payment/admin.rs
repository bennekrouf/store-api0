// src/admin_credit_handler.rs
//
// POST /api/admin/credits
//
// Admin-only endpoint to manually add or remove credits for any api0 user.
//
// Auth:   Valid Firebase JWT whose email is "mohamed.bennekrouf@gmail.com".
// Body:   { "email": "user@example.com", "amount": 100, "description": "optional note" }
//         amount can be negative to deduct credits.
// Return: { success, email, amount, new_balance, description }

use crate::infra::auth::AdminUser;
use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct AdminCreditRequest {
    pub email: String,
    pub amount: i64,
    pub description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminCreditResponse {
    pub success: bool,
    pub email: String,
    pub amount: i64,
    pub new_balance: i64,
    pub description: Option<String>,
}

pub async fn admin_credit_handler(
    _admin: AdminUser, // 401 if not authenticated as admin
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<AdminCreditRequest>,
) -> impl Responder {
    let email = request.email.trim().to_lowercase();

    if email.is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "email is required",
        }));
    }
    if request.amount == 0 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "error": "amount must be non-zero",
        }));
    }

    let action_type = if request.amount > 0 {
        "admin_topup"
    } else {
        "admin_deduct"
    };

    match store
        .update_credit_balance(
            &email,
            request.amount,
            action_type,
            request.description.as_deref(),
        )
        .await
    {
        Ok(new_balance) => {
            app_log!(
                info,
                email = %email,
                amount = request.amount,
                new_balance = new_balance,
                action = %action_type,
                "Admin credit adjustment applied"
            );
            HttpResponse::Ok().json(AdminCreditResponse {
                success: true,
                email,
                amount: request.amount,
                new_balance,
                description: request.description.clone(),
            })
        }
        Err(e) => {
            app_log!(error, email = %email, error = %e, "Admin credit adjustment failed");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "error": format!("Credit update failed: {}", e),
            }))
        }
    }
}
