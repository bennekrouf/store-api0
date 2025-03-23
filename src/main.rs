mod http_server;
mod server;

use crate::server::EndpointServiceImpl;
use endpoint::endpoint_service_server::EndpointServiceServer;
use sensei_store::{ApiGroup, ApiGroupWithEndpoints, ApiStorage, Endpoint, EndpointStore};
use serde::Deserialize;
use std::error::Error;
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initialize logging
    Registry::default()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new("DEBUG")))
        .init();

    // Create and initialize the endpoint store
    let mut store = EndpointStore::new("../db/endpoints.db")?;

    // Load default API groups from YAML and initialize DB
    let config_content = std::fs::read_to_string("endpoints.yaml")?;

    // Try parsing as the new ApiStorage format
    let api_storage: ApiStorage = match serde_yaml::from_str(&config_content) {
        Ok(storage) => storage,
        Err(e) => {
            // If parsing as new format fails, try to convert from old format
            tracing::warn!(
                "Failed to parse config as new format: {}. Trying legacy format...",
                e
            );

            // Try to parse as old format (endpoints list or wrapper)
            let endpoints = match serde_yaml::from_str::<serde_json::Value>(&config_content) {
                Ok(value) => {
                    if let Some(_endpoints_array) = value.get("endpoints") {
                        // Parse as EndpointsWrapper
                        #[derive(Deserialize)]
                        struct EndpointsWrapper {
                            endpoints: Vec<Endpoint>,
                        }
                        let wrapper: EndpointsWrapper = serde_yaml::from_str(&config_content)?;
                        wrapper.endpoints
                    } else {
                        // Parse as direct Vec<Endpoint>
                        let endpoints: Vec<Endpoint> = serde_yaml::from_str(&config_content)?;
                        endpoints
                    }
                }
                Err(e) => {
                    return Err(format!("Failed to parse config file: {}", e).into());
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
                let group_id = sensei_store::generate_id_from_text(&name);
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

    // Initialize the store with the default API groups
    store.initialize_if_empty(&api_storage.api_groups)?;

    // Wrap the store in an Arc for sharing between servers
    let store_arc = Arc::new(store);

    // Clone for the HTTP server
    let http_store = Arc::clone(&store_arc);

    // Try different ports for HTTP server
    let http_ports = [9090]; // , 8080, 9090, 3333];
    let mut http_port = http_ports[0];

    // Find an available port
    for &port in &http_ports {
        match tokio::net::TcpListener::bind(format!("127.0.0.1:{}", port)).await {
            Ok(_) => {
                http_port = port;
                tracing::info!("Found available port for HTTP server: {}", port);
                break;
            }
            Err(e) => {
                tracing::warn!("Port {} is not available: {}", port, e);
            }
        }
    }

    // Start the HTTP server as a separate task
    let http_handle = tokio::spawn(async move {
        tracing::info!("Starting HTTP server on port {}", http_port);

        if let Err(e) = http_server::start_http_server(http_store, "127.0.0.1", http_port).await {
            tracing::error!("HTTP server error: {}", e);
        }
    });

    // Configure gRPC server
    let service = EndpointServiceImpl::new(store_arc);
    let addr = "0.0.0.0:50055".parse()?;

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
    tracing::info!("Starting gRPC server on {}", addr);
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
            .serve(addr)
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

    // Wait for either server to finish or for the shutdown signal
    tokio::select! {
        _ = http_handle => tracing::info!("HTTP server has shut down"),
        _ = grpc_handle => tracing::info!("gRPC server has shut down"),
        _ = shutdown => tracing::info!("Shutdown signal received"),
    }

    tracing::info!("Application shutting down");
    Ok(())
}
