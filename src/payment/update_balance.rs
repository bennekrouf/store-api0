use crate::app_log;
use crate::email::{send_async, EmailKind};
use crate::endpoint_store::{EndpointStore, UpdateCreditRequest};
use crate::middleware::service_key::{require_scope, Scope};
use crate::payment::tenant_billing::BillingMode;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use std::sync::Arc;

const LOW_CREDITS_THRESHOLD: i64 = 50;
/// POST /api/user/credits — add or spend credits.
///
/// Until now this took no credential at all: anything that could reach the
/// store could top up any account. It needs the internal secret (gateway) or a
/// service key with `credits.write`.
pub async fn update_credit_balance_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<UpdateCreditRequest>,
) -> impl Responder {
    let caller = match require_scope(&req, Scope::CreditsWrite) {
        Ok(c) => c,
        Err(deny) => return deny,
    };
    let email = request.email.to_lowercase();

    // A tenant id in the email field would create a tenant named after a UUID —
    // which is exactly how one of those appeared. Refuse rather than invent.
    if !email.contains('@') {
        app_log!(warn, value = %email, "Rejected a credit update whose 'email' is not an address");
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "'email' must be an email address. Pass 'tenant_id' to target a tenant directly."
        }));
    }
    let amount = request.amount;

    // A key bound to a workspace adds where that workspace bills. Per person,
    // there is no balance to add to: a refund goes back to the plan it came
    // from, and anything else is refused. Per workspace, its balance it is.
    let bound = match crate::payment::tenant_billing::caller_mode(&store, &caller).await {
        Ok(b) => b,
        Err(e) => {
            app_log!(error, error = %e, "Could not read the caller's billing mode");
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": "Billing mode lookup failed",
            }));
        }
    };
    if let Some((_, BillingMode::User)) = &bound {
        return per_person_credit(&store, &email, amount, &request.action_type, request.description.as_deref(), caller.name()).await;
    }

    // Resolve tenant_id: the bound workspace, else an explicit one, else the email's default.
    let tenant_id = if let Some((bound_tenant, _)) = bound {
        bound_tenant
    } else if let Some(tid) = &request.tenant_id {
        tid.clone()
    } else {
        use crate::endpoint_store::tenant_management;
        match tenant_management::get_default_tenant(&store, &email).await {
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

    match store.update_credit_balance(&tenant_id, &email, amount, &request.action_type, request.description.as_deref()).await {
        Ok(new_balance) => {
            app_log!(info, email = %email, amount = amount, new_balance = new_balance, caller = %caller.name(), "Successfully updated credit balance");

            // Low credits warning: only when a deduction crosses the threshold.
            if amount < 0 && new_balance < LOW_CREDITS_THRESHOLD && (new_balance - amount) >= LOW_CREDITS_THRESHOLD {
                send_async(store.as_ref().clone(), email.clone(), EmailKind::LowCredits { balance: new_balance });
            }

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

/// What a refund ends with, for the action types partners use ("cv_generation_refund").
const REFUND_SUFFIX: &str = "_refund";

/// POST /api/user/credits from a key bound to a workspace that bills each
/// person's plan. Only a refund has somewhere to go: back to the plan.
async fn per_person_credit(
    store: &EndpointStore,
    email: &str,
    amount: i64,
    action_type: &str,
    description: Option<&str>,
    caller: &str,
) -> HttpResponse {
    if amount <= 0 || !action_type.ends_with(REFUND_SUFFIX) {
        app_log!(warn, email = %email, amount = amount, action = %action_type, caller = %caller,
            "Refused a balance change: the workspace bills each person's plan");
        return HttpResponse::Conflict().json(serde_json::json!({
            "success": false,
            "message": "This workspace bills each person's plan, so credits cannot be added to or taken from a balance",
        }));
    }
    match crate::payment::spend::refund_to_plan(store, email, amount).await {
        Ok(refunded) => {
            match &refunded {
                // In the plan's usage history beside the charge, as credits back.
                Some(subscription_id) => {
                    let logged = crate::payment::spend::SpendRequest {
                        email: email.to_string(),
                        amount,
                        action_type: action_type.to_string(),
                        description: description.map(str::to_string),
                        input_tokens: None,
                        output_tokens: None,
                        model: None,
                    };
                    crate::payment::spend::log_plan_use(store, subscription_id, email, caller, &logged, -amount).await;
                }
                None => app_log!(warn, email = %email, amount = amount, "A refund found no live plan that had spent credits; nothing given back"),
            }
            let left = crate::payment::spend::plan_credits_left_for(store, email).await.unwrap_or(0);
            app_log!(info, email = %email, amount = amount, caller = %caller, "Refunded credits to a plan");
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "mode": "user",
                "message": format!("Refunded {} credits to the plan", amount),
                "balance": 0,
                "plan_credits_left": left,
                "available": left,
            }))
        }
        Err(e) => {
            app_log!(error, error = %e, email = %email, "Failed to refund credits to a plan");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": "Failed to refund credits",
            }))
        }
    }
}
