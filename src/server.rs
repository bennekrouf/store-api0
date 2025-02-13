use crate::endpoint::endpoint_service_server::EndpointService;
use crate::endpoint::{
    Endpoint as ProtoEndpoint, GetEndpointsRequest, GetEndpointsResponse,
    Parameter as ProtoParameter,
};
use api_store::EndpointStore;
use std::sync::Arc;
use tonic::{Request, Response, Status};

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
    async fn get_default_endpoints(
        &self,
        request: Request<GetEndpointsRequest>,
    ) -> Result<Response<GetEndpointsResponse>, Status> {
        let email = &request.get_ref().email;
        tracing::info!(email = %email, "Received get_default_endpoints request");

        let endpoints = match self.store.get_endpoints_by_email(email) {
            Ok(endpoints) => {
                tracing::debug!(count = endpoints.len(), "Retrieved endpoints from store");
                endpoints
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to get endpoints from store");
                return Err(Status::internal(e.to_string()));
            }
        };

        tracing::info!("Starting endpoint transformation");
        let proto_endpoints: Vec<ProtoEndpoint> = endpoints
            .into_iter()
            .map(|e| {
                let param_count = e.parameters.len();
                tracing::info!(
                    endpoint_id = %e.id,
                    parameter_count = param_count,
                    "Transforming endpoint"
                );
                // tracing::info!("Tranfor toto {}", param_count);
                ProtoEndpoint {
                    id: e.id,
                    text: e.text,
                    description: e.description,
                    parameters: e
                        .parameters
                        .into_iter()
                        .map(|p| ProtoParameter {
                            name: p.name,
                            description: p.description,
                            required: p.required,
                            alternatives: p.alternatives,
                        })
                        .collect(),
                }
            })
            .collect();

        tracing::info!(
            endpoint_count = proto_endpoints.len(),
            "Successfully transformed endpoints"
        );

        let response = GetEndpointsResponse {
            endpoints: proto_endpoints,
        };
        tracing::debug!(response = ?response, "Sending response");

        Ok(Response::new(response))
    }
}
