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
        let endpoints = self
            .store
            .get_endpoints_by_email(email)
            .map_err(|e| Status::internal(e.to_string()))?;

        let proto_endpoints: Vec<ProtoEndpoint> = endpoints
            .into_iter()
            .map(|e| ProtoEndpoint {
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
            })
            .collect();

        Ok(Response::new(GetEndpointsResponse {
            endpoints: proto_endpoints,
        }))
    }
}
