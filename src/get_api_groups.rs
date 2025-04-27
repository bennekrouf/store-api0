use crate::{endpoint_store::EndpointStore, models::ApiGroupsResponse};

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

pub async fn get_api_groups(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP get API groups request");

    match store.get_api_groups_with_preferences(&email).await {
        Ok(api_groups) => {
            tracing::info!(
                email = %email,
                group_count = api_groups.len(),
                "Successfully retrieved API groups with preferences applied"
            );
            HttpResponse::Ok().json(ApiGroupsResponse {
                success: true,
                api_groups,
                message: "API groups successfully retrieved".to_string(),
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to retrieve API groups"
            );
            HttpResponse::InternalServerError().json(ApiGroupsResponse {
                success: false,
                api_groups: vec![],
                message: format!("Error: {}", e),
            })
        }
    }
}
