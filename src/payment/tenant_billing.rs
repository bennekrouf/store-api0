// src/payment/tenant_billing.rs
//
// Who pays for a tenant's paid tools (mcp_tools.cost_credits). Each tenant picks
// one way, tenants.billing_mode, and only that one is ever read or charged:
//
//   tenant  the tenant's balance (tenants.credit_balance, filled by top-ups)
//           pays for every call made with its keys, whoever makes it.
//   user    the person calling pays from their own plan's monthly credits
//           (payment::subscription). The tenant's balance is never touched, and
//           someone without a live plan cannot run paid tools.
//
//   GET  /api/internal/billing/{tenant_id}?email=…  → { mode, available }
//   POST /api/internal/billing/charge               { tenant_id, email, amount, action_type,
//                                                     description? } → { mode, charged, available }
//   GET  /api/user/billing-mode?email=…             → { mode, can_edit } for the caller's tenant
//   PUT  /api/user/billing-mode                     { email, mode } → { mode }; owners and admins
//
// All require X-Internal-Secret; the gateway is the only caller.
//
// The partner credit routes follow the same mode when the service key is bound
// to a workspace (middleware::service_key): payment::spend reads and spends with
// spend_strict, payment::balance reports only the payer's credits, and
// payment::update_balance sends a refund back where the mode says.
//
// A charge follows a call that already ran, so it is always taken: the balance
// may end below zero, and a plan is used up to its allowance, as the AI endpoint
// does. The read before the call is what refuses one that cannot be paid.

use crate::app_log;
use crate::endpoint_store::tenant_management::get_default_tenant;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use crate::middleware::service_key::Caller;
use crate::payment::spend::{self, SpendRequest};
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

/// Roles that may change who pays. It moves money, so members may not.
const EDITOR_ROLES: &[&str] = &["owner", "admin"];

/// What a plan's usage history shows these charges as.
const PRODUCT: &str = "mcp";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BillingMode {
    Tenant,
    User,
}

impl BillingMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tenant => "tenant",
            Self::User => "user",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "tenant" => Some(Self::Tenant),
            "user" => Some(Self::User),
            _ => None,
        }
    }
}

pub async fn billing_mode(store: &EndpointStore, tenant_id: &str) -> Result<BillingMode, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let row = client
        .query_opt("SELECT billing_mode FROM tenants WHERE id = $1", &[&tenant_id])
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("No tenant {}", tenant_id))?;
    let mode: String = row.get(0);
    BillingMode::parse(&mode).ok_or_else(|| format!("Tenant {} has an unknown billing mode '{}'", tenant_id, mode))
}

/// What `email` may spend on `tenant_id`'s tools: the tenant's balance, or the
/// person's plan credits left this month — never the two added up.
pub async fn available(store: &EndpointStore, tenant_id: &str, email: &str) -> Result<(BillingMode, i64), String> {
    let mode = billing_mode(store, tenant_id).await?;
    let credits = match mode {
        BillingMode::Tenant => store.get_credit_balance(tenant_id).await.map_err(|e| e.to_string())?,
        BillingMode::User => spend::plan_credits_left_for(store, email).await?,
    };
    Ok((mode, credits))
}

/// Charges one call to whoever the tenant's mode names. Returns what was
/// charged and what that payer has left.
pub async fn charge(
    store: &EndpointStore,
    tenant_id: &str,
    email: &str,
    amount: i64,
    action_type: &str,
    description: Option<&str>,
) -> Result<(BillingMode, i64, i64), String> {
    let mode = billing_mode(store, tenant_id).await?;
    match mode {
        BillingMode::Tenant => {
            let after = store
                .update_credit_balance(tenant_id, email, -amount, action_type, description)
                .await
                .map_err(|e| e.to_string())?;
            Ok((mode, amount, after))
        }
        BillingMode::User => {
            let email = email.trim().to_lowercase();
            // Someone can hold a paid plan and the free allowance at once; each
            // take spends one of them, so keep going until the call is paid or
            // nothing is left.
            let mut charged = 0;
            while charged < amount {
                let Some((subscription_id, took, _left)) = spend::take_from_plan(store, &email, amount - charged).await? else {
                    break;
                };
                let logged = SpendRequest {
                    email: email.clone(),
                    amount: took,
                    action_type: action_type.to_string(),
                    description: description.map(str::to_string),
                    input_tokens: None,
                    output_tokens: None,
                    model: None,
                };
                spend::log_plan_use(store, &subscription_id, &email, PRODUCT, &logged, took).await;
                charged += took;
            }
            let left = spend::plan_credits_left_for(store, &email).await?;
            Ok((mode, charged, left))
        }
    }
}

