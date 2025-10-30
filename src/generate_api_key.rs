use crate::endpoint_store::EndpointStore;
use crate::endpoint_store::GenerateKeyRequest;

use crate::app_log;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;
// Handler for generating a new API key
pub async fn generate_api_key(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<GenerateKeyRequest>,
) -> impl Responder {
    let email = &request.email;
    let key_name = &request.key_name;

    app_log!(info,
        email = %email,
        key_name = %key_name,
        "Received HTTP generate API key request"
    );

    match store.generate_api_key(email, key_name).await {
        Ok((key, key_prefix, _)) => {
            app_log!(info,
                email = %email,
                key_prefix = %key_prefix,
                "Successfully generated API key"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "API key generated successfully",
                "key": key,
                "keyPrefix": key_prefix,
            }))
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                email = %email,
                "Failed to generate API key"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to generate API key: {}", e),
            }))
        }
    }
}
