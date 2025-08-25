// src/endpoint_store/mod.rs
mod add_user_api_group;
mod api_key_management;
mod cleanup;
mod db_helpers;
mod delete_user_api_group;
mod errors;
mod get_api_groups_by_email;
mod get_create_user_api_groups;
mod get_default_api_groups;
mod get_endpoints_by_group_id;
mod init;
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
use get_endpoints_by_group_id::get_endpoints_by_group_id;

/// The main EndpointStore struct that provides access to all functionality
#[derive(Clone)]
pub struct EndpointStore {
    pool: MobcSQLitePool,
}

impl EndpointStore {
    /// Creates a new EndpointStore instance with the given database path
    pub async fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, StoreError> {
        tracing::info!(
            "Initializing EndpointStore with path: {:?}",
            db_path.as_ref()
        );

        let pool = create_db_pool(
            db_path,
            10,                            // max pool size
            Some(Duration::from_secs(60)), // max idle timeout
        )
        .map_err(|e| StoreError::Pool(format!("Failed to create connection pool: {:?}", e)))?;

        let store = Self { pool };
        // Get a connection to initialize the schema
        let conn = store.get_conn().await?;

        // Create tables with the schema
        conn.execute_batch(include_str!("../../sql/schema.sql"))
            .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(store)
    }

/// Gets endpoints for a specific group ID
    pub(crate) async fn get_endpoints_by_group_id(
        &self,
        group_id: &str,
    ) -> Result<Vec<Endpoint>, StoreError> {
        get_endpoints_by_group_id(self, group_id).await
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
        // Get user preferences
        let preferences = self.get_user_preferences(email).await?;

        // Get API groups
        let api_groups = self.get_api_groups_by_email(email).await?;

        // Apply preferences filter
        let filtered_groups = api_groups
            .into_iter()
            .map(|group| {
                let filtered_endpoints = group
                    .endpoints
                    .into_iter()
                    .filter(|endpoint| {
                        // Keep the endpoint if it's NOT (default AND hidden)
                        let is_default = endpoint.is_default.unwrap_or(false);
                        !(is_default && preferences.hidden_defaults.contains(&endpoint.id))
                    })
                    .collect();

                ApiGroupWithEndpoints {
                    group: group.group,
                    endpoints: filtered_endpoints,
                }
            })
            .filter(|group| !group.endpoints.is_empty()) // Remove empty groups
            .collect();

        Ok(filtered_groups)
    }

    /// Initializes the database with default API groups if it's empty
    pub async fn initialize_if_empty(
        &mut self,
        default_api_groups: &[ApiGroupWithEndpoints],
    ) -> Result<(), StoreError> {
        init::initialize_if_empty(self, default_api_groups).await
    }

    /// Gets or creates API groups for a user
    pub async fn get_or_create_user_api_groups(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        get_create_user_api_groups::get_or_create_user_api_groups(self, email).await
    }

    /// Gets the default API groups from the database
    pub(crate) async fn get_default_api_groups(
        &self,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        get_default_api_groups::get_default_api_groups(self).await
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

    /// Updates credit balance for a user
    pub async fn update_credit_balance(&self, email: &str, amount: i64) -> Result<i64, StoreError> {
        api_key_management::update_credit_balance(self, email, amount).await
    }

    /// Gets credit balance for a user
    pub async fn get_credit_balance(&self, email: &str) -> Result<i64, StoreError> {
        api_key_management::get_credit_balance(self, email).await
    }
}