/// The workspace a service key is bound to, and how it bills. `None` for the
/// gateway and for keys bound to no workspace, which keep the older rule.
pub async fn caller_mode(store: &EndpointStore, caller: &Caller) -> Result<Option<(String, BillingMode)>, String> {
    let Some(tenant_id) = caller.tenant_id() else {
        return Ok(None);
    };
    let mode = billing_mode(store, tenant_id).await?;
    Ok(Some((tenant_id.to_string(), mode)))
}

#[derive(Debug, PartialEq)]
pub enum StrictSpend {
    /// Paid in full; `available` is what the payer has left.
    Paid { from_plan: i64, from_balance: i64, available: i64 },
    /// Nothing was taken: the payer has only `available`.
    Short { available: i64 },
}

/// Takes `amount` before an operation runs, from the one payer the mode names,
/// or nothing at all when that payer cannot cover it. Unlike `charge`, which
/// follows a call that already ran, this one refuses.
#[allow(clippy::too_many_arguments)]
pub async fn spend_strict(
    store: &EndpointStore,
    tenant_id: &str,
    mode: BillingMode,
    email: &str,
    amount: i64,
    action_type: &str,
    description: Option<&str>,
    product: &str,
) -> Result<StrictSpend, String> {
    match mode {
        BillingMode::Tenant => {
            let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
            // Checked and taken in one statement, so two charges at once cannot
            // both spend the same credits.
            let row = client
                .query_opt(
                    "UPDATE tenants SET credit_balance = credit_balance - $1 \
                     WHERE id = $2 AND credit_balance >= $1 RETURNING credit_balance",
                    &[&amount, &tenant_id],
                )
                .await
                .map_err(|e| e.to_string())?;
            let Some(row) = row else {
                let balance = store.get_credit_balance(tenant_id).await.map_err(|e| e.to_string())?;
                return Ok(StrictSpend::Short { available: balance.max(0) });
            };
            let after: i64 = row.get(0);
            let description = description.map(str::to_string);
            if let Err(e) = client
                .execute(
                    "INSERT INTO credit_transactions (tenant_id, email, amount, balance_after, action_type, description) \
                     VALUES ($1, $2, $3, $4, $5, $6)",
                    &[&tenant_id, &email, &-amount, &after, &action_type, &description],
                )
                .await
            {
                app_log!(error, error = %e, tenant_id = %tenant_id, email = %email, "Failed to record a workspace charge");
            }
            Ok(StrictSpend::Paid { from_plan: 0, from_balance: amount, available: after })
        }
        BillingMode::User => {
            let email = email.trim().to_lowercase();
            let left = spend::plan_credits_left_for(store, &email).await?;
            if left < amount {
                return Ok(StrictSpend::Short { available: left });
            }
            // A paid plan and the free allowance can each cover part of it.
            let mut taken: Vec<(String, i64)> = Vec::new();
            let mut total = 0;
            while total < amount {
                let Some((subscription_id, took, _)) = spend::take_from_plan(store, &email, amount - total).await? else {
                    break;
                };
                total += took;
                taken.push((subscription_id, took));
            }
            if total < amount {
                // Another charge spent the plan since it was read: put back what
                // this one took, so a refused operation costs nothing.
                for (subscription_id, took) in &taken {
                    if let Err(e) = spend::give_back_to_plan(store, subscription_id, *took).await {
                        app_log!(error, error = %e, subscription_id = %subscription_id, credits = took, "Could not give credits back to a plan");
                    }
                }
                let left = spend::plan_credits_left_for(store, &email).await?;
                return Ok(StrictSpend::Short { available: left });
            }
            for (subscription_id, took) in &taken {
                let logged = SpendRequest {
                    email: email.clone(),
                    amount: *took,
                    action_type: action_type.to_string(),
                    description: description.map(str::to_string),
                    input_tokens: None,
                    output_tokens: None,
                    model: None,
                };
                spend::log_plan_use(store, subscription_id, &email, product, &logged, *took).await;
            }
            let left = spend::plan_credits_left_for(store, &email).await?;
            Ok(StrictSpend::Paid { from_plan: amount, from_balance: 0, available: left })
        }
    }
}

#[derive(Debug)]
pub enum SetModeError {
    Forbidden,
    Store(String),
}

/// Sets who pays on `email`'s active tenant. Returns the tenant's id.
pub async fn set_billing_mode(store: &EndpointStore, email: &str, mode: BillingMode) -> Result<String, SetModeError> {
    let tenant = get_default_tenant(store, email).await.map_err(|e| SetModeError::Store(e.to_string()))?;
    if !can_edit(store, &tenant.id, email).await.map_err(SetModeError::Store)? {
        return Err(SetModeError::Forbidden);
    }
    let client = store.get_admin_conn().await.map_err(|e| SetModeError::Store(e.to_string()))?;
    client
        .execute("UPDATE tenants SET billing_mode = $1 WHERE id = $2", &[&mode.as_str(), &tenant.id])
        .await
        .map_err(|e| SetModeError::Store(e.to_string()))?;
    Ok(tenant.id)
}

