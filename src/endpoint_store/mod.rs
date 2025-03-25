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

use std::path::Path;
use r2d2::Pool;
use r2d2_duckdb::DuckDBConnectionManager;
/// The main EndpointStore struct that provides access to all functionality
#[derive(Clone)]
pub struct EndpointStore {
    pool: Pool<DuckDBConnectionManager>,
}

impl EndpointStore {
    /// Creates a new EndpointStore instance with the given database path
    pub fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, StoreError> {
        tracing::info!(
            "Initializing EndpointStore with path: {:?}",
            db_path.as_ref()
        );

        let manager = DuckDBConnectionManager::file(db_path.as_ref());        
        let pool = Pool::builder()
            .max_size(10)
            .build(manager)
            .map_err(|e| StoreError::Pool(e.to_string()))?;


        // Initialize schema if needed
        let conn = pool.get()
            .map_err(|e| StoreError::Pool(e.to_string()))?;

        // Create tables with the schema
        conn.execute_batch(include_str!("../../sql/schema.sql"));
        Ok(Self { pool })
    }

    /// Get a connection from the pool
    pub fn get_conn(&self) -> Result<r2d2::PooledConnection<DuckDBConnectionManager>, StoreError> {
        self.pool.get().map_err(|e| StoreError::Pool(e.to_string()))
    }

    /// Initializes the database with default API groups if it's empty
    pub fn initialize_if_empty(
        &mut self,
        default_api_groups: &[ApiGroupWithEndpoints],
    ) -> Result<(), StoreError> {
        init::initialize_if_empty(self, default_api_groups)
    }

    /// Gets or creates API groups for a user
    pub async fn get_or_create_user_api_groups(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        get_create_user_api_groups::get_or_create_user_api_groups(self, email).await
    }

    /// Gets the default API groups from the database
    pub(crate) fn get_default_api_groups(
        &self,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        get_default_api_groups::get_default_api_groups(self)
    }

    /// Gets the endpoints for a specific group
    pub(crate) fn get_endpoints_by_group_id(
        &self,
        group_id: &str,
        // conn: &Connection,
    ) -> Result<Vec<Endpoint>, StoreError> {
        get_endpoints_by_group_id::get_endpoints_by_group_id(self, group_id)
    }

    /// Gets all API groups and endpoints for a user
    pub fn get_api_groups_by_email(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        get_api_groups_by_email::get_api_groups_by_email(self, email)
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
    pub(crate) fn force_clean_user_data(&self, email: &str) -> Result<(), StoreError> {
        cleanup::force_clean_user_data(self, email)
    }

    /// Cleans up user data in a more conservative way (fallback)
    pub(crate) fn fallback_clean_user_data(
        &self,
        email: &str,
        // conn: &mut Connection,
    ) -> Result<(), StoreError> {
        cleanup::fallback_clean_user_data(self, email)
    }
}
