use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use base64::{engine::general_purpose, Engine as _};
use crate::endpoint_store::{ApiGroupWithEndpoints, ApiStorage, EndpointStore, generate_id_from_text};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use crate::endpoint_store::UpdatePreferenceRequest;

// Request and Response models
#[derive(Debug, Clone, Deserialize)]
pub struct UploadRequest {
    email: String,
    file_name: String,
    file_content: String, // Base64 encoded
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    success: bool,
    message: String,
    imported_count: i32,
    group_count: i32,
}

#[derive(Debug, Serialize)]
pub struct ApiGroupsResponse {
    success: bool,
    api_groups: Vec<ApiGroupWithEndpoints>,
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddApiGroupRequest {
    email: String,
    api_group: ApiGroupWithEndpoints,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateApiGroupRequest {
    email: String,
    group_id: String,
    api_group: ApiGroupWithEndpoints,
}

// Handler for uploading API configuration
async fn upload_api_config(
    store: web::Data<Arc<EndpointStore>>,
    upload_data: web::Json<UploadRequest>,
) -> impl Responder {
    tracing::info!(
        email = %upload_data.email,
        filename = %upload_data.file_name,
        "Received HTTP upload request via Actix"
    );

    // Decode base64 content
    let file_bytes = match general_purpose::STANDARD.decode(&upload_data.file_content) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!(error = %e, "Failed to decode base64 content");
            return HttpResponse::BadRequest().json(UploadResponse {
                success: false,
                message: format!("Invalid base64 encoding: {}", e),
                imported_count: 0,
                group_count: 0,
            });
        }
    };

    // Convert to string
    let file_content = match String::from_utf8(file_bytes) {
        Ok(content) => content,
        Err(e) => {
            tracing::error!(error = %e, "File content is not valid UTF-8");
            return HttpResponse::BadRequest().json(UploadResponse {
                success: false,
                message: "File content must be valid UTF-8 text".to_string(),
                imported_count: 0,
                group_count: 0,
            });
        }
    };

    // Parse based on file extension
    let api_storage =
        if upload_data.file_name.ends_with(".yaml") || upload_data.file_name.ends_with(".yml") {
            // Parse YAML
            match serde_yaml::from_str::<ApiStorage>(&file_content) {
                Ok(storage) => storage,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to parse YAML content");
                    return HttpResponse::BadRequest().json(UploadResponse {
                        success: false,
                        message: "Invalid YAML format. Expected structure with 'api_groups'."
                            .to_string(),
                        imported_count: 0,
                        group_count: 0,
                    });
                }
            }
        } else if upload_data.file_name.ends_with(".json") {
            // Parse JSON
            match serde_json::from_str::<ApiStorage>(&file_content) {
                Ok(storage) => storage,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to parse JSON content");
                    return HttpResponse::BadRequest().json(UploadResponse {
                        success: false,
                        message: "Invalid JSON format. Expected structure with 'api_groups'."
                            .to_string(),
                        imported_count: 0,
                        group_count: 0,
                    });
                }
            }
        } else {
            return HttpResponse::BadRequest().json(UploadResponse {
                success: false,
                message: "Unsupported file format. Use YAML or JSON.".to_string(),
                imported_count: 0,
                group_count: 0,
            });
        };

    // Process and validate each API group
    let group_count = api_storage.api_groups.len();

    if group_count == 0 {
        return HttpResponse::BadRequest().json(UploadResponse {
            success: false,
            message: "No API groups found in the file".to_string(),
            imported_count: 0,
            group_count: 0,
        });
    }

    // Generate IDs for groups and endpoints if needed
    let mut processed_groups = Vec::new();

    for mut group in api_storage.api_groups {
        // Generate ID for group if not provided
        if group.group.id.is_empty() {
            group.group.id = generate_id_from_text(&group.group.name);
        }

        // Process endpoints
        let mut processed_endpoints = Vec::new();
        for mut endpoint in group.endpoints {
            // Generate ID for endpoint if not provided
            if endpoint.id.is_empty() {
                endpoint.id = generate_id_from_text(&endpoint.text);
            }

            // Set group_id reference
            endpoint.group_id = group.group.id.clone();

            processed_endpoints.push(endpoint);
        }

        let processed_group = ApiGroupWithEndpoints {
            group: group.group,
            endpoints: processed_endpoints,
        };

        processed_groups.push(processed_group);
    }

    // Replace user API groups
    match store
        .replace_user_api_groups(&upload_data.email, processed_groups)
        .await
    {
        Ok(endpoint_count) => {
            tracing::info!(
                email = %upload_data.email,
                endpoint_count = endpoint_count,
                group_count = group_count,
                "Successfully imported API groups and endpoints via HTTP API"
            );
            HttpResponse::Ok().json(UploadResponse {
                success: true,
                message: "API groups and endpoints successfully imported".to_string(),
                imported_count: endpoint_count as i32,
                group_count: group_count as i32,
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %upload_data.email,
                "Failed to import API groups via HTTP API"
            );
            HttpResponse::InternalServerError().json(UploadResponse {
                success: false,
                message: format!("Failed to import API groups: {}", e),
                imported_count: 0,
                group_count: 0,
            })
        }
    }
}

