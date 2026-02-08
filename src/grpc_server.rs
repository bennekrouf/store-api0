use crate::app_log;
use crate::endpoint::endpoint_service_server::EndpointService;
use crate::endpoint::{
    ApiGroup as ProtoApiGroup, ConfirmPaymentRequest, ConfirmPaymentResponse, CreatePaymentIntentRequest,
    CreatePaymentIntentResponse, Endpoint as ProtoEndpoint, GetApiGroupsRequest, GetApiGroupsResponse,
    GetUserPreferencesRequest, GetUserPreferencesResponse, Parameter as ProtoParameter,
    ResetUserPreferencesRequest, ResetUserPreferencesResponse, UpdateUserPreferencesRequest,
    UpdateUserPreferencesResponse, UploadApiGroupsRequest, UploadApiGroupsResponse,
    UserPreferences as ProtoUserPreferences,
};
use crate::formatter::YamlFormatter;
use crate::payment_service::PaymentService;

use crate::endpoint_store::{
    generate_id_from_text, ApiGroup, ApiGroupWithEndpoints, ApiStorage, EndpointStore,
};
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

#[derive(Clone)]
pub struct EndpointServiceImpl {
    store: Arc<EndpointStore>,
    formatter: Arc<YamlFormatter>,
    payment_service: Arc<PaymentService>,
}

