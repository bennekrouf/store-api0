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
mod delete_user_endpoint;
pub mod models;
mod replace_user_api_groups;
mod user_preferences;
mod utils;

pub use errors::*;
pub use models::*;
pub use utils::*;

use crate::db_pool::{create_pg_pool, PgConnection, PgPool};

#[derive(Clone)]
pub struct EndpointStore {
    pool: PgPool,
}

impl EndpointStore {
    pub async fn get_all_authorized_domains(&self) -> Result<Vec<String>, StoreError> {
        authorized_domains::get_all_authorized_domains(self).await
    }

    pub async fn initialize_system_domains(&self) -> Result<(), StoreError> {
        authorized_domains::initialize_system_domains(self).await
    }

    pub async fn get_group_base_url(&self, group_id: &str) -> Result<String, StoreError> {
        let client = self.get_conn().await?;

        let row = client
            .query_one("SELECT base FROM api_groups WHERE id = $1", &[&group_id])
            .await
            .map_err(|_| StoreError::Database("Group not found".to_string()))?;

        Ok(row.get(0))
    }

    pub async fn new(database_url: &str) -> Result<Self, StoreError> {
        tracing::info!("Initializing EndpointStore with PostgreSQL");

        let pool = create_pg_pool(database_url)
            .map_err(|e| StoreError::Pool(format!("Failed to create connection pool: {:?}", e)))?;

        let store = Self { pool };

        let client = store.get_conn().await?;

        client
            .batch_execute(include_str!("../../sql/schema.sql"))
            .await
            .map_err(|e| StoreError::Database(format!("Schema execution failed: {}", e)))?;

        store.initialize_system_domains().await?;
        Ok(store)
    }

    pub async fn get_conn(&self) -> Result<PgConnection, StoreError> {
        self.pool.get().await.to_store_error()
    }

    pub async fn get_user_preferences(&self, email: &str) -> Result<UserPreferences, StoreError> {
        user_preferences::get_user_preferences(self, email).await
    }

    pub async fn update_user_preferences(
        &self,
        email: &str,
        action: &str,
        endpoint_id: &str,
    ) -> Result<bool, StoreError> {
        user_preferences::update_user_preferences(self, email, action, endpoint_id).await
    }

    pub async fn reset_user_preferences(&self, email: &str) -> Result<bool, StoreError> {
        user_preferences::reset_user_preferences(self, email).await
    }

    pub async fn get_api_groups_with_preferences(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        let api_groups = self.get_api_groups_by_email(email).await?;

        let filtered_groups = api_groups
            .into_iter()
            .map(|group| ApiGroupWithEndpoints {
                group: group.group,
                endpoints: group.endpoints,
            })
            .filter(|group| !group.endpoints.is_empty())
            .collect();

        Ok(filtered_groups)
    }

    pub async fn get_or_create_user_api_groups(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        get_create_user_api_groups::get_or_create_user_api_groups(self, email).await
    }

    pub async fn get_api_groups_by_email(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        get_api_groups_by_email::get_api_groups_by_email(self, email).await
    }

    pub async fn replace_user_api_groups(
        &self,
        email: &str,
        api_groups: Vec<ApiGroupWithEndpoints>,
    ) -> Result<usize, StoreError> {
        replace_user_api_groups::replace_user_api_groups(self, email, api_groups).await
    }

    pub async fn add_user_api_group(
        &self,
        email: &str,
        api_group: &ApiGroupWithEndpoints,
    ) -> Result<usize, StoreError> {
        add_user_api_group::add_user_api_group(self, email, api_group).await
    }

    pub async fn delete_user_api_group(
        &self,
        email: &str,
        group_id: &str,
    ) -> Result<bool, StoreError> {
        delete_user_api_group::delete_user_api_group(self, email, group_id).await
    }

    pub async fn delete_user_endpoint(
        &self,
        email: &str,
        endpoint_id: &str,
    ) -> Result<bool, StoreError> {
        delete_user_endpoint::delete_user_endpoint(self, email, endpoint_id).await
    }

    pub(crate) async fn force_clean_user_data(&self, email: &str) -> Result<(), StoreError> {
        cleanup::force_clean_user_data(self, email).await
    }

    pub(crate) async fn fallback_clean_user_data(&self, email: &str) -> Result<(), StoreError> {
        cleanup::fallback_clean_user_data(self, email).await
    }

