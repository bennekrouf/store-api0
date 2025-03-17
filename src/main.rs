mod http_server;
mod server; // Add this line to include our new module

use crate::server::EndpointServiceImpl;
use endpoint::endpoint_service_server::EndpointServiceServer;
use sensei_store::{Endpoint, EndpointStore};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Serialize, Deserialize)]
struct EndpointsWrapper {
    endpoints: Vec<Endpoint>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initialize logging
    Registry::default()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new("INFO")))
        .init();

    // Create and initialize the endpoint store
    let mut store = EndpointStore::new("../db/endpoints.db")?;

    // Load default endpoints from YAML and initialize DB
    let config_content = std::fs::read_to_string("endpoints.yaml")?;
    let wrapper: EndpointsWrapper = serde_yaml::from_str(&config_content)?;
    let default_endpoints = wrapper.endpoints;
    store.initialize_if_empty(&default_endpoints)?;

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
