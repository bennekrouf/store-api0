use crate::endpoint::endpoint_service_server::EndpointService;
use crate::endpoint::{
    ApiGroup as ProtoApiGroup, Endpoint as ProtoEndpoint, GetApiGroupsRequest,
    GetApiGroupsResponse, Parameter as ProtoParameter, UploadApiGroupsRequest,
    UploadApiGroupsResponse,
};
use crate::endpoint_store::{ApiGroup, ApiGroupWithEndpoints, ApiStorage, EndpointStore, generate_id_from_text};
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};
#[derive(Clone)]
pub struct EndpointServiceImpl {
    store: Arc<EndpointStore>,
}

impl EndpointServiceImpl {
    pub fn new(store: Arc<EndpointStore>) -> Self {
        Self { store }
    }
}

#[tonic::async_trait]
impl EndpointService for EndpointServiceImpl {
    type GetApiGroupsStream =
        Pin<Box<dyn Stream<Item = Result<GetApiGroupsResponse, Status>> + Send + 'static>>;

    async fn get_api_groups(
        &self,
        request: Request<GetApiGroupsRequest>,
    ) -> Result<Response<Self::GetApiGroupsStream>, Status> {
        let email = request.into_inner().email;
        tracing::info!(email = %email, "Received get_api_groups request");

        // Clone necessary data for the stream
        let store = self.store.clone();

        // Create the stream
        let stream = async_stream::try_stream! {
            // Get API groups and endpoints
            let api_groups = match store.get_or_create_user_api_groups(&email).await {
                Ok(groups) => groups,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to get API groups from store");
                    // Yield an empty response instead of returning an error
                    yield GetApiGroupsResponse { api_groups: vec![] };
                    return;
                }
            };

            const BATCH_SIZE: usize = 5; // Process 5 groups at a time
            let mut current_batch = Vec::with_capacity(BATCH_SIZE);

            tracing::info!("Starting API group transformation and streaming");

            for api_group_with_endpoints in api_groups {
                let group = api_group_with_endpoints.group;
                let endpoints = api_group_with_endpoints.endpoints;

                tracing::debug!(
                    group_id = %group.id,
                    group_name = %group.name,
                    endpoint_count = endpoints.len(),
                    "Transforming API group"
                );

                // Transform endpoints to proto format
                let proto_endpoints: Vec<ProtoEndpoint> = endpoints
                    .into_iter()
                    .map(|e| ProtoEndpoint {
                        id: e.id,
                        text: e.text,
                        description: e.description,
                        verb: e.verb,
                        base: e.base,
                        path: e.path,
                        group_id: e.group_id,
                        parameters: e.parameters
                            .into_iter()
                            .map(|p| ProtoParameter {
                                name: p.name,
                                description: p.description,
                                required: p.required,
                                alternatives: p.alternatives,
                            })
                            .collect(),
                    })
                    .collect();

                // Create the proto API group
                let proto_group = ProtoApiGroup {
                    id: group.id,
                    name: group.name,
                    description: group.description,
                    base: group.base,
                    endpoints: proto_endpoints,
                };

                current_batch.push(proto_group);

                // When batch is full, yield it
                if current_batch.len() >= BATCH_SIZE {
                    tracing::info!(
                        batch_size = current_batch.len(),
                        "Sending batch of API groups"
                    );

                    yield GetApiGroupsResponse {
                        api_groups: std::mem::take(&mut current_batch),
                    };
                }
            }

            // Send any remaining API groups
            if !current_batch.is_empty() {
                tracing::info!(
                    batch_size = current_batch.len(),
                    "Sending final batch of API groups"
                );

                yield GetApiGroupsResponse {
                    api_groups: current_batch,
                };
            }

            tracing::info!("Finished streaming all API groups");
        };

        Ok(Response::new(Box::pin(stream)))
    }

    async fn upload_api_groups(
        &self,
        request: Request<UploadApiGroupsRequest>,
    ) -> Result<Response<UploadApiGroupsResponse>, Status> {
        let req = request.into_inner();
        let email = req.email;
        let file_content = String::from_utf8(req.file_content.clone())
            .map_err(|e| Status::invalid_argument(format!("Invalid file content: {}", e)))?;

        tracing::info!(
            email = %email,
            filename = %req.file_name,
            "Processing API group upload request"
        );

        // Detect and parse content based on file extension
        let mut api_storage = if req.file_name.ends_with(".yaml") || req.file_name.ends_with(".yml")
        {
            // Parse YAML content
            match serde_yaml::from_str::<ApiStorage>(&file_content) {
                Ok(storage) => storage,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        email = %email,
                        "Failed to parse YAML content"
                    );
                    return Err(Status::invalid_argument(format!(
                        "Invalid YAML format: {}",
                        e
                    )));
                }
            }
        } else if req.file_name.ends_with(".json") {
            // Parse JSON content
            match serde_json::from_str::<ApiStorage>(&file_content) {
                Ok(storage) => storage,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        email = %email,
                        "Failed to parse JSON content"
                    );
                    return Err(Status::invalid_argument(format!(
                        "Invalid JSON format: {}",
                        e
                    )));
                }
            }
        } else {
            tracing::error!(
                email = %email,
                filename = %req.file_name,
                "Unsupported file format"
            );
            return Err(Status::invalid_argument(
                "Unsupported file format. Please upload a YAML (.yaml/.yml) or JSON (.json) file.",
            ));
        };

        // Validate API groups
        if api_storage.api_groups.is_empty() {
            tracing::warn!(
                email = %email,
                "No API groups found in uploaded file"
            );
            return Err(Status::invalid_argument(
                "No API groups found in uploaded file",
            ));
        }

        // Process and enhance each group and endpoint
        let mut processed_groups = Vec::new();

        for group in &mut api_storage.api_groups {
            // Generate ID for group if not provided
            let group_id = if group.group.id.is_empty() {
                generate_id_from_text(&group.group.name)
            } else {
                group.group.id.clone()
            };

            // Process endpoints
            let mut processed_endpoints = Vec::new();
            for endpoint in &mut group.endpoints {
                // Generate ID for endpoint if not provided
                if endpoint.id.is_empty() {
                    endpoint.id = generate_id_from_text(&endpoint.text);
                }

                // Set group_id reference
                endpoint.group_id = group_id.clone();

                processed_endpoints.push(endpoint.clone());
            }

            // Create processed group
            let processed_group = ApiGroupWithEndpoints {
                group: ApiGroup {
                    id: group_id,
                    name: group.group.name.clone(),
                    description: group.group.description.clone(),
                    base: group.group.base.clone(),
                },
                endpoints: processed_endpoints,
            };

            processed_groups.push(processed_group);
        }

        let group_count = api_storage.api_groups.len();

        // Replace user API groups
        match self
            .store
            .replace_user_api_groups(&email, processed_groups)
            .await
        {
            Ok(endpoint_count) => {
                //let group_count = api_storage.api_groups.len();

                tracing::info!(
                    email = %email,
                    group_count = group_count,
                    endpoint_count = endpoint_count,
                    "Successfully imported API groups and endpoints"
                );

                Ok(Response::new(UploadApiGroupsResponse {
                    success: true,
                    message: "API groups successfully imported".to_string(),
                    imported_count: endpoint_count as i32,
                    group_count: group_count as i32,
                }))
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    "Failed to import API groups"
                );

                Err(Status::internal(format!(
                    "Failed to import API groups: {}",
                    e
                )))
            }
        }
    }
}
