use crate::endpoint_store::utils::generate_uuid;
use serde::{Deserialize, Serialize};

// Helper function to provide default verb value
fn default_verb() -> String {
    "GET".to_string()
}

// fn default_base_url() -> String {
//     "".to_string()
// }

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserPreferences {
    pub email: String,
    pub hidden_defaults: Vec<String>, // List of hidden default endpoint IDs
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdatePreferenceRequest {
    pub email: String,
    pub action: String, // "hide_default" or "show_default"
    pub endpoint_id: String,
}

use serde::Deserializer;

// Helper function for flexible boolean parsing
fn deserialize_flexible_bool<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FlexibleBool {
        Bool(bool),
        String(String),
    }

    match FlexibleBool::deserialize(deserializer)? {
        FlexibleBool::Bool(b) => Ok(b.to_string()),
        FlexibleBool::String(s) => {
            // Validate string is a valid boolean representation
            match s.to_lowercase().as_str() {
                "true" | "false" => Ok(s.to_lowercase()),
                _ => Err(Error::custom(
                    "Invalid boolean string, must be 'true' or 'false'",
                )),
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Parameter {
    pub name: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(
        default = "default_false_string",
        deserialize_with = "deserialize_flexible_bool"
    )]
    pub required: String,
    #[serde(default)]
    pub alternatives: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Endpoint {
    #[serde(default = "String::new")] // Allow empty, will be auto-generated
    pub id: String,
    pub text: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default = "default_verb")]
    #[serde(alias = "method")]
    pub verb: String,
    #[serde(default = "String::new")] // Allow empty, will inherit from group
    pub base: String,
    #[serde(default = "String::new")]
    pub path: String,
    #[serde(default = "String::new")] // Allow empty, will be set by parent group
    pub group_id: String,
}

fn default_false_string() -> String {
    "false".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiGroup {
    #[serde(default = "generate_uuid")]
    pub id: String,
    pub name: String,
    #[serde(default = "String::new")]
    pub description: String,
    pub base: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiGroupWithEndpoints {
    #[serde(flatten)]
    pub group: ApiGroup,
    pub endpoints: Vec<Endpoint>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiStorage {
    pub api_groups: Vec<ApiGroupWithEndpoints>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiKeyInfo {
    pub id: String,
    pub key_prefix: String,
    pub key_name: String,
    pub generated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
    pub usage_count: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyPreference {
    pub has_keys: bool,
    pub active_key_count: usize,
    pub keys: Vec<ApiKeyInfo>,
    pub balance: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GenerateKeyRequest {
    pub email: String,
    pub key_name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GenerateKeyResponse {
    pub success: bool,
    pub message: String,
    pub key: Option<String>,
    pub key_prefix: Option<String>,
    pub key_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RevokeKeyRequest {
    pub email: String,
    pub key_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RevokeKeyResponse {
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyStatusResponse {
    pub success: bool,
    pub key_preference: Option<KeyPreference>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdateCreditRequest {
    pub email: String,
    pub amount: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CreditBalanceResponse {
    pub success: bool,
    pub message: String,
    pub balance: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogApiUsageRequest {
    pub key_id: String,
    pub email: String,
    pub endpoint_path: String,
    pub method: String,
    pub response_status: Option<i32>,
    pub response_time_ms: Option<i64>,
    pub request_size: Option<i64>,
    pub response_size: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogApiUsageResponse {
    pub success: bool,
    pub message: String,
    pub log_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiUsageLog {
    pub id: String,
    pub key_id: String,
    pub email: String,
    pub endpoint_path: String,
    pub method: String,
    pub timestamp: String,
    pub response_status: Option<i32>,
    pub response_time_ms: Option<i64>,
    pub request_size: Option<i64>,
    pub response_size: Option<i64>,
}
