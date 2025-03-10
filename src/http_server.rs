use crate::EndpointsWrapper;
use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use base64::{engine::general_purpose, Engine as _};
use sensei_store::{Endpoint, EndpointStore};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
}

#[derive(Debug, Serialize)]
pub struct EndpointsResponse {
    success: bool,
    endpoints: Vec<Endpoint>,
}

// Handler for uploading endpoints
async fn upload_endpoints(
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
            });
        }
    };

    // Parse based on file extension
    let endpoints =
        if upload_data.file_name.ends_with(".yaml") || upload_data.file_name.ends_with(".yml") {
            // Parse YAML
            match serde_yaml::from_str::<EndpointsWrapper>(&file_content) {
                Ok(wrapper) => wrapper.endpoints,
                Err(e) => {
                    // Try parsing as a list directly
                    match serde_yaml::from_str::<Vec<Endpoint>>(&file_content) {
                        Ok(endpoints) => endpoints,
                        Err(_) => {
                            tracing::error!(error = %e, "Failed to parse YAML content");
                            return HttpResponse::BadRequest().json(UploadResponse {
                                success: false,
                                message: "Invalid YAML format".to_string(),
                                imported_count: 0,
                            });
                        }
                    }
                }
            }
        } else if upload_data.file_name.ends_with(".json") {
            // Parse JSON
            match serde_json::from_str::<EndpointsWrapper>(&file_content) {
                Ok(wrapper) => wrapper.endpoints,
                Err(e) => {
                    // Try parsing as a list directly
                    match serde_json::from_str::<Vec<Endpoint>>(&file_content) {
                        Ok(endpoints) => endpoints,
                        Err(_) => {
                            tracing::error!(error = %e, "Failed to parse JSON content");
                            return HttpResponse::BadRequest().json(UploadResponse {
                                success: false,
                                message: "Invalid JSON format".to_string(),
                                imported_count: 0,
                            });
                        }
                    }
                }
            }
        } else {
            return HttpResponse::BadRequest().json(UploadResponse {
                success: false,
                message: "Unsupported file format. Use YAML or JSON.".to_string(),
                imported_count: 0,
            });
        };

    // Replace user endpoints
    match store
        .replace_user_endpoints(&upload_data.email, endpoints)
        .await
    {
        Ok(count) => {
            tracing::info!(
                email = %upload_data.email,
                imported_count = count,
                "Successfully imported endpoints via HTTP API"
            );
            HttpResponse::Ok().json(UploadResponse {
                success: true,
                message: "Endpoints successfully imported".to_string(),
                imported_count: count as i32,
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %upload_data.email,
                "Failed to import endpoints via HTTP API"
            );
            HttpResponse::InternalServerError().json(UploadResponse {
                success: false,
                message: format!("Failed to import endpoints: {}", e),
                imported_count: 0,
            })
        }
    }
}

// Handler for deleting a specific endpoint
async fn delete_endpoint(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>,
) -> impl Responder {
    let (email, endpoint_id) = path_params.into_inner();

    tracing::info!(
        email = %email,
        endpoint_id = %endpoint_id,
        "Received HTTP delete endpoint request"
    );

    match store.delete_user_endpoint(&email, &endpoint_id).await {
        Ok(deleted) => {
            if deleted {
                tracing::info!(
                    email = %email,
                    endpoint_id = %endpoint_id,
                    "Successfully deleted endpoint"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "Endpoint successfully deleted"
                }))
            } else {
                tracing::warn!(
                    email = %email,
                    endpoint_id = %endpoint_id,
                    "Endpoint not found or not deletable"
                );
                HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "message": "Endpoint not found or is a default endpoint that cannot be deleted"
                }))
            }
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                endpoint_id = %endpoint_id,
                "Failed to delete endpoint"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to delete endpoint: {}", e)
            }))
        }
    }
}

// Request model for adding a single endpoint
#[derive(Debug, Clone, Deserialize)]
pub struct AddEndpointRequest {
    email: String,
    endpoint: Endpoint,
}

// Handler for adding a single endpoint
async fn add_endpoint(
    store: web::Data<Arc<EndpointStore>>,
    add_data: web::Json<AddEndpointRequest>,
) -> impl Responder {
    let email = &add_data.email;
    let endpoint = &add_data.endpoint;

    tracing::info!(
        email = %email,
        endpoint_id = %endpoint.id,
        "Received HTTP add endpoint request"
    );

    // Validate endpoint data
    if endpoint.id.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Endpoint ID cannot be empty"
        }));
    }

    if endpoint.text.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Endpoint text cannot be empty"
        }));
    }

    // Add the endpoint
    match store.add_user_endpoint(email, endpoint.clone()).await {
        Ok(_added) => {
            tracing::info!(
                email = %email,
                endpoint_id = %endpoint.id,
                "Successfully added endpoint"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "Endpoint successfully added",
                "endpoint_id": endpoint.id
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                endpoint_id = %endpoint.id,
                "Failed to add endpoint"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to add endpoint: {}", e)
            }))
        }
    }
}

// Handler for getting endpoints
async fn get_endpoints(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP get endpoints request");

    match store.get_endpoints_by_email(&email) {
        Ok(endpoints) => {
            tracing::info!(
                email = %email,
                endpoint_count = endpoints.len(),
                "Successfully retrieved endpoints via HTTP API"
            );
            HttpResponse::Ok().json(EndpointsResponse {
                success: true,
                endpoints,
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to retrieve endpoints via HTTP API"
            );
            // Return empty list on error
            HttpResponse::Ok().json(EndpointsResponse {
                success: false,
                endpoints: vec![],
            })
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
// In http_server.rs
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
                            .route("/upload", web::post().to(upload_endpoints))
                            .route("/endpoints/{email}", web::get().to(get_endpoints))
                            .route("/endpoint", web::post().to(add_endpoint))
                            .route(
                                "/endpoints/{email}/{endpoint_id}",
                                web::delete().to(delete_endpoint),
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
