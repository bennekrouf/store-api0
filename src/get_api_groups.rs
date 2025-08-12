use crate::endpoint_store::EndpointStore;

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

#[derive(serde::Serialize)]
pub struct EnhancedApiGroupsResponse {
    pub success: bool,
    pub api_groups: Vec<crate::endpoint_store::ApiGroupWithEndpoints>,
    pub message: String,
    // Auto-generated API key for new users
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_prefix: Option<String>,
}

pub async fn get_api_groups(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP get API groups request");

    // Check if this is a new user by looking for existing API keys
    let is_new_user = match store.get_api_keys_status(&email).await {
        Ok(status) => {
            tracing::info!(
                email = %email,
                has_keys = status.has_keys,
                active_key_count = status.active_key_count,
                keys_length = status.keys.len(),
                "API key status check result"
            );
            !status.has_keys
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                email = %email,
                "Failed to check API key status, assuming new user"
            );
            true // Assume new user if we can't check
        }
    };

    tracing::info!(
        email = %email,
        is_new_user = is_new_user,
        "Computed is_new_user flag"
    );

    let mut response = EnhancedApiGroupsResponse {
        success: true,
        api_groups: vec![],
        message: "API groups successfully retrieved".to_string(),
        api_key: None,
        key_prefix: None,
    };

    // Auto-create API key for new users
    if is_new_user {
        tracing::info!(email = %email, "New user detected, creating default API key with $5 credit");

        match store.generate_api_key(&email, "Default API Key").await {
            Ok((api_key, key_prefix, _)) => {
                // Add $5 default credit (500 cents)
                if let Err(e) = store.update_credit_balance(&email, 500).await {
                    tracing::warn!(
                        error = %e,
                        email = %email,
                        "Failed to add default credit, but continuing"
                    );
                }

                tracing::info!(
                    email = %email,
                    key_prefix = %key_prefix,
                    "Auto-generated API key with $5 credit for new user"
                );
                response.api_key = Some(api_key);
                response.key_prefix = Some(key_prefix);
                response.message =
                    "Welcome! Your API key and $5 credit have been added to your account."
                        .to_string();
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    "Failed to auto-generate API key for new user"
                );
                // Continue anyway - user can manually create key later
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
