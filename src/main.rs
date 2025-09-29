// src/main.rs
mod add_api_group;
mod config;
mod db_pool;
mod delete_api_group;
mod endpoint_store;
mod formatter;
mod generate_api_key;
mod get_api_groups;
mod get_api_key_usage;
mod get_api_keys_status;
mod get_api_usage_logs;
mod get_authorized_domains;
mod get_credit_balance_handler;
mod get_user_preferences;
mod grpc_server;
mod health_check;
mod http_server;
mod log_api_usage;
mod manage_endpoint;
mod models;
mod reset_user_preferences;
mod revoke_all_api_keys_handler;
mod revoke_api_key_handler;
mod update_api_group;
mod update_credit_balance_handler;
mod update_user_preferences;
mod upload_api_config;
mod validate_api_key;
use config::Config;
use formatter::YamlFormatter;

use crate::endpoint_store::{
    generate_id_from_text, ApiGroup, ApiGroupWithEndpoints, ApiStorage, Endpoint, EndpointStore,
};
use crate::grpc_server::EndpointServiceImpl;
use endpoint::endpoint_service_server::EndpointServiceServer;
use serde::Deserialize;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tonic::transport::Server;
use tonic_reflection::server::Builder;
use tonic_web::GrpcWebLayer;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};

pub mod endpoint {
    tonic::include_proto!("endpoint");
}

fn resolve_config_path() -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    // Try environment variable first
    if let Ok(config_path) = env::var("CONFIG_PATH") {
        let path = PathBuf::from(&config_path);
        if path.exists() {
            tracing::info!("Found config at CONFIG_PATH: {}", config_path);
            return Ok(path);
        } else {
            return Err(
                format!("CONFIG_PATH specified but file not found: {}", config_path).into(),
            );
        }
    }

    // Get the directory where the executable is located
    let exe_path =
        env::current_exe().map_err(|e| format!("Failed to get executable path: {}", e))?;

    let exe_dir = exe_path
        .parent()
        .ok_or("Failed to get executable directory")?;

    // Try different possible locations relative to exe
    let possible_paths = vec![
        exe_dir.join("config.yaml"), // Same dir as exe
        exe_dir.parent().unwrap_or(exe_dir).join("config.yaml"), // Parent dir
        exe_dir.join("..").join("config.yaml"), // Explicit parent
        PathBuf::from("/opt/api0/store/config.yaml"), // Absolute fallback
        PathBuf::from("./config.yaml"), // Current working dir
    ];

    for path in possible_paths {
        let canonical_path = path.canonicalize().unwrap_or(path.clone());
        tracing::debug!("Checking config path: {:?}", canonical_path);
        if canonical_path.exists() {
            tracing::info!("Found config at: {:?}", canonical_path);
            return Ok(canonical_path);
        }
    }

    Err("config.yaml not found in any expected location. Please set CONFIG_PATH environment variable or place config.yaml in the executable directory.".into())
}

fn resolve_db_path() -> Result<PathBuf, Box<dyn Error + Send + Sync>> {
    let db_path_str = env::var("DB_PATH").unwrap_or_else(|_| {
        // Default to a directory relative to the executable
        let exe_path = env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
        let exe_dir = exe_path.parent().unwrap_or(Path::new("."));
        exe_dir.join("db").to_string_lossy().to_string()
    });

    let db_dir = PathBuf::from(&db_path_str);

    // Ensure the directory exists
    if let Err(e) = std::fs::create_dir_all(&db_dir) {
        return Err(format!(
            "Failed to create database directory {}: {}",
            db_dir.display(),
            e
        )
        .into());
    }

    // Test write permissions
    let test_file = db_dir.join(".write_test");
    if let Err(e) = std::fs::write(&test_file, "test") {
        return Err(format!(
            "Cannot write to database directory {}: {}",
            db_dir.display(),
            e
        )
        .into());
    }
    let _ = std::fs::remove_file(test_file); // Clean up test file

    let db_file = db_dir.join("endpoints.db");
    tracing::info!("Using database file: {}", db_file.display());

    Ok(db_file)
}

