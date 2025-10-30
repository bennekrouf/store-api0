use crate::app_log;
use crate::{
    endpoint_store::{generate_id_from_text, EndpointStore},
    models::AddApiGroupRequest,
};
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

pub async fn add_api_group(
    store: web::Data<Arc<EndpointStore>>,
    add_data: web::Json<AddApiGroupRequest>,
) -> impl Responder {
    let email = &add_data.email;
    let mut api_group = add_data.api_group.clone();

    app_log!(info,
        email = %email,
        group_name = %api_group.group.name,
        "Received HTTP add API group request"
    );

    // Validate group data
    if api_group.group.name.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "API group name cannot be empty"
        }));
    }

    if api_group.group.base.trim().is_empty() {
        api_group.group.base = "https://api.example.com".to_string();
        // return HttpResponse::BadRequest().json(serde_json::json!({
        //     "success": false,
        //     "message": "Base URL cannot be empty"
        // }));
    }

    // Generate group ID if not provided
    if api_group.group.id.trim().is_empty() {
        api_group.group.id = generate_id_from_text(&api_group.group.name);
    }

    // Process endpoints with inheritance and auto-generation
    for endpoint in &mut api_group.endpoints {
        // Generate endpoint ID if not provided
        if endpoint.id.trim().is_empty() {
            endpoint.id = generate_id_from_text(&endpoint.text);
        }

        // Set group_id from parent group
        endpoint.group_id = api_group.group.id.clone();

        // Inherit base URL from group if endpoint base is empty
        if endpoint.base.trim().is_empty() {
            endpoint.base = api_group.group.base.clone();
        }
    }

    // Add the API group (don't replace existing ones)
    match store.add_user_api_group(email, &api_group).await {
        Ok(endpoint_count) => {
            app_log!(info,
                email = %email,
                group_id = %api_group.group.id,
                endpoint_count = endpoint_count,
                "Successfully added API group"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "API group successfully added",
                "group_id": api_group.group.id,
                "endpoint_count": endpoint_count
            }))
        }
        Err(e) => {
            app_log!(error,
                error = %e,
                email = %email,
                group_id = %api_group.group.id,
                "Failed to add API group"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to add API group: {}", e)
            }))
        }
    }
}
