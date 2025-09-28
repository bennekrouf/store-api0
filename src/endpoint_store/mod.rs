// src/endpoint_store/mod.rs
mod add_user_api_group;
mod api_key_management;
mod authorized_domains;
mod cleanup;
pub mod db_helpers;
mod delete_user_api_group;
mod errors;
mod get_api_groups_by_email;
mod get_create_user_api_groups;
mod manage_single_endpoint;
use crate::endpoint_store::db_helpers::ResultExt;
pub mod models;
mod replace_user_api_groups;
mod user_preferences;
mod utils;

// Re-export everything needed for the public API
pub use errors::*;
pub use models::*;
pub use utils::*;

use crate::db_pool::{create_db_pool, MobcSQLiteConnection, MobcSQLitePool};
use std::path::Path;
use std::time::Duration;

/// The main EndpointStore struct that provides access to all functionality
#[derive(Clone)]
pub struct EndpointStore {
    pool: MobcSQLitePool,
}

impl EndpointStore {
    /// Get all authorized domains for CORS
    pub async fn get_all_authorized_domains(&self) -> Result<Vec<String>, StoreError> {
        authorized_domains::get_all_authorized_domains(self).await
    }

    /// Initialize system domains
    pub async fn initialize_system_domains(&self) -> Result<(), StoreError> {
        authorized_domains::initialize_system_domains(self).await
    }

    /// Gets the base URL for a group
    /// Gets the base URL for a group
    pub async fn get_group_base_url(&self, group_id: &str) -> Result<String, StoreError> {
        let conn = self.get_conn().await?;

        let base_url: String = conn
            .query_row(
                "SELECT base FROM api_groups WHERE id = ?",
                [group_id],
                |row| row.get(0),
            )
            .map_err(|_| StoreError::Database("Group not found".to_string()))?;

        Ok(base_url)
    }