fn resolve_endpoints_config_path() -> Option<PathBuf> {
    // Try environment variable first
    if let Ok(config_path) = env::var("ENDPOINTS_CONFIG_PATH") {
        let path = PathBuf::from(&config_path);
        if path.exists() {
            tracing::info!(
                "Found endpoints config at ENDPOINTS_CONFIG_PATH: {}",
                config_path
            );
            return Some(path);
        } else {
            tracing::warn!(
                "ENDPOINTS_CONFIG_PATH specified but file not found: {}",
                config_path
            );
            return None;
        }
    }

    // Get the directory where the executable is located
    let exe_path = env::current_exe().ok()?;
    let exe_dir = exe_path.parent()?;

    // Try different possible locations
    let possible_paths = vec![
        exe_dir.join("endpoints.yaml"),
        exe_dir.parent()?.join("endpoints.yaml"),
        PathBuf::from("/opt/api0/endpoints.yaml"),
        PathBuf::from("./endpoints.yaml"),
    ];

    for path in possible_paths {
        if path.exists() {
            tracing::info!("Found endpoints config at: {:?}", path);
            return Some(path);
        }
    }

    tracing::info!("No endpoints.yaml found, will use empty default configuration");
    None
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initialize logging first
    Registry::default()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new("INFO")))
        .init();

    tracing::info!("Starting API Store service");
    tracing::info!("Executable path: {:?}", env::current_exe());
    tracing::info!("Working directory: {:?}", env::current_dir());

    // Resolve and load configuration - FAIL if not found
    let config_path = resolve_config_path().map_err(|e| {
        tracing::error!("Configuration error: {}", e);
        e
    })?;

    tracing::info!("Loading configuration from: {:?}", config_path);

    let config = Config::from_file(&config_path).map_err(|e| {
        tracing::error!("Failed to parse configuration file: {}", e);
        format!("Configuration parse error: {}", e)
    })?;

    tracing::info!("Successfully loaded configuration");

    let formatter_url = config.formatter_url();
    tracing::info!("Using YAML formatter at: {}", formatter_url);

    let formatter = Arc::new(YamlFormatter::new(&formatter_url));

    // Resolve database path
    let db_file = resolve_db_path()?;
    let store = EndpointStore::new(&db_file).await.map_err(|e| {
        tracing::error!("Failed to initialize database: {}", e);
        e
    })?;

    // Load default API groups from YAML if available
    if let Some(endpoints_config_path) = resolve_endpoints_config_path() {
        tracing::info!(
            "Loading endpoints configuration from: {:?}",
            endpoints_config_path
        );

        let config_content = match std::fs::read_to_string(&endpoints_config_path) {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!(
                    "Failed to load endpoints config: {}. Continuing without default endpoints.",
                    e
                );
                String::new()
            }
        };

        // Only process endpoints config if we successfully loaded it
        if !config_content.is_empty() {
            // Try parsing as the new ApiStorage format
            let _api_storage: ApiStorage = match serde_yaml::from_str(&config_content) {
                Ok(storage) => storage,
                Err(e) => {
                    // If parsing as new format fails, try to convert from old format
                    tracing::warn!(
                        "Failed to parse config as new format: {}. Trying legacy format...",
                        e
                    );

                    // Try to parse as old format (endpoints list or wrapper)
                    let endpoints = match serde_yaml::from_str::<serde_json::Value>(&config_content)
                    {
                        Ok(value) => {
                            if let Some(_endpoints_array) = value.get("endpoints") {
                                // Parse as EndpointsWrapper
                                #[derive(Deserialize)]
                                struct EndpointsWrapper {
                                    endpoints: Vec<Endpoint>,
                                }
                                let wrapper: EndpointsWrapper =
                                    serde_yaml::from_str(&config_content)?;
                                wrapper.endpoints
                            } else {
                                // Parse as direct Vec<Endpoint>
                                let endpoints: Vec<Endpoint> =
                                    serde_yaml::from_str(&config_content)?;
                                endpoints
                            }
                        }
                        Err(e) => {
                            return Err(
                                format!("Failed to parse endpoints config file: {}", e).into()
                            );
                        }
                    };

                    // Group endpoints by base URL to create API groups
                    use std::collections::HashMap;
                    let mut groups_map: HashMap<String, Vec<Endpoint>> = HashMap::new();

                    for endpoint in endpoints {
                        groups_map
                            .entry(endpoint.base.clone())
                            .or_insert_with(Vec::new)
                            .push(endpoint);
                    }

                    // Convert to ApiStorage format
                    let mut api_groups = Vec::new();

                    for (base, endpoints) in groups_map {
                        // Generate a name from the base URL
                        let domain = base
                            .split("://")
                            .nth(1)
                            .unwrap_or(&base)
                            .split('/')
                            .next()
                            .unwrap_or("API");

                        let name = if domain.contains("localhost") {
                            "Local API".to_string()
                        } else if domain.contains("example.com") {
                            "Example API".to_string()
                        } else {
                            format!("{} API", domain)
                        };

                        // Create group
                        let group_id = generate_id_from_text(&name);
                        let mut processed_endpoints = Vec::new();

                        // Process endpoints to set group_id
                        for mut endpoint in endpoints {
                            endpoint.group_id = group_id.clone();
                            processed_endpoints.push(endpoint);
                        }

                        // Create API group
                        let group = ApiGroup {
                            id: group_id,
                            name,
                            description: format!("APIs for {}", domain),
                            base,
                        };

                        api_groups.push(ApiGroupWithEndpoints {
                            group,
                            endpoints: processed_endpoints,
                        });
                    }

                    ApiStorage { api_groups }
                }
            };

            tracing::info!("Successfully processed endpoints configuration");
        }
    } else {
        tracing::info!("No endpoints configuration file found, starting with empty configuration");
    }

    // Wrap the store in an Arc for sharing between servers
    let store_arc = Arc::new(store);

    // Get HTTP configuration
    let http_host = config.http_host().to_string();
    let http_port = config.http_port();

    // Clone for the HTTP server
    let http_formatter = Arc::clone(&formatter);
    let http_store = Arc::clone(&store_arc);

    // Start the HTTP server as a separate task
    let http_handle = tokio::spawn(async move {
        tracing::info!("Starting HTTP server on {}:{}", http_host, http_port);

        if let Err(e) =
            http_server::start_http_server(http_store, http_formatter, &http_host, http_port).await
        {
            tracing::error!("HTTP server error: {}", e);
        }
    });

    // Configure gRPC server
    let service = EndpointServiceImpl::new(store_arc, &formatter_url);
    let grpc_addr = config.grpc_address().parse()?;

    // Load the file descriptor for reflection
    let descriptor_set = include_bytes!(concat!(env!("OUT_DIR"), "/endpoint_descriptor.bin"));
    let reflection_service = Builder::configure()
        .register_encoded_file_descriptor_set(descriptor_set)
        .build_v1()?;

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_headers(Any)
        .allow_methods(Any)
        .expose_headers(Any);

    // Start the gRPC server as a separate task
    tracing::info!("Starting gRPC server on {}", grpc_addr);
    let grpc_handle = tokio::spawn(async move {
        if let Err(e) = Server::builder()
            .accept_http1(true)
            .max_concurrent_streams(128)
            .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
            .tcp_nodelay(true)
            .layer(cors)
            .layer(GrpcWebLayer::new())
            .add_service(EndpointServiceServer::new(service))
            .add_service(reflection_service)
            .serve(grpc_addr)
            .await
        {
            tracing::error!("gRPC server error: {}", e);
        }
    });

    // Create a shutdown signal
    let shutdown = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to listen for CTRL+C");
        tracing::info!("Received CTRL+C, shutting down all servers");
    };

    tracing::info!("All services started successfully");

    // Wait for either server to finish or for the shutdown signal
    tokio::select! {
        _ = http_handle => tracing::info!("HTTP server has shut down"),
        _ = grpc_handle => tracing::info!("gRPC server has shut down"),
        _ = shutdown => tracing::info!("Shutdown signal received"),
    }

    tracing::info!("Application shutting down");
    Ok(())
}