pub(crate) async fn can_edit(store: &EndpointStore, tenant_id: &str, email: &str) -> Result<bool, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let row = client
        .query_opt(
            "SELECT role FROM tenant_users WHERE tenant_id = $1 AND LOWER(email) = LOWER($2)",
            &[&tenant_id, &email],
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(row.map(|r| r.get::<_, String>(0)).is_some_and(|role| EDITOR_ROLES.contains(&role.as_str())))
}

// ── HTTP ─────────────────────────────────────────────────────────────────────

fn failed(what: &str, e: String) -> HttpResponse {
    app_log!(error, error = %e, "Tenant billing: {}", what);
    HttpResponse::InternalServerError().json(serde_json::json!({ "success": false, "error": format!("Could not {}", what) }))
}

#[derive(Deserialize)]
pub struct EmailQuery {
    pub email: String,
}

/// GET /api/internal/billing/{tenant_id}?email=…
pub async fn available_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    tenant_id: web::Path<String>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    match available(&store, &tenant_id, &query.email).await {
        Ok((mode, credits)) => HttpResponse::Ok().json(serde_json::json!({
            "success": true,
            "mode": mode.as_str(),
            "available": credits,
        })),
        Err(e) => failed("read the credits available", e),
    }
}

#[derive(Deserialize)]
pub struct ChargeBody {
    pub tenant_id: String,
    pub email: String,
    pub amount: i64,
    pub action_type: String,
    pub description: Option<String>,
}

/// POST /api/internal/billing/charge
pub async fn charge_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<ChargeBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    if body.amount <= 0 {
        return HttpResponse::BadRequest().json(serde_json::json!({ "success": false, "error": "amount must be above 0" }));
    }
    match charge(&store, &body.tenant_id, &body.email, body.amount, &body.action_type, body.description.as_deref()).await {
        Ok((mode, charged, left)) => {
            if charged < body.amount {
                app_log!(warn, tenant_id = %body.tenant_id, email = %body.email, charged = charged, amount = body.amount,
                    "Plan ran out during a paid call; charged what was left");
            }
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "mode": mode.as_str(),
                "charged": charged,
                "available": left,
            }))
        }
        Err(e) => failed("charge the call", e),
    }
}

/// GET /api/user/billing-mode?email=…
pub async fn get_my_billing_mode_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant = match get_default_tenant(&store, &query.email).await {
        Ok(t) => t,
        Err(e) => return failed("find the workspace", e.to_string()),
    };
    let mode = match billing_mode(&store, &tenant.id).await {
        Ok(m) => m,
        Err(e) => return failed("read who pays", e),
    };
    let editable = match can_edit(&store, &tenant.id, &query.email).await {
        Ok(b) => b,
        Err(e) => return failed("read the caller's role", e),
    };
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tenant_id": tenant.id,
        "mode": mode.as_str(),
        "balance": tenant.credit_balance,
        "can_edit": editable,
    }))
}

#[derive(Deserialize)]
pub struct SetModeBody {
    pub email: String,
    pub mode: String,
}

/// PUT /api/user/billing-mode
pub async fn set_my_billing_mode_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SetModeBody>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let Some(mode) = BillingMode::parse(&body.mode) else {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({ "success": false, "error": "mode must be 'tenant' or 'user'" }));
    };
    match set_billing_mode(&store, &body.email, mode).await {
        Ok(tenant_id) => {
            app_log!(info, tenant_id = %tenant_id, email = %body.email, mode = mode.as_str(), "Billing mode set");
            HttpResponse::Ok().json(serde_json::json!({ "success": true, "mode": mode.as_str() }))
        }
        Err(SetModeError::Forbidden) => HttpResponse::Forbidden().json(serde_json::json!({
            "success": false,
            "error": "Only the workspace's owners and admins can change who pays",
        })),
        Err(SetModeError::Store(e)) => failed("save who pays", e),
    }
}

#[cfg(test)]
mod tests {
    use super::BillingMode;

    #[test]
    fn a_mode_reads_back_as_written() {
        for mode in [BillingMode::Tenant, BillingMode::User] {
            assert_eq!(BillingMode::parse(mode.as_str()), Some(mode));
        }
    }

    #[test]
    fn there_is_no_mode_that_mixes_the_two() {
        assert_eq!(BillingMode::parse("both"), None);
        assert_eq!(BillingMode::parse(""), None);
    }
}
