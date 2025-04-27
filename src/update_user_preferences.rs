use crate::endpoint_store::EndpointStore;
use crate::endpoint_store::UpdatePreferenceRequest;

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for updating user preferences
pub async fn update_user_preferences(
    store: web::Data<Arc<EndpointStore>>,
    update_data: web::Json<UpdatePreferenceRequest>,
) -> impl Responder {
    let email = &update_data.email;
    let action = &update_data.action;
    let endpoint_id = &update_data.endpoint_id;

    tracing::info!(
        email = %email,
        action = %action,
        endpoint_id = %endpoint_id,
        "Received HTTP update user preferences request"
    );

    match store
        .update_user_preferences(email, action, endpoint_id)
        .await
    {
        Ok(_) => {
            tracing::info!(
                email = %email,
                action = %action,
                endpoint_id = %endpoint_id,
                "Successfully updated user preferences"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "User preferences successfully updated",
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to update user preferences"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to update user preferences: {}", e),
            }))
        }
    }
}
