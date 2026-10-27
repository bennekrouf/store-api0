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
/// `plan` describes the email's live plan (name, credits, when the month ends),
/// or is null; it never carries the licence, which only its owner may see.
pub async fn get_credit_balance_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    tenant_id: web::Path<String>,
) -> impl Responder {
    let caller = match require_scope(&req, Scope::CreditsRead) {
        Ok(c) => c,
        Err(deny) => return deny,
    };
    let mut tenant_id = tenant_id.into_inner();
    let email = tenant_id.contains('@').then(|| tenant_id.clone());

    // A key bound to a workspace sees only what pays for its users: the
    // workspace's balance, or the person's plan — never the two added up.
    if let Some(email) = &email {
        match crate::payment::tenant_billing::caller_mode(&store, &caller).await {
            Ok(Some((bound, mode))) => return bound_balance(&store, &bound, mode, email).await,
            Ok(None) => {}
            Err(e) => {
                app_log!(error, error = %e, "Could not read the caller's billing mode");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": "Billing mode lookup failed"
                }));
            }
        }
    }
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
                    "plan": plan_summary(&store, &tenant_id).await,
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
            let (plan, summary) = match &email {
                Some(e) => (plan_credits(&store, e).await, plan_summary(&store, e).await),
                None => (0, serde_json::Value::Null),
            };
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "balance": balance,
                "plan_credits_left": plan,
                "available": balance.max(0) + plan,
                "plan": summary,
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

/// The balance read for a key bound to a workspace, in the usual shape plus
/// `mode`: "user" reports the person's plan only, "tenant" the workspace's balance.
async fn bound_balance(
    store: &EndpointStore,
    tenant_id: &str,
    mode: crate::payment::tenant_billing::BillingMode,
    email: &str,
) -> HttpResponse {
    use crate::payment::tenant_billing::BillingMode;
    let (balance, plan_left, plan) = match mode {
        BillingMode::User => (0, plan_credits(store, email).await, plan_summary(store, email).await),
        BillingMode::Tenant => match store.get_credit_balance(tenant_id).await {
            Ok(b) => (b.max(0), 0, serde_json::Value::Null),
            Err(e) => {
                app_log!(error, error = %e, tenant_id = %tenant_id, "Failed to read a workspace balance");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": "Error retrieving credit balance",
                }));
            }
        },
    };
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "mode": mode.as_str(),
        "balance": balance,
        "plan_credits_left": plan_left,
        "available": balance + plan_left,
        "plan": plan,
        "message": "Credit balance retrieved successfully",
    }))
}

/// The email's plan credits left; 0 when they cannot be read, so a balance
/// read never fails over the plan.
async fn plan_credits(store: &EndpointStore, email: &str) -> i64 {
    crate::payment::spend::plan_credits_left_for(store, email).await.unwrap_or_else(|e| {
        app_log!(warn, error = %e, "Could not read plan credits for a balance read");
        0
    })
}

/// The email's live plan, for showing next to the balance; null when it has
/// none or it cannot be read. The licence is left out on purpose: partner
/// backends read this, and the licence is the plan's key to every app.
async fn plan_summary(store: &EndpointStore, email: &str) -> serde_json::Value {
    match crate::payment::subscription::current_subscription(store, email).await {
        Ok(Some(s)) if s.live => serde_json::json!({
            "name": s.plan_name,
            "credits_granted": s.credits_granted,
            "credits_left": s.credits_left,
            "period_end": s.period_end,
            "cancel_at_period_end": s.cancel_at_period_end,
            "is_free": s.is_free,
        }),
        Ok(_) => serde_json::Value::Null,
        Err(e) => {
            app_log!(warn, error = %e, "Could not read the plan for a balance read");
            serde_json::Value::Null
        }
    }
}
