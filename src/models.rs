use crate::endpoint_store::ApiGroupWithEndpoints;
use serde::{Deserialize, Serialize};

// use crate::endpoint_store::Endpoint;
// Request and Response models for API key validation
#[derive(Debug, Deserialize)]
pub struct ValidateKeyRequest {
    pub api_key: String,
}

// #[derive(Debug, Clone, Deserialize)]
// pub struct ManageEndpointRequest {
//     pub email: String,
//     pub group_id: String,
//     pub endpoint: Endpoint,
// }

#[derive(Debug, Serialize)]
pub struct ValidateKeyResponse {
    pub valid: bool,
    pub email: Option<String>,
    pub key_id: Option<String>,
    pub message: String,
}

// Response model for API key usage
// #[derive(Debug, Serialize)]
// pub struct RecordUsageResponse {
//     pub success: bool,
//     pub message: String,
// }

// Request and Response models
#[derive(Debug, Clone, Deserialize)]
pub struct UploadRequest {
    pub email: String,
    pub file_name: String,
    pub file_content: String, // Base64 encoded
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub success: bool,
    pub message: String,
    pub imported_count: i32,
    pub group_count: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddApiGroupRequest {
    pub email: String,
    pub api_group: ApiGroupWithEndpoints,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateApiGroupRequest {
    pub email: String,
    pub group_id: String,
    pub api_group: ApiGroupWithEndpoints,
}

// Handler for recording API key usage
// #[derive(Debug, Deserialize)]
// pub struct RecordUsageRequest {
//     pub key_id: String,
// }
