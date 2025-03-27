use serde::{Deserialize, Serialize};
use crate::endpoint_store::utils::generate_uuid;

// Helper function to provide default verb value
fn default_verb() -> String {
    "GET".to_string()
}

fn default_base_url() -> String {
    "http://localhost:3000".to_string()
}

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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Parameter {
    pub name: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub alternatives: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Endpoint {
    #[serde(default = "generate_uuid")]
    pub id: String,
    pub text: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default = "default_verb")]
    #[serde(alias = "method")] // Allow 'method' as an alternative name
    pub verb: String,
    #[serde(default = "default_base_url")]
    pub base: String,
    #[serde(default = "String::new")]
    pub path: String,
    #[serde(default = "String::new")]
    pub group_id: String,
    #[serde(default)]
    pub is_default: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiGroup {
    #[serde(default = "generate_uuid")]
    pub id: String,
    pub name: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default = "default_base_url")]
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
