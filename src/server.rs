use crate::endpoint::endpoint_service_server::EndpointService;
use crate::endpoint::{
    Endpoint as ProtoEndpoint, GetEndpointsRequest, GetEndpointsResponse,
    Parameter as ProtoParameter,
};
use api_store::EndpointStore;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tokio_stream::Stream;
use std::pin::Pin;

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
            // Get endpoints from store
            // let endpoints = match store.get_endpoints_by_email(&email) {
            //     Ok(endpoints) => {
            //         tracing::debug!(count = endpoints.len(), "Retrieved endpoints from store");
            //         endpoints
            //     }
            //     Err(e) => {
            //         tracing::error!(error = %e, "Failed to get endpoints from store");
            //         Err(Status::internal(e.to_string()))?;
            //         return;
            //     }
            // };

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
    //     let endpoints = match self.store.get_endpoints_by_email(email) {
    //         Ok(endpoints) => {
    //             tracing::debug!(count = endpoints.len(), "Retrieved endpoints from store");
    //             endpoints
    //         }
    //         Err(e) => {
    //             tracing::error!(error = %e, "Failed to get endpoints from store");
    //             return Err(Status::internal(e.to_string()));
    //         }
    //     };
    //
    //     tracing::info!("Starting endpoint transformation");
    //     let proto_endpoints: Vec<ProtoEndpoint> = endpoints
    //         .into_iter()
    //         .map(|e| {
    //             let param_count = e.parameters.len();
    //             tracing::info!(
    //                 endpoint_id = %e.id,
    //                 parameter_count = param_count,
    //                 "Transforming endpoint"
    //             );
    //             // tracing::info!("Tranfor toto {}", param_count);
    //             ProtoEndpoint {
    //                 id: e.id,
    //                 text: e.text,
    //                 description: e.description,
    //                 parameters: e
    //                     .parameters
    //                     .into_iter()
    //                     .map(|p| ProtoParameter {
    //                         name: p.name,
    //                         description: p.description,
    //                         required: p.required,
    //                         alternatives: p.alternatives,
    //                     })
    //                     .collect(),
    //             }
    //         })
    //         .collect();
    //
    //     tracing::info!(
    //         endpoint_count = proto_endpoints.len(),
    //         "Successfully transformed endpoints"
    //     );
    //
    //     let response = GetEndpointsResponse {
    //         endpoints: proto_endpoints,
    //     };
    //     tracing::debug!(response = ?response, "Sending response");
    //
    //     Ok(Response::new(response))
    // }
}
