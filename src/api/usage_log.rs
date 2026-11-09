// src/log_api_usage.rs
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::app_log;
use crate::email::{send_async, EmailKind};
use crate::endpoint_store::{tenant_management, EndpointStore, LogApiUsageRequest, LogApiUsageResponse};
use crate::payment::{plans, tenant_billing};

/// Takes a logged AI call's credits from whoever its workspace's billing mode
/// names, priced as any AI call is (plans::Pricing), and notes what was charged
/// on the log row for the super admin's ledger (admin/llm_calls.rs). The tokens
/// are already spent, so a failure here is logged and the call stays logged.
async fn charge_usage(store: &EndpointStore, request: &LogApiUsageRequest, log_id: &str) {
    let Some(usage) = &request.usage else { return };
    let cost = match plans::effective_pricing(store).await {
        Ok(pricing) => pricing.credits_for(&usage.model, usage.total_tokens),
        Err(e) => {
            app_log!(error, error = %e, email = %request.email, "Could not price an AI call");
            return;
        }
    };
    if cost == 0 {
        return;
    }
    // The tenant the call names (a messaging bot's workspace, say), else the
    // person's default one.
    let tenant_id = match request.tenant_id.as_deref().filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => match tenant_management::get_default_tenant(store, &request.email).await {
            Ok(t) => t.id,
            Err(e) => {
                app_log!(error, error = %e, email = %request.email, "Failed to resolve tenant for credit deduction");
                return;
            }
        },
    };
    let charged = match tenant_billing::charge(store, &tenant_id, &request.email, cost, "api_usage", Some(&request.endpoint_path)).await {
        Ok((_, charged, _)) => charged,
        Err(e) => {
            app_log!(error, error = %e, email = %request.email, tenant_id = %tenant_id, "Failed to deduct credits");
            return;
        }
    };
    app_log!(info, email = %request.email, tenant_id = %tenant_id, cost = cost, charged = charged, "Credits deducted for usage");
    let noted = match store.get_admin_conn().await {
        Ok(client) => client
            .execute(
                "UPDATE api_usage_logs SET metadata = COALESCE(metadata, '{}'::jsonb) || jsonb_build_object('credits', $1::BIGINT) WHERE id = $2",
                &[&charged, &log_id],
            )
            .await
            .map_err(|e| e.to_string()),
        Err(e) => Err(e.to_string()),
    };
    if let Err(e) = noted {
        app_log!(warn, error = %e, log_id = %log_id, "Could not note the credits on the usage log");
    }
}

/// Handler for logging detailed API usage with token information
pub async fn log_api_usage(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<LogApiUsageRequest>,
) -> impl Responder {
    let log_request = request.into_inner();

    app_log!(info,
        key_id = %log_request.key_id,
        email = %log_request.email,
        endpoint = %log_request.endpoint_path,
        method = %log_request.method,
        has_token_usage = log_request.usage.is_some(),
        total_tokens = log_request.usage.as_ref().map(|u| u.total_tokens),
        model = log_request.usage.as_ref().map(|u| u.model.as_str()).unwrap_or("none"),
        "Received HTTP log API usage request with token data"
    );

    match store.log_api_usage(&log_request).await {
        Ok(log_id) => {
            app_log!(info,
                key_id = %log_request.key_id,
                log_id = %log_id,
                total_tokens = log_request.usage.as_ref().map(|u| u.total_tokens),
                "Successfully logged API usage with token data"
            );

            charge_usage(&store, &log_request, &log_id).await;

            // First-call milestone: fire once per user (idempotent via first_call_at column).
            let store2 = store.as_ref().clone();
            let email2 = log_request.email.clone();
            let endpoint2 = log_request.endpoint_path.clone();
            tokio::spawn(async move {
                if let Ok(client) = store2.get_admin_conn().await {
                    let already: bool = client
                        .query_one(
                            "SELECT (first_call_at IS NOT NULL) FROM tenants t
                             JOIN user_preferences up ON up.default_tenant_id = t.id
                             WHERE up.email = $1",
                            &[&email2],
                        )
                        .await
                        .map(|r| r.get(0))
                        .unwrap_or(true);
                    if !already {
                        let _ = client.execute(
                            "UPDATE tenants SET first_call_at = NOW()
                             FROM user_preferences up
                             WHERE up.default_tenant_id = tenants.id AND up.email = $1",
                            &[&email2],
                        ).await;
                        send_async(store2, email2, EmailKind::FirstCallMilestone { endpoint: endpoint2 });
                    }
                }
            });

            HttpResponse::Ok().json(LogApiUsageResponse {
                success: true,
                message: "API usage logged successfully".to_string(),
                log_id: Some(log_id),
                ..Default::default()
            })
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                key_id = %log_request.key_id,
                "Failed to log API usage"
            );
            HttpResponse::InternalServerError().json(LogApiUsageResponse {
                success: false,
                message: format!("Failed to log API usage: {}", e),
                log_id: None,
                ..Default::default()
            })
        }
    }
}
