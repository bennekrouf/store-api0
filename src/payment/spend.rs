// src/payment/spend.rs
//
// Credits spent by a partner backend for one of its users, by email — CVenom
// charges a fixed price per CV, cover letter or translation. Someone with a
// plan (payment::subscription) spends its monthly credits first: they lapse at
// the end of the month, the account balance does not. The account balance
// (tenants.credit_balance, filled by top-ups) covers the rest.
//
//   POST /api/user/credits/spend   { email, amount, action_type, description?,
//                                    input_tokens?, output_tokens?, model? }
//        → 200 { from_plan, from_balance, plan_credits_left, balance, available }
//        → 402 { available } when the plan and the balance together fall short;
//          nothing is taken then
//
// Needs a service key with `credits.write` (or the internal secret). The
// balance read, GET /api/user/credits/{email}, reports the plan's credits left
// too (plan_credits_left_for).
//
// The tokens and model are what the partner's own AI call reported; api0 does
// not see that call. They go on the plan's usage row, the action then moving
// to `region`, which is what the subscriber's history shows in place of the
// model (llm_wallet::recent_usage).
//
// A refund goes back to the account balance, through POST /api/user/credits:
// the caller does not know which side paid, and a refunded credit kept past
// the month is the user's gain, never their loss.

use crate::app_log;
use crate::endpoint_store::{tenant_management, EndpointStore};
use crate::middleware::service_key::{require_scope, Scope};
use actix_web::http::StatusCode;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

/// Statuses that give credits; the same as payment::subscription's.
const LIVE: &str = "s.status IN ('active', 'trialing') AND s.period_end > NOW()";

/// The order a live plan is spent in when an email has more than one: a paid
/// plan before the free allowance, the one ending last first.
const PLAN_ORDER: &str = "s.is_free ASC, s.period_end DESC";

/// Credits left this month on the email's live plans.
pub async fn plan_credits_left_for(store: &EndpointStore, email: &str) -> Result<i64, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!(
        "SELECT COALESCE(SUM(GREATEST(s.credits_granted - s.credits_used, 0)), 0)::BIGINT \
         FROM subscriptions s WHERE s.email = $1 AND {LIVE}"
    );
    let row = client.query_one(sql.as_str(), &[&email.to_lowercase()]).await.map_err(|e| e.to_string())?;
    Ok(row.get(0))
}

/// Takes up to `amount` from the email's live plan, in one statement so two
/// charges at once cannot both spend the same credits. Returns the plan's id,
/// what it gave and what that plan has left; `None` when no live plan has any.
async fn take_from_plan(store: &EndpointStore, email: &str, amount: i64) -> Result<Option<(String, i64, i64)>, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let sql = format!(
        "WITH pick AS ( \
             SELECT s.id, LEAST($2, s.credits_granted - s.credits_used) AS take \
             FROM subscriptions s \
             WHERE s.email = $1 AND {LIVE} AND s.credits_granted > s.credits_used \
             ORDER BY {PLAN_ORDER} LIMIT 1 FOR UPDATE) \
         UPDATE subscriptions s SET credits_used = s.credits_used + pick.take, updated_at = NOW() \
         FROM pick WHERE s.id = pick.id \
         RETURNING s.id, pick.take, s.credits_granted - s.credits_used"
    );
    let row = client.query_opt(sql.as_str(), &[&email, &amount]).await.map_err(|e| e.to_string())?;
    Ok(row.map(|r| (r.get(0), r.get(1), r.get(2))))
}

/// Puts back what take_from_plan gave, when the balance then could not pay the rest.
async fn give_back_to_plan(store: &EndpointStore, subscription_id: &str, amount: i64) -> Result<(), String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    client
        .execute(
            "UPDATE subscriptions SET credits_used = GREATEST(credits_used - $2, 0), updated_at = NOW() WHERE id = $1",
            &[&subscription_id, &amount],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The plan's side of the charge in the plan's usage history, next to the
/// apps' AI calls, so the subscriber sees where their credits went.
async fn log_plan_use(store: &EndpointStore, subscription_id: &str, email: &str, product: &str, body: &SpendRequest, credits: i64) {
    let Ok(client) = store.get_admin_conn().await else { return };
    let id = format!("spend_{}", uuid::Uuid::new_v4().simple());
    let (model, region) = usage_model(&body.action_type, body.model.as_deref());
    let input = body.input_tokens.unwrap_or(0).max(0);
    let output = body.output_tokens.unwrap_or(0).max(0);
    let result = client
        .execute(
            "INSERT INTO llm_usage (id, tenant_id, license_id, email, product, provider, model, region,
                input_tokens, output_tokens, total_tokens, estimated, credits, status_code)
             VALUES ($1, COALESCE((SELECT default_tenant_id FROM user_preferences WHERE email = $3), ''),
                $2, $3, $4, $4, $5, $6, $7, $8, $7 + $8, FALSE, $9, 200)",
            &[&id, &subscription_id, &email, &product, &model, &region, &input, &output, &credits],
        )
        .await;
    if let Err(e) = result {
        app_log!(warn, error = %e, email = %email, "Could not log a plan's credits spent by a partner");
    }
}

/// The usage row's model and region: the model the partner names, with the
/// action as the region; without one, the action as the model.
fn usage_model(action: &str, model: Option<&str>) -> (String, Option<String>) {
    match model.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => (m.to_string(), Some(action.to_string())),
        None => (action.to_string(), None),
    }
}

