// src/log_api_usage.rs
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::app_log;
use crate::email::{send_async, EmailKind};
use crate::endpoint_store::{EndpointStore, LogApiUsageRequest, LogApiUsageResponse};
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

            // Deduct credits if usage is present
            if let Some(usage) = &log_request.usage {
                // Pricing model:
                // 1 Cent = 100 Internal Credits (allows fractional cents)
                // Cost: $0.002 per 1k tokens = 0.2 cents = 20 Credits
                // Formula: (total_tokens * 20) / 1000 = total_tokens / 50
                // Minimum 1 credit if tokens > 0
                let total_tokens = usage.total_tokens;
                let cost = if total_tokens > 0 {
                    (total_tokens as i64 / 50).max(1)
                } else {
                    0
                };

                if cost > 0 {
                    app_log!(info, email = %log_request.email, cost = cost, "Deducting credits for usage");
                    // Resolve tenant_id
                    let tenant_id = match crate::endpoint_store::tenant_management::get_default_tenant(&store, &log_request.email).await {
                        Ok(t) => t.id,
                        Err(e) => {
                             app_log!(error, error = %e, email = %log_request.email, "Failed to resolve tenant for credit deduction");
                             "".to_string()
                        }
                    };
                    
                    if !tenant_id.is_empty() {
                        // Pass negative amount to decrement
                        if let Err(e) = store.update_credit_balance(&tenant_id, &log_request.email, -cost, "api_usage", None).await {
                             app_log!(error, error = %e, email = %log_request.email, "Failed to deduct credits");
                        }
                    }
                }
            }

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
            })
        }
    }
}
