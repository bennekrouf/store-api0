// src/endpoint_store/mod.rs
pub mod models;
mod errors;
mod utils;
mod init;
mod get_default_api_groups;
mod get_endpoints_by_group_id;
mod get_api_groups_by_email;
mod get_create_user_api_groups;
mod replace_user_api_groups;
mod add_user_api_group;
mod delete_user_api_group;
mod cleanup;
mod db_helpers;

// Re-export everything needed for the public API
pub use models::*;
pub use errors::*;
pub use utils::*;

use crate::db_pool::{create_db_pool, MobcDuckDBConnection, MobcDuckDBPool};
use std::path::Path;
use std::time::Duration;

/// The main EndpointStore struct that provides access to all functionality
#[derive(Clone)]
pub struct EndpointStore {
    pool: MobcDuckDBPool,
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
            10, // max pool size
            Some(Duration::from_secs(60)) // max idle timeout
        ).map_err(|e| StoreError::Pool(format!("Failed to create connection pool: {:?}", e)))?;

        let store = Self { pool };
        // Get a connection to initialize the schema
        let conn = store.get_conn().await?;
        
        // Create tables with the schema
        conn.execute_batch(include_str!("../../sql/schema.sql"))
            .map_err(|e| StoreError::Database(e.to_string()))?;

        Ok(store)
    }

    /// Get a connection from the pool
    pub async fn get_conn(&self) -> Result<MobcDuckDBConnection, StoreError> {
        self.pool.get().await.map_err(|e| StoreError::Pool(e.to_string()))
    }

    /// Static helper to get a connection from a pool
    // async fn get_conn_from_pool(pool: &MobcDuckDBPool) -> Result<MobcDuckDBConnection, StoreError> {
    //     pool.get().await.map_err(|e| StoreError::Pool(e.to_string()))
    // }

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

    /// Gets the endpoints for a specific group
    pub(crate) async fn get_endpoints_by_group_id(
        &self,
        group_id: &str,
    ) -> Result<Vec<Endpoint>, StoreError> {
        get_endpoints_by_group_id::get_endpoints_by_group_id(self, group_id).await
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
    pub(crate) async fn fallback_clean_user_data(
        &self,
        email: &str,
    ) -> Result<(), StoreError> {
        cleanup::fallback_clean_user_data(self, email).await
    }
}