async fn get_api_groups(
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

// Handler for adding a new API group
async fn add_api_group(
    store: web::Data<Arc<EndpointStore>>,
    add_data: web::Json<AddApiGroupRequest>,
) -> impl Responder {
    let email = &add_data.email;
    let mut api_group = add_data.api_group.clone();

    tracing::info!(
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
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Base URL cannot be empty"
        }));
    }

    // Generate ID if not provided
    if api_group.group.id.trim().is_empty() {
        api_group.group.id = generate_id_from_text(&api_group.group.name);
    }

    // Set group_id on all endpoints
    for endpoint in &mut api_group.endpoints {
        // Generate endpoint ID if not provided
        if endpoint.id.trim().is_empty() {
            endpoint.id = generate_id_from_text(&endpoint.text);
        }
        endpoint.group_id = api_group.group.id.clone();
    }

    // Add the API group
    let groups = vec![api_group.clone()];
    match store.replace_user_api_groups(email, groups).await {
        Ok(endpoint_count) => {
            tracing::info!(
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
            tracing::error!(
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

// Handler for updating an API group
async fn update_api_group(
    store: web::Data<Arc<EndpointStore>>,
    update_data: web::Json<UpdateApiGroupRequest>,
) -> impl Responder {
    let email = &update_data.email;
    let group_id = &update_data.group_id;
    let mut api_group = update_data.api_group.clone();

    tracing::info!(
        email = %email,
        group_id = %group_id,
        "Received HTTP update API group request"
    );

    // Validate group data
    if api_group.group.name.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "API group name cannot be empty"
        }));
    }

    if api_group.group.base.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Base URL cannot be empty"
        }));
    }

    // Check if group is a default group
    let is_default_group = match check_is_default_group(&store, group_id).await {
        Ok(is_default) => is_default,
        Err(e) => {
            tracing::error!(
                error = %e,
                group_id = %group_id,
                "Failed to check if group is default"
            );
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to check group status: {}", e)
            }));
        }
    };

    if is_default_group {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Cannot update a default API group. Default groups are read-only."
        }));
    }

    // Ensure group ID is consistent
    api_group.group.id = group_id.clone();

    // Set group_id on all endpoints
    for endpoint in &mut api_group.endpoints {
        // Generate endpoint ID if not provided
        if endpoint.id.trim().is_empty() {
            endpoint.id = generate_id_from_text(&endpoint.text);
        }
        endpoint.group_id = group_id.clone();
    }

    // Update API group by first deleting and then adding
    match store.delete_user_api_group(email, group_id).await {
        Ok(_) => {
            match store.add_user_api_group(email, &api_group).await {
                Ok(endpoint_count) => {
                    tracing::info!(
                        email = %email,
                        group_id = %group_id,
                        endpoint_count = endpoint_count,
                        "Successfully updated API group"
                    );
                    HttpResponse::Ok().json(serde_json::json!({
                        "success": true,
                        "message": "API group successfully updated",
                        "group_id": group_id,
                        "endpoint_count": endpoint_count
                    }))
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        email = %email,
                        group_id = %group_id,
                        "Failed to add updated API group"
                    );
                    HttpResponse::InternalServerError().json(serde_json::json!({
                        "success": false,
                        "message": format!("Failed to update API group: {}", e)
                    }))
                }
            }
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                group_id = %group_id,
                "Failed to delete API group before update"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to update API group: {}", e)
            }))
        }
    }
}

