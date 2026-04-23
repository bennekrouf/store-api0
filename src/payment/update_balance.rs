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

    // Resolve tenant_id: use explicit one if provided, otherwise fallback to default for email.
    let tenant_id = if let Some(tid) = &request.tenant_id {
        tid.clone()
    } else {
        use crate::endpoint_store::tenant_management;
        match tenant_management::get_default_tenant(&store, email).await {
            Ok(t) => t.id,
            Err(e) => {
                app_log!(error, email = %email, "Failed to resolve default tenant for credit update: {}", e);
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": "Failed to resolve tenant",
                }));
            }
        }
    };

    match store.update_credit_balance(&tenant_id, email, amount, &request.action_type, request.description.as_deref()).await {
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
