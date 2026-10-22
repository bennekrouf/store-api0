use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::middleware::service_key::{require_scope, Scope};
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use std::sync::Arc;

/// GET /api/user/credits/{tenant_id or email}.
///
/// Needs the internal secret (gateway) or a service key with `credits.read`.
/// Asked by email, it also reports the credits left on the email's plan, and
/// `available`: what POST /api/user/credits/spend can charge (payment::spend).
/// `balance` stays the account's own, for callers that show its transactions.
pub async fn get_credit_balance_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    tenant_id: web::Path<String>,
) -> impl Responder {
    if let Err(deny) = require_scope(&req, Scope::CreditsRead) {
        return deny;
    }
    let mut tenant_id = tenant_id.into_inner();
    let email = tenant_id.contains('@').then(|| tenant_id.clone());
    app_log!(info, tenant_id = %tenant_id, "Received HTTP get credit balance request");

    // If tenant_id looks like an email, resolve it to the actual tenant ID
    if tenant_id.contains('@') {
        use crate::endpoint_store::tenant_management;
        // find_ rather than get_: reading a balance must never bring an account
        // into being. This path is called by partner backends polling balances.
        match tenant_management::find_default_tenant(&store, &tenant_id).await {
            Ok(Some(t)) => {
                app_log!(info, email = %tenant_id, resolved_tenant_id = %t.id, "Resolved email to tenant ID");
                tenant_id = t.id;
            }
            // No account yet is a real answer — zero — not a reason to create one.
            Ok(None) => {
                app_log!(info, email = %tenant_id, "No tenant for this email; reporting a zero balance");
                // Same shape as the found case — a caller must not have to
                // branch on which of two response bodies it got.
                let plan = plan_credits(&store, &tenant_id).await;
                return HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "balance": 0,
                    "plan_credits_left": plan,
                    "available": plan,
                    "message": "Credit balance retrieved successfully",
                }));
            }
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
            let plan = match &email {
                Some(e) => plan_credits(&store, e).await,
                None => 0,
            };
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "balance": balance,
                "plan_credits_left": plan,
                "available": balance.max(0) + plan,
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

/// The email's plan credits left; 0 when they cannot be read, so a balance
/// read never fails over the plan.
async fn plan_credits(store: &EndpointStore, email: &str) -> i64 {
    crate::payment::spend::plan_credits_left_for(store, email).await.unwrap_or_else(|e| {
        app_log!(warn, error = %e, "Could not read plan credits for a balance read");
        0
    })
}