#[derive(Deserialize)]
pub struct SpendRequest {
    pub email: String,
    /// What the operation costs, in credits; more than 0.
    pub amount: i64,
    /// What the caller logs it as, e.g. "cv_generation".
    pub action_type: String,
    pub description: Option<String>,
    /// The tokens the partner's AI call took, as its provider reported them.
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    /// The model that ran, e.g. "deepseek-chat".
    pub model: Option<String>,
}

fn reply(status: StatusCode, body: serde_json::Value) -> HttpResponse {
    HttpResponse::build(status).json(body)
}

/// POST /api/user/credits/spend
pub async fn spend_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SpendRequest>,
) -> impl Responder {
    let caller = match require_scope(&req, Scope::CreditsWrite) {
        Ok(c) => c,
        Err(deny) => return deny,
    };
    let email = body.email.trim().to_lowercase();
    if !email.contains('@') || body.amount <= 0 {
        return reply(
            StatusCode::BAD_REQUEST,
            serde_json::json!({ "success": false, "message": "Give an email address and an amount above 0" }),
        );
    }
    let failed = |what: &str, e: String| {
        app_log!(error, error = %e, email = %email, "Could not spend credits: {}", what);
        reply(
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({ "success": false, "message": "Credits could not be charged" }),
        )
    };

    // What the account can add. Read without creating an account: someone who
    // only has a plan has none, and needs none.
    let tenant = match tenant_management::find_default_tenant(&store, &email).await {
        Ok(t) => t,
        Err(e) => return failed("account lookup", e.to_string()),
    };
    let balance = match &tenant {
        Some(t) => match store.get_credit_balance(&t.id).await {
            Ok(b) => b.max(0),
            Err(e) => return failed("balance read", e.to_string()),
        },
        None => 0,
    };
    let plan_left = match plan_credits_left_for(&store, &email).await {
        Ok(n) => n,
        Err(e) => return failed("plan read", e),
    };
    if plan_left + balance < body.amount {
        return reply(
            StatusCode::PAYMENT_REQUIRED,
            serde_json::json!({
                "success": false,
                "message": "Not enough credits",
                "available": plan_left + balance,
                "plan_credits_left": plan_left,
                "balance": balance,
            }),
        );
    }

    // The plan first, then the balance for whatever it could not cover.
    let (plan_id, from_plan) = match take_from_plan(&store, &email, body.amount).await {
        Ok(Some((id, took, _left))) => (Some(id), took),
        Ok(None) => (None, 0),
        Err(e) => return failed("plan charge", e),
    };
    let from_balance = body.amount - from_plan;
    let mut balance_after = balance;
    if from_balance > 0 {
        // Another charge may have spent the balance since it was read.
        let paid = match &tenant {
            Some(t) if balance >= from_balance => store
                .update_credit_balance(&t.id, &email, -from_balance, &body.action_type, body.description.as_deref())
                .await
                .map_err(|e| e.to_string()),
            _ => Err("the balance no longer covers it".to_string()),
        };
        match paid {
            Ok(after) => balance_after = after,
            Err(e) => {
                if let Some(id) = &plan_id {
                    if let Err(e) = give_back_to_plan(&store, id, from_plan).await {
                        app_log!(error, error = %e, subscription_id = %id, credits = from_plan, "Could not give credits back to a plan");
                    }
                }
                app_log!(warn, email = %email, reason = %e, "Credits short after all; nothing charged");
                return reply(
                    StatusCode::PAYMENT_REQUIRED,
                    serde_json::json!({ "success": false, "message": "Not enough credits", "available": plan_left + balance }),
                );
            }
        }
    }
    if let Some(id) = &plan_id {
        log_plan_use(&store, id, &email, caller.name(), &body, from_plan).await;
    }
    app_log!(info, email = %email, caller = %caller.name(), action = %body.action_type,
        from_plan = from_plan, from_balance = from_balance, "Credits spent");

    // Across every live plan of the email, as the balance read reports it.
    let plan_credits_left = (plan_left - from_plan).max(0);
    reply(
        StatusCode::OK,
        serde_json::json!({
            "success": true,
            "from_plan": from_plan,
            "from_balance": from_balance,
            "plan_credits_left": plan_credits_left,
            "balance": balance_after,
            "available": plan_credits_left + balance_after,
        }),
    )
}

#[cfg(test)]
mod tests {
    // The charge order lives in SQL; these keep its two rules written down.
    use super::{usage_model, LIVE, PLAN_ORDER};

    #[test]
    fn only_a_paid_up_plan_in_its_month_gives_credits() {
        assert!(LIVE.contains("'active', 'trialing'") && LIVE.contains("period_end > NOW()"));
    }

    #[test]
    fn a_paid_plan_is_spent_before_the_free_allowance() {
        assert!(PLAN_ORDER.starts_with("s.is_free ASC"));
    }

    #[test]
    fn a_named_model_keeps_the_action_as_what_the_subscriber_sees() {
        assert_eq!(usage_model("cv_generation", Some("deepseek-chat")), ("deepseek-chat".into(), Some("cv_generation".into())));
        assert_eq!(usage_model("cv_generation", Some("  ")), ("cv_generation".into(), None));
        assert_eq!(usage_model("cv_generation", None), ("cv_generation".into(), None));
    }
}