    /// Creates a new EndpointStore instance with the given database path
    pub async fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, StoreError> {
        tracing::info!(
            "Initializing EndpointStore with path: {:?}",
            db_path.as_ref()
        );

        let pool = create_db_pool(db_path, 10, Some(Duration::from_secs(60)))
            .map_err(|e| StoreError::Pool(format!("Failed to create connection pool: {:?}", e)))?;

        let store = Self { pool };

        // Get a connection and execute schema statements individually
        let conn = store.get_conn().await?;

        // Split the schema into individual statements and execute them
        let schema = include_str!("../../sql/schema.sql");
        let statements: Vec<&str> = schema
            .split(';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && !s.starts_with("--"))
            .collect();

        // Execute PRAGMA statements first
        for statement in &statements {
            if statement.starts_with("PRAGMA") {
                conn.execute(statement, [])
                    .map_err(|e| StoreError::Database(format!("PRAGMA execution failed: {}", e)))?;
            }
        }

        // Execute CREATE TABLE statements
        for statement in &statements {
            if statement.starts_with("CREATE TABLE") {
                conn.execute(statement, []).map_err(|_e| {
                    StoreError::Database(format!("CREATE TABLE failed on: {}", statement))
                })?;
            }
        }

        // Execute CREATE INDEX statements last
        for statement in &statements {
            if statement.starts_with("CREATE INDEX") {
                conn.execute(statement, []).map_err(|_e| {
                    StoreError::Database(format!("CREATE INDEX failed on: {}", statement))
                })?;
            }
        }

        store.initialize_system_domains().await?;
        Ok(store)
    }

    /// Get a connection from the pool
    pub async fn get_conn(&self) -> Result<MobcSQLiteConnection, StoreError> {
        self.pool
            .get()
            .await
            .map_err(|e| StoreError::Pool(e.to_string()))
    }

    /// Gets user preferences by email
    pub async fn get_user_preferences(&self, email: &str) -> Result<UserPreferences, StoreError> {
        user_preferences::get_user_preferences(self, email).await
    }

    /// Updates user preferences
    pub async fn update_user_preferences(
        &self,
        email: &str,
        action: &str,
        endpoint_id: &str,
    ) -> Result<bool, StoreError> {
        user_preferences::update_user_preferences(self, email, action, endpoint_id).await
    }

    /// Resets user preferences
    pub async fn reset_user_preferences(&self, email: &str) -> Result<bool, StoreError> {
        user_preferences::reset_user_preferences(self, email).await
    }

    /// Gets API groups with user preferences applied
    pub async fn get_api_groups_with_preferences(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        // Get API groups
        let api_groups = self.get_api_groups_by_email(email).await?;

        // Apply preferences filter
        let filtered_groups = api_groups
            .into_iter()
            .map(|group| {
                let filtered_endpoints = group.endpoints.into_iter().collect();

                ApiGroupWithEndpoints {
                    group: group.group,
                    endpoints: filtered_endpoints,
                }
            })
            .filter(|group| !group.endpoints.is_empty()) // Remove empty groups
            .collect();

        Ok(filtered_groups)
    }

    /// Gets or creates API groups for a user
    pub async fn get_or_create_user_api_groups(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        get_create_user_api_groups::get_or_create_user_api_groups(self, email).await
    }

    /// Gets all API groups and endpoints for a user
    pub async fn get_api_groups_by_email(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        get_api_groups_by_email::get_api_groups_by_email(self, email).await
    }

    /// Replaces all API groups and endpoints for a user
    pub async fn replace_user_api_groups(
        &self,
        email: &str,
        api_groups: Vec<ApiGroupWithEndpoints>,
    ) -> Result<usize, StoreError> {
        replace_user_api_groups::replace_user_api_groups(self, email, api_groups).await
    }

    /// Adds a single API group for a user
    pub async fn add_user_api_group(
        &self,
        email: &str,
        api_group: &ApiGroupWithEndpoints,
    ) -> Result<usize, StoreError> {
        add_user_api_group::add_user_api_group(self, email, api_group).await
    }

    /// Deletes an API group and all its endpoints for a user
    pub async fn delete_user_api_group(
        &self,
        email: &str,
        group_id: &str,
    ) -> Result<bool, StoreError> {
        delete_user_api_group::delete_user_api_group(self, email, group_id).await
    }

    /// Cleans up user data in a forced way (internal use)
    pub(crate) async fn force_clean_user_data(&self, email: &str) -> Result<(), StoreError> {
        cleanup::force_clean_user_data(self, email).await
    }

    /// Cleans up user data in a more conservative way (fallback)
    pub(crate) async fn fallback_clean_user_data(&self, email: &str) -> Result<(), StoreError> {
        cleanup::fallback_clean_user_data(self, email).await
    }

    /// Gets the API keys status for a user
    pub async fn get_api_keys_status(&self, email: &str) -> Result<KeyPreference, StoreError> {
        api_key_management::get_api_keys_status(self, email).await
    }

    /// Generates a new API key for a user
    pub async fn generate_api_key(
        &self,
        email: &str,
        key_name: &str,
    ) -> Result<(String, String, String), StoreError> {
        api_key_management::generate_api_key(self, email, key_name).await
    }

    /// Revokes a specific API key for a user
    pub async fn revoke_api_key(&self, email: &str, key_id: &str) -> Result<bool, StoreError> {
        api_key_management::revoke_api_key(self, email, key_id).await
    }

    /// Revokes all API keys for a user
    pub async fn revoke_all_api_keys(&self, email: &str) -> Result<usize, StoreError> {
        api_key_management::revoke_all_api_keys(self, email).await
    }

    /// Validates an API key and returns key_id and email if valid
    pub async fn validate_api_key(
        &self,
        key: &str,
    ) -> Result<Option<(String, String)>, StoreError> {
        api_key_management::validate_api_key(self, key).await
    }

    /// Records usage of an API key
    pub async fn record_api_key_usage(&self, key_id: &str) -> Result<(), StoreError> {
        api_key_management::record_api_key_usage(self, key_id).await
    }

    /// Gets usage statistics for a specific API key
    pub async fn get_api_key_usage(&self, key_id: &str) -> Result<Option<ApiKeyInfo>, StoreError> {
        api_key_management::get_api_key_usage(self, key_id).await
    }

    /// Log detailed API usage with token information
    pub async fn log_api_usage(&self, request: &LogApiUsageRequest) -> Result<String, StoreError> {
        let conn = self.get_conn().await?;
        let log_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO api_usage_logs (
            id, key_id, email, endpoint_path, method, timestamp,
            response_status, response_time_ms, request_size, response_size,
            ip_address, user_agent, usage_estimated, input_tokens,
            output_tokens, total_tokens, model_used
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            rusqlite::params![
                log_id,
                request.key_id,
                request.email,
                request.endpoint_path,
                request.method,
                now,
                request.status_code,
                request.response_time_ms,
                request.request_size_bytes,
                request.response_size_bytes,
                request.ip_address,
                request.user_agent,
                request.usage.as_ref().map(|u| u.estimated),
                request.usage.as_ref().map(|u| u.input_tokens),
                request.usage.as_ref().map(|u| u.output_tokens),
                request.usage.as_ref().map(|u| u.total_tokens),
                request.usage.as_ref().map(|u| u.model.clone()),
            ],
        )
        .to_store_error()?;

        Ok(log_id)
    }

    /// Get API usage logs for a key
    pub async fn get_api_usage_logs(
        &self,
        key_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<ApiUsageLog>, StoreError> {
        let conn = self.get_conn().await?;
        let limit = limit.unwrap_or(50).min(100);

        let mut stmt = conn
            .prepare(
                "SELECT id, key_id, email, endpoint_path, method, timestamp,
                response_status, response_time_ms, request_size, response_size,
                ip_address, user_agent, usage_estimated, input_tokens,
                output_tokens, total_tokens, model_used
         FROM api_usage_logs 
         WHERE key_id = ? 
         ORDER BY timestamp DESC 
         LIMIT ?",
            )
            .to_store_error()?;

        let logs_iter = stmt
            .query_map([key_id, &limit.to_string()], |row| {
                Ok(ApiUsageLog {
                    id: row.get(0)?,
                    key_id: row.get(1)?,
                    email: row.get(2)?,
                    endpoint_path: row.get(3)?,
                    method: row.get(4)?,
                    timestamp: row.get(5)?,
                    response_status: row.get(6)?,
                    response_time_ms: row.get(7)?,
                    request_size: row.get(8)?,
                    response_size: row.get(9)?,
                    ip_address: row.get(10)?,
                    user_agent: row.get(11)?,
                    usage_estimated: row.get(12)?,
                    input_tokens: row.get(13)?,
                    output_tokens: row.get(14)?,
                    total_tokens: row.get(15)?,
                    model_used: row.get(16)?,
                })
            })
            .to_store_error()?;

        let mut logs = Vec::new();
        for log_result in logs_iter {
            logs.push(log_result.to_store_error()?);
        }

        Ok(logs)
    }

    /// Updates credit balance for a user
    pub async fn update_credit_balance(&self, email: &str, amount: i64) -> Result<i64, StoreError> {
        api_key_management::update_credit_balance(self, email, amount).await
    }

    /// Gets credit balance for a user
    pub async fn get_credit_balance(&self, email: &str) -> Result<i64, StoreError> {
        api_key_management::get_credit_balance(self, email).await
    }

    pub async fn manage_single_endpoint(
        &self,
        email: &str,
        endpoint: &Endpoint,
    ) -> Result<String, StoreError> {
        manage_single_endpoint::manage_single_endpoint(self, email, endpoint).await
    }
}