// Helper function to check if a group is a default group
async fn check_is_default_group(store: &web::Data<Arc<EndpointStore>>, group_id: &str) -> Result<bool, String> {
    let conn = store.get_conn().await.map_err(|e| e.to_string())?;
    
    let is_default: bool = conn.query_row(
        "SELECT is_default FROM api_groups WHERE id = ?",
        [group_id],
        |row| row.get(0),
    ).map_err(|e| e.to_string())?;
    
    Ok(is_default)
}

// Handler for deleting an API group
async fn delete_api_group(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>,
) -> impl Responder {
    let (email, group_id) = path_params.into_inner();

    tracing::info!(
        email = %email,
        group_id = %group_id,
        "Received HTTP delete API group request"
    );

    // Check if group is a default group
    let is_default_group = match check_is_default_group(&store, &group_id).await {
        Ok(is_default) => is_default,
        Err(e) => {
            tracing::error!(
                error = %e,
                group_id = %group_id,
                "Failed to check if group is default"
            );
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to check group status: {}", e)
            }));
        }
    };

    if is_default_group {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Cannot delete a default API group. Default groups are read-only."
        }));
    }

    match store.delete_user_api_group(&email, &group_id).await {
        Ok(deleted) => {
            if deleted {
                tracing::info!(
                    email = %email,
                    group_id = %group_id,
                    "Successfully deleted API group"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "API group and its endpoints successfully deleted"
                }))
            } else {
                tracing::warn!(
                    email = %email,
                    group_id = %group_id,
                    "API group not found or not deletable"
                );
                HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "message": "API group not found or is a default group that cannot be deleted"
                }))
            }
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                group_id = %group_id,
                "Failed to delete API group"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to delete API group: {}", e)
            }))
        }
    }
}

// Health check endpoint
async fn health_check() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "service": "api-store-http",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

use std::net::SocketAddr;
use tokio::task;

// Server startup function
pub async fn start_http_server(
    store: Arc<EndpointStore>,
    host: &str,
    port: u16,
) -> std::io::Result<()> {
    let addr = format!("{}:{}", host, port);
    let addr = addr.parse::<SocketAddr>().unwrap();
    let store_clone = store.clone();

    // Run Actix Web in a blocking task to avoid Send issues
    let _ = task::spawn_blocking(move || {
        let sys = actix_web::rt::System::new();
        sys.block_on(async move {
            tracing::info!("Starting HTTP server at {}", addr);

            HttpServer::new(move || {
                // Configure CORS
                let cors = Cors::default()
                    .allow_any_origin()
                    .allow_any_method()
                    .allow_any_header()
                    .max_age(3600);

                App::new()
                    .wrap(Logger::default())
                    .wrap(cors)
                    .app_data(web::Data::new(store_clone.clone()))
                    .service(
                        web::scope("/api")
                            // API groups endpoints
                            .route("/upload", web::post().to(upload_api_config))
                            .route("/groups/{email}", web::get().to(get_api_groups))
                            .route("/group", web::post().to(add_api_group))
                            .route("/group", web::put().to(update_api_group))
                            .route("/user/preferences/{email}", web::get().to(get_user_preferences))
                            .route("/user/preferences", web::post().to(update_user_preferences))
                            .route("/user/preferences/{email}", web::delete().to(reset_user_preferences))
                            .route(
                                "/groups/{email}/{group_id}",
                                web::delete().to(delete_api_group),
                            )
                            .route("/health", web::get().to(health_check)),
                    )
            })
            .bind(addr)?
            .workers(1) // Use fewer workers for testing
            .run()
            .await
        })
    })
    .await
    .expect("Actix system panicked");

    Ok(())
}

// Handler for getting user preferences
async fn get_user_preferences(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP get user preferences request");

    match store.get_user_preferences(&email).await {
        Ok(preferences) => {
            tracing::info!(
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
            tracing::error!(
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

// Handler for updating user preferences
async fn update_user_preferences(
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

    match store.update_user_preferences(email, action, endpoint_id).await {
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

// Handler for resetting user preferences
async fn reset_user_preferences(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP reset user preferences request");

    match store.reset_user_preferences(&email).await {
        Ok(_) => {
            tracing::info!(
                email = %email,
                "Successfully reset user preferences"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "User preferences successfully reset",
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to reset user preferences"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to reset user preferences: {}", e),
            }))
        }
    }
}
