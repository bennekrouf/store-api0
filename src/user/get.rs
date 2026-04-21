use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for getting user preferences
pub async fn get_user_preferences(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    app_log!(info, email = %email, "Received HTTP get user preferences request");

    match store.get_user_preferences(&email).await {
        Ok(preferences) => {
            app_log!(info,
                email = %email,
                hidden_count = preferences.hidden_defaults.len(),
                "Successfully retrieved user preferences"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "preferences": preferences,
            }))
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                email = %email,
                "Failed to retrieve user preferences"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error: {}", e),
            }))
        }
    }
}