    pub async fn get_api_keys_status(&self, email: &str) -> Result<KeyPreference, StoreError> {
        api_key_management::get_api_keys_status(self, email).await
    }

    pub async fn generate_api_key(
        &self,
        email: &str,
        key_name: &str,
    ) -> Result<(String, String, String), StoreError> {
        api_key_management::generate_api_key(self, email, key_name).await
    }

    pub async fn revoke_api_key(&self, email: &str, key_id: &str) -> Result<bool, StoreError> {
        api_key_management::revoke_api_key(self, email, key_id).await
    }

    pub async fn revoke_all_api_keys(&self, email: &str) -> Result<usize, StoreError> {
        api_key_management::revoke_all_api_keys(self, email).await
    }

    pub async fn validate_api_key(
        &self,
        key: &str,
    ) -> Result<Option<(String, String)>, StoreError> {
        api_key_management::validate_api_key(self, key).await
    }

    pub async fn record_api_key_usage(&self, key_id: &str) -> Result<(), StoreError> {
        api_key_management::record_api_key_usage(self, key_id).await
    }

    pub async fn get_api_key_usage(&self, key_id: &str) -> Result<Option<ApiKeyInfo>, StoreError> {
        api_key_management::get_api_key_usage(self, key_id).await
    }

    pub async fn log_api_usage(&self, request: &LogApiUsageRequest) -> Result<String, StoreError> {
        let client = self.get_conn().await?;
        let log_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        // Convert metadata to JSON string for storage
        let metadata_json = request
            .metadata
            .as_ref()
            .and_then(|m| serde_json::to_string(m).ok());

        client
        .execute(
            "INSERT INTO api_usage_logs (
            id, key_id, email, endpoint_path, method, timestamp,
            response_status, response_time_ms, request_size, response_size,
            ip_address, user_agent, usage_estimated, input_tokens,
            output_tokens, total_tokens, model_used, metadata
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18::jsonb)",
            &[
                &log_id,
                &request.key_id,
                &request.email,
                &request.endpoint_path,
                &request.method,
                &now,
                &request.status_code,
                &request.response_time_ms,
                &request.request_size_bytes,
                &request.response_size_bytes,
                &request.ip_address,
                &request.user_agent,
                &request.usage.as_ref().map(|u| u.estimated),
                &request.usage.as_ref().map(|u| u.input_tokens),
                &request.usage.as_ref().map(|u| u.output_tokens),
                &request.usage.as_ref().map(|u| u.total_tokens),
                &request.usage.as_ref().map(|u| u.model.clone()),
                &metadata_json, // Now a String, which can be cast to jsonb
            ],
        )
        .await
        .to_store_error()?;

        Ok(log_id)
    }

    pub async fn get_api_usage_logs(
        &self,
        key_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<ApiUsageLog>, StoreError> {
        let client = self.get_conn().await?;
        let limit = limit.unwrap_or(50).min(100);

        let rows = client
            .query(
                "SELECT id, key_id, email, endpoint_path, method, timestamp,
            response_status, response_time_ms, request_size, response_size,
            ip_address, user_agent, usage_estimated, input_tokens,
            output_tokens, total_tokens, model_used, metadata
            FROM api_usage_logs 
            WHERE key_id = $1 
            ORDER BY timestamp DESC 
            LIMIT $2",
                &[&key_id, &limit],
            )
            .await
            .to_store_error()?;

        let mut logs = Vec::new();
        for row in rows {
            // Get metadata as string first, then parse to JSON
            let metadata_str: Option<String> = row.get(17);
            let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

            logs.push(ApiUsageLog {
                id: row.get(0),
                key_id: row.get(1),
                email: row.get(2),
                endpoint_path: row.get(3),
                method: row.get(4),
                timestamp: row.get::<_, chrono::DateTime<chrono::Utc>>(5).to_rfc3339(),
                response_status: row.get(6),
                response_time_ms: row.get(7),
                request_size: row.get(8),
                response_size: row.get(9),
                ip_address: row.get(10),
                user_agent: row.get(11),
                usage_estimated: row.get(12),
                input_tokens: row.get(13),
                output_tokens: row.get(14),
                total_tokens: row.get(15),
                model_used: row.get(16),
                metadata,
            });
        }

        Ok(logs)
    }

    pub async fn update_credit_balance(&self, email: &str, amount: i64) -> Result<i64, StoreError> {
        api_key_management::update_credit_balance(self, email, amount).await
    }

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
