use crate::endpoint_store::EndpointStore;

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

#[derive(serde::Serialize)]
pub struct EnhancedApiGroupsResponse {
    pub success: bool,
    pub api_groups: Vec<crate::endpoint_store::ApiGroupWithEndpoints>,
    pub message: String,
    // Remove API key fields since we won't auto-generate keys
    pub is_new_user: bool,
    pub credit_balance: i64,
}

pub async fn get_api_groups(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP get API groups request");

    // Check if this is a new user by looking for existing API keys
    let (is_new_user, current_balance) = match store.get_api_keys_status(&email).await {
        Ok(status) => {
            tracing::info!(
                email = %email,
                has_keys = status.has_keys,
                active_key_count = status.active_key_count,
                current_balance = status.balance,
                "API key status check result"
            );
            (!status.has_keys, status.balance)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                email = %email,
                "Failed to check API key status, assuming new user"
            );
            (true, 0) // Assume new user if we can't check
        }
    };

    tracing::info!(
        email = %email,
        is_new_user = is_new_user,
        current_balance = current_balance,
        "Computed user status"
    );

    let mut response = EnhancedApiGroupsResponse {
        success: true,
        api_groups: vec![],
        message: "API groups successfully retrieved".to_string(),
        is_new_user,
        credit_balance: current_balance,
    };

    // Add default credit for new users (without creating an API key)
    if is_new_user && current_balance == 0 {
        tracing::info!(email = %email, "New user detected, adding $5 default credit");

        match store.update_credit_balance(&email, 500).await {
            Ok(new_balance) => {
                tracing::info!(
                    email = %email,
                    new_balance = new_balance,
                    "Added $5 default credit for new user"
                );
                response.credit_balance = new_balance;
                response.message = "Welcome! $5 credit has been added to your account. Create an API key to start using the service.".to_string();
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    "Failed to add default credit for new user"
                );
                response.message =
                    "Welcome! Please create an API key to start using the service.".to_string();
            }
        }
    }

    // Get API groups with preferences applied
    match store.get_api_groups_with_preferences(&email).await {
        Ok(api_groups) => {
            tracing::info!(
                email = %email,
                group_count = api_groups.len(),
                is_new_user = is_new_user,
                credit_balance = response.credit_balance,
                "Successfully retrieved API groups"
            );

            response.api_groups = api_groups;
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to retrieve API groups"
            );

            response.success = false;
            response.message = format!("Error: {}", e);
            HttpResponse::InternalServerError().json(response)
        }
    }
}
