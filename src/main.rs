mod server;
use crate::server::EndpointServiceImpl;
use endpoint::endpoint_service_server::EndpointServiceServer;
use sensei_store::{Endpoint, EndpointStore};
use serde::{Deserialize, Serialize};
use std::error::Error;
use tonic::transport::Server;
use tonic_reflection::server::Builder;
use tonic_web::GrpcWebLayer;
use tower_http::cors::{Any, CorsLayer};
// use grpc_logger::setup_logging;
// use grpc_logger::load_config;

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
    // Initialize logging configuration
    // let config = load_config("config.yaml")?;
    // setup_logging(&config).await?;

    // Test log generation
    // loop {
    //     tracing::info!("Test semantic log message");
    //     tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    // }

    Registry::default()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new("INFO")))
        .init();
    let mut store = EndpointStore::new("db/endpoints.db")?;

    // Load default endpoints from YAML and initialize DB
    let config_content = std::fs::read_to_string("endpoints.yaml")?;
    let wrapper: EndpointsWrapper = serde_yaml::from_str(&config_content)?;
    let default_endpoints = wrapper.endpoints;
    store.initialize_if_empty(&default_endpoints)?;
    let service = EndpointServiceImpl::new(store);
    let addr = "0.0.0.0:50055".parse()?;

    // Load the file descriptor for reflection
    let descriptor_set = include_bytes!(concat!(env!("OUT_DIR"), "/endpoint_descriptor.bin"));
    let reflection_service = Builder::configure()
        .register_encoded_file_descriptor_set(descriptor_set)
        .build_v1()?;

    // Create CORS layer
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_headers(Any)
        .allow_methods(Any)
        .expose_headers(Any);

    tracing::info!("Starting api-store gRPC server on {}", addr);

    Server::builder()
        .accept_http1(true)
        .max_concurrent_streams(128)
        .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
        .tcp_nodelay(true)
        .layer(cors)
        .layer(GrpcWebLayer::new())
        .add_service(EndpointServiceServer::new(service))
        .add_service(reflection_service)
        .serve_with_shutdown(addr, async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Shutting down api-store server...");
        })
        .await?;

    Ok(())
}
