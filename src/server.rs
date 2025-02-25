use crate::endpoint::endpoint_service_server::EndpointService;
use crate::endpoint::{
    Endpoint as ProtoEndpoint, GetEndpointsRequest, GetEndpointsResponse,
    Parameter as ProtoParameter,
};
use crate::Endpoint;
use api_store::EndpointStore;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tokio_stream::Stream;
use std::pin::Pin;

use crate::endpoint::UploadEndpointsRequest;
use crate::endpoint::UploadEndpointsResponse;
use crate::EndpointsWrapper;

#[derive(Debug, Clone)]
pub struct EndpointServiceImpl {
    store: Arc<EndpointStore>,
}

impl EndpointServiceImpl {
    pub fn new(store: EndpointStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }
}

#[tonic::async_trait]
impl EndpointService for EndpointServiceImpl {

    type GetDefaultEndpointsStream = Pin<Box<dyn Stream<Item = Result<GetEndpointsResponse, Status>> + Send + 'static>>;

    async fn get_default_endpoints(
        &self,
        request: Request<GetEndpointsRequest>,
    ) -> Result<Response<Self::GetDefaultEndpointsStream>, Status> {
        let email = request.into_inner().email;
        tracing::info!(email = %email, "Received get_default_endpoints request");

        // Clone necessary data for the stream
        let store = self.store.clone();

        // Create the stream
        let stream = async_stream::try_stream! {
            let endpoints = store.get_endpoints_by_email(&email).map_err(|e| {
                tracing::error!(error = %e, "Failed to get endpoints from store");
                Status::internal(e.to_string())
            })?;

            const BATCH_SIZE: usize = 10;
            let mut current_batch = Vec::with_capacity(BATCH_SIZE);

            tracing::info!("Starting endpoint transformation and streaming");

            for endpoint in endpoints {
                let param_count = endpoint.parameters.len();
                tracing::info!(
                    endpoint_id = %endpoint.id,
                    parameter_count = param_count,
                    "Transforming endpoint"
                );

                let proto_endpoint = ProtoEndpoint {
                    id: endpoint.id,
                    text: endpoint.text,
                    description: endpoint.description,
                    parameters: endpoint
                        .parameters
                        .into_iter()
                        .map(|p| ProtoParameter {
                            name: p.name,
                            description: p.description,
                            required: p.required,
                            alternatives: p.alternatives,
                        })
                        .collect(),
                };

                current_batch.push(proto_endpoint);

                // When batch is full, yield it
                if current_batch.len() >= BATCH_SIZE {
                    tracing::info!(
                        batch_size = current_batch.len(),
                        "Sending batch of endpoints"
                    );

                    yield GetEndpointsResponse {
                        endpoints: std::mem::take(&mut current_batch),
                    };
                }
            }

            // Send any remaining endpoints
            if !current_batch.is_empty() {
                tracing::info!(
                    batch_size = current_batch.len(),
                    "Sending final batch of endpoints"
                );

                yield GetEndpointsResponse {
                    endpoints: current_batch,
                };
            }

            tracing::info!("Finished streaming all endpoints");
        };

        Ok(Response::new(Box::pin(stream)))
    }

    async fn upload_endpoints(
        &self,
        request: Request<UploadEndpointsRequest>,
    ) -> Result<Response<UploadEndpointsResponse>, Status> {
        let req = request.into_inner();
        let email = req.email;
        let file_content = String::from_utf8(req.file_content.clone())
            .map_err(|e| Status::invalid_argument(format!("Invalid file content: {}", e)))?;

        tracing::info!(
            email = %email,
            filename = %req.file_name,
            "Processing endpoint upload request"
        );

        // Detect and parse content based on file extension
        let endpoints = if req.file_name.ends_with(".yaml") || req.file_name.ends_with(".yml") {
            // Parse YAML content
            match serde_yaml::from_str::<EndpointsWrapper>(&file_content) {
                Ok(wrapper) => wrapper.endpoints,
                Err(e) => {
                    // Try parsing as a list of endpoints directly
                    match serde_yaml::from_str::<Vec<Endpoint>>(&file_content) {
                        Ok(endpoints) => endpoints,
                        Err(_) => {
                            tracing::error!(
                                error = %e,
                                email = %email,
                                "Failed to parse YAML content"
                            );
                            return Err(Status::invalid_argument(
                                "Invalid YAML format. Expected either a list of endpoints or an object with 'endpoints' field."
                            ));
                        }
                    }
                }
            }
        } else if req.file_name.ends_with(".json") {
            // Parse JSON content
            match serde_json::from_str::<EndpointsWrapper>(&file_content) {
                Ok(wrapper) => wrapper.endpoints,
                Err(e) => {
                    // Try parsing as a list of endpoints directly
                    match serde_json::from_str::<Vec<Endpoint>>(&file_content) {
                        Ok(endpoints) => endpoints,
                        Err(_) => {
                            tracing::error!(
                                error = %e,
                                email = %email,
                                "Failed to parse JSON content"
                            );
                            return Err(Status::invalid_argument(
                                "Invalid JSON format. Expected either a list of endpoints or an object with 'endpoints' field."
                            ));
                        }
                    }
                }
            }
        } else {
            tracing::error!(
                email = %email,
                filename = %req.file_name,
                "Unsupported file format"
            );
            return Err(Status::invalid_argument(
                "Unsupported file format. Please upload a YAML (.yaml/.yml) or JSON (.json) file."
            ));
        };

        // Validate endpoints
        if endpoints.is_empty() {
            tracing::warn!(
                email = %email,
                "No endpoints found in uploaded file"
            );
            return Err(Status::invalid_argument("No endpoints found in uploaded file"));
        }

        // Validate endpoint structure
        for (index, endpoint) in endpoints.iter().enumerate() {
            if endpoint.id.trim().is_empty() {
                return Err(Status::invalid_argument(
                    format!("Endpoint at index {} has an empty ID", index)
                ));
            }
            if endpoint.text.trim().is_empty() {
                return Err(Status::invalid_argument(
                    format!("Endpoint '{}' has an empty text", endpoint.id)
                ));
            }
        }

        // Replace user endpoints
        match self.store.replace_user_endpoints(&email, endpoints).await {
            Ok(count) => {
                tracing::info!(
                    email = %email,
                    imported_count = count,
                    "Successfully imported endpoints"
                );
                Ok(Response::new(UploadEndpointsResponse {
                    success: true,
                    message: "Endpoints successfully imported".to_string(),
                    imported_count: count as i32,
                }))
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    "Failed to import endpoints"
                );
                Err(Status::internal(format!("Failed to import endpoints: {}", e)))
            }
        }
    }
}