impl EndpointServiceImpl {
    pub fn new(
        store: Arc<EndpointStore>,
        formatter_url: &str,
        payment_service: Arc<PaymentService>,
    ) -> Self {
        Self {
            store,
            formatter: Arc::new(YamlFormatter::new(formatter_url)),
            payment_service,
        }
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
        app_log!(info, email = %email, "Received get_api_groups request");

        // Clone necessary data for the stream
        let store = self.store.clone();

        // Create the stream
        let stream = async_stream::try_stream! {
            // Get API groups and endpoints
            let api_groups = match store.get_or_create_user_api_groups(&email).await {
                Ok(groups) => groups,
                Err(e) => {
                    app_log!(error, error = %e, "Failed to get API groups from store");
                    // Yield an empty response instead of returning an error
                    yield GetApiGroupsResponse { api_groups: vec![] };
                    return;
                }
            };

            const BATCH_SIZE: usize = 5; // Process 5 groups at a time
            let mut current_batch = Vec::with_capacity(BATCH_SIZE);

            app_log!(info, "Starting API group transformation and streaming");

            for api_group_with_endpoints in api_groups {
                let group = api_group_with_endpoints.group;
                let endpoints = api_group_with_endpoints.endpoints;

                app_log!(debug,
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
                    app_log!(info,
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
                app_log!(info,
                    batch_size = current_batch.len(),
                    "Sending final batch of API groups"
                );

                yield GetApiGroupsResponse {
                    api_groups: current_batch,
                };
            }

            app_log!(info, "Finished streaming all API groups");
        };

        Ok(Response::new(Box::pin(stream)))
    }

    async fn upload_api_groups(
        &self,
        request: Request<UploadApiGroupsRequest>,
    ) -> Result<Response<UploadApiGroupsResponse>, Status> {
        let req = request.into_inner();
        let email = req.email;
        let file_content = req.file_content.clone();
        let file_name = req.file_name.clone();

        app_log!(info,
            email = %email,
            filename = %req.file_name,
            "Processing API group upload request"
        );

        // Format YAML content if needed
        let file_content = if file_name.ends_with(".yaml") || file_name.ends_with(".yml") {
            match self.formatter.format_yaml(&file_content, &file_name).await {
                Ok(formatted) => formatted,
                Err(e) => {
                    app_log!(warn,
                        error = %e,
                        email = %email,
                        "Failed to format YAML, proceeding with original content"
                    );
                    file_content
                }
            }
        } else {
            file_content
        };

        // Convert to string
        let file_content = match String::from_utf8(file_content) {
            Ok(content) => content,
            Err(e) => {
                app_log!(error, error = %e, "Invalid file content: not UTF-8");
                return Err(Status::invalid_argument(format!(
                    "Invalid file content: {}",
                    e
                )));
            }
        };

        // Detect and parse content based on file extension
        let mut api_storage = if req.file_name.ends_with(".yaml") || req.file_name.ends_with(".yml")
        {
            // Parse YAML content
            match serde_yaml::from_str::<ApiStorage>(&file_content) {
                Ok(storage) => storage,
                Err(e) => {
                    app_log!(error,
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
                    app_log!(error,
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
            app_log!(error,
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
            app_log!(warn,
                email = %email,
                "No API groups found in uploaded file"
            );
            return Err(Status::invalid_argument(
                "No API groups found in uploaded file",
            ));
        }

        // Resolve tenant_id for the user
        use crate::endpoint_store::tenant_management;
        // Logic to get tenant_id. We need "store" available.
        // self.store is Arc<EndpointStore>.
        let tenant_id = match tenant_management::get_default_tenant(&self.store, &email).await {
             Ok(t) => t.id,
             Err(e) => {
                 app_log!(error, error=%e, "Failed to resolve tenant for upload_api_groups");
                 return Err(Status::internal("Failed to resolve tenant organization"));
             }
        };

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
                    tenant_id: tenant_id.clone(),
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

                app_log!(info,
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
                app_log!(error,
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

    // Add these methods to impl EndpointService for EndpointServiceImpl in src/grpc_server.rs
    async fn get_user_preferences(
        &self,
        request: Request<GetUserPreferencesRequest>,
    ) -> Result<Response<GetUserPreferencesResponse>, Status> {
        let email = request.into_inner().email;
        app_log!(info, email = %email, "Received get_user_preferences gRPC request");

        match self.store.get_user_preferences(&email).await {
            Ok(prefs) => {
                app_log!(info,
                    email = %email,
                    hidden_count = prefs.hidden_defaults.len(),
                    "Successfully retrieved user preferences"
                );

                // Convert to proto format
                let proto_prefs = ProtoUserPreferences {
                    email: prefs.email,
                    hidden_defaults: prefs.hidden_defaults,
                };

                Ok(Response::new(GetUserPreferencesResponse {
                    success: true,
                    message: "User preferences successfully retrieved".to_string(),
                    preferences: Some(proto_prefs),
                }))
            }
            Err(e) => {
                app_log!(error,
                    error = %e,
                    email = %email,
                    "Failed to retrieve user preferences"
                );

                Ok(Response::new(GetUserPreferencesResponse {
                    success: false,
                    message: format!("Failed to retrieve user preferences: {}", e),
                    preferences: None,
                }))
            }
        }
    }

    async fn update_user_preferences(
        &self,
        request: Request<UpdateUserPreferencesRequest>,
    ) -> Result<Response<UpdateUserPreferencesResponse>, Status> {
        let req = request.into_inner();
        let email = req.email;
        let action = req.action;
        let endpoint_id = req.endpoint_id;

        app_log!(info,
            email = %email,
            action = %action,
            endpoint_id = %endpoint_id,
            "Received update_user_preferences gRPC request"
        );

        match self
            .store
            .update_user_preferences(&email, &action, &endpoint_id)
            .await
        {
            Ok(_) => {
                app_log!(info,
                    email = %email,
                    action = %action,
                    endpoint_id = %endpoint_id,
                    "Successfully updated user preferences"
                );

                Ok(Response::new(UpdateUserPreferencesResponse {
                    success: true,
                    message: "User preferences successfully updated".to_string(),
                }))
            }
            Err(e) => {
                app_log!(error,
                    error = %e,
                    email = %email,
                    "Failed to update user preferences"
                );

                Ok(Response::new(UpdateUserPreferencesResponse {
                    success: false,
                    message: format!("Failed to update user preferences: {}", e),
                }))
            }
        }
    }

    async fn reset_user_preferences(
        &self,
        request: Request<ResetUserPreferencesRequest>,
    ) -> Result<Response<ResetUserPreferencesResponse>, Status> {
        let email = request.into_inner().email;

        app_log!(info,
            email = %email,
            "Received reset_user_preferences gRPC request"
        );

        match self.store.reset_user_preferences(&email).await {
            Ok(_) => {
                app_log!(info,
                    email = %email,
                    "Successfully reset user preferences"
                );

                Ok(Response::new(ResetUserPreferencesResponse {
                    success: true,
                    message: "User preferences successfully reset".to_string(),
                }))
            }
            Err(e) => {
                app_log!(error,
                    error = %e,
                    email = %email,
                    "Failed to reset user preferences"
                );

                Ok(Response::new(ResetUserPreferencesResponse {
                    success: false,
                    message: format!("Failed to reset user preferences: {}", e),
                }))
            }
        }
    }

    async fn get_reference_data(
        &self,
        request: Request<crate::endpoint::GetReferenceDataRequest>,
    ) -> Result<Response<crate::endpoint::GetReferenceDataResponse>, Status> {
        let email = request.into_inner().email;
        app_log!(info, email = %email, "Received get_reference_data gRPC request");

        match self.store.get_reference_data(&email).await {
            Ok(data) => {
                app_log!(info,
                    email = %email,
                    count = data.len(),
                    "Successfully retrieved reference data"
                );

                let proto_data = data
                    .into_iter()
                    .map(|d| crate::endpoint::ReferenceData {
                        id: d.id,
                        email: d.email,
                        name: d.name,
                        data: d.data.to_string(),
                        created_at: d.created_at.to_rfc3339(),
                    })
                    .collect();

                Ok(Response::new(crate::endpoint::GetReferenceDataResponse {
                    reference_data: proto_data,
                }))
            }
            Err(e) => {
                app_log!(error,
                    error = %e,
                    email = %email,
                    "Failed to retrieve reference data"
                );
                Err(Status::internal(format!("Failed to retrieve reference data: {}", e)))
            }
        }
    }

    async fn create_payment_intent(
        &self,
        request: Request<CreatePaymentIntentRequest>,
    ) -> Result<Response<CreatePaymentIntentResponse>, Status> {
        let req = request.into_inner();
        let email = req.email;
        let amount = req.amount;
        let currency = req.currency;

        app_log!(info, email = %email, amount = amount, currency = %currency, "Received create_payment_intent request");

        match self
            .payment_service
            .create_payment_intent(amount, &currency, &email)
            .await
        {
            Ok(intent) => {
                app_log!(info, email = %email, intent_id = %intent.id.as_str(), "Successfully created payment intent");
                Ok(Response::new(CreatePaymentIntentResponse {
                    success: true,
                    client_secret: intent.client_secret.unwrap_or_default(),
                    payment_intent_id: intent.id.to_string(),
                    message: "Payment intent created successfully".to_string(),
                }))
            }
            Err(e) => {
                app_log!(error, error = %e, email = %email, "Failed to create payment intent");
                Ok(Response::new(CreatePaymentIntentResponse {
                    success: false,
                    client_secret: "".to_string(),
                    payment_intent_id: "".to_string(),
                    message: format!("Failed to create payment intent: {}", e),
                }))
            }
        }
    }

    async fn confirm_payment(
        &self,
        request: Request<ConfirmPaymentRequest>,
    ) -> Result<Response<ConfirmPaymentResponse>, Status> {
        let req = request.into_inner();
        let email = req.email;
        let payment_intent_id = req.payment_intent_id;
        let amount = req.amount;

        app_log!(info, email = %email, payment_intent_id = %payment_intent_id, "Received confirm_payment request");

        let intent = match self.payment_service.confirm_payment(&payment_intent_id).await {
            Ok(intent) => intent,
            Err(e) => {
                app_log!(error, error = %e, email = %email, "Failed to verify payment intent");
                return Ok(Response::new(ConfirmPaymentResponse {
                    success: false,
                    payment_verified: false,
                    new_credit_balance: 0,
                    message: format!("Failed to verify payment: {}", e),
                }));
            }
        };

        if intent.status != stripe::PaymentIntentStatus::Succeeded {
            app_log!(warn, email = %email, status = ?intent.status, "Payment intent not succeeded");
            return Ok(Response::new(ConfirmPaymentResponse {
                success: false,
                payment_verified: false,
                new_credit_balance: 0,
                message: format!("Payment not succeeded. Status: {:?}", intent.status),
            }));
        }

        // Add credits to user balance (1 cent = 100 credits)
        let credits_to_add = amount * 100;

        match self.store.update_credit_balance(&email, credits_to_add).await {
            Ok(new_balance) => {
                app_log!(info, email = %email, amount = amount, credits = credits_to_add, new_balance = new_balance, "Successfully added credits");
                Ok(Response::new(ConfirmPaymentResponse {
                    success: true,
                    payment_verified: true,
                    new_credit_balance: new_balance,
                    message: "Payment confirmed and credits added".to_string(),
                }))
            }
            Err(e) => {
                app_log!(error, error = %e, email = %email, "Failed to update credit balance");
                Ok(Response::new(ConfirmPaymentResponse {
                    success: false,
                    payment_verified: true,
                    new_credit_balance: 0,
                    message: format!("Payment verified but failed to update balance: {}", e),
                }))
            }
        }
    }
}
