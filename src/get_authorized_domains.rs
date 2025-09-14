use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;

#[derive(serde::Serialize)]
pub struct AuthorizedDomainsResponse {
    pub success: bool,
    pub domains: Vec<String>,
}

/// Handler for getting all authorized domains (used by gateway for CORS)
pub async fn get_authorized_domains(store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    tracing::info!("Received HTTP get authorized domains request");

    match store.get_all_authorized_domains().await {
        Ok(domains) => {
            tracing::info!(
                domain_count = domains.len(),
                "Successfully retrieved authorized domains"
            );
            HttpResponse::Ok().json(AuthorizedDomainsResponse {
                success: true,
                domains,
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "Failed to retrieve authorized domains"
            );
            HttpResponse::InternalServerError().json(AuthorizedDomainsResponse {
                success: false,
                domains: vec![],
            })
        }
    }
}
