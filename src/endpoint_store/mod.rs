mod add_user_api_group;
pub mod api_key_management;
pub mod mcp_tools_management;
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
pub mod reference_data;
mod replace_user_api_groups;
mod user_preferences;
mod utils;
pub mod tenant_management;
pub mod downstream_auth_management;
use crate::app_log;
pub use errors::*;
pub use models::*;
pub use utils::*;

use crate::infra::db::{create_pg_pool, PgConnection, PgPool};

#[derive(Clone)]
pub struct EndpointStore {
    pool: PgPool,
}

impl EndpointStore {
    /// Health check for database connectivity
    pub async fn health_check(&self) -> Result<bool, StoreError> {
        let client = self.get_conn().await?;

        // Simple query to test database connectivity
        let _row = client
            .query_one("SELECT 1 as health_check", &[])
            .await
            .to_store_error()?;

        Ok(true)
    }

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
        app_log!(info, "Initializing EndpointStore with PostgreSQL");

        let pool = create_pg_pool(database_url)
            .map_err(|e| StoreError::Pool(format!("Failed to create connection pool: {:?}", e)))?;

        let store = Self { pool };

        let client = store.get_conn().await?;

        if let Err(e) = client
            .batch_execute(include_str!("../../sql/schema.sql"))
            .await
        {
            let error_str = e.to_string();
            // Don't fail on notices about existing relations
            if error_str.contains("already exists") || error_str.contains("NOTICE") {
                app_log!(info, "Schema execution completed with notices: {}", e);
            } else {
                return Err(StoreError::Database(format!(
                    "Schema execution failed: {}",
                    e
                )));
            }
        } else {
            app_log!(info, "Schema executed successfully");
        }

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

    #[allow(dead_code)]
    pub async fn generate_api_key(
        &self,
        email: &str,
        key_name: &str,
    ) -> Result<(String, String, String), StoreError> {
        api_key_management::generate_api_key(self, email, key_name).await
    }

    #[allow(dead_code)]
    pub async fn generate_api_key_with_provider(
        &self,
        email: &str,
        key_name: &str,
        provider_tenant_id: Option<&str>,
    ) -> Result<(String, String, String), StoreError> {
        api_key_management::generate_api_key_with_provider(self, email, key_name, provider_tenant_id).await
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
        expected_tenant_id: Option<&str>,
    ) -> Result<Option<(String, String, String, Option<String>)>, StoreError> {
        api_key_management::validate_api_key(self, key, expected_tenant_id).await
    }

    pub async fn record_api_key_usage(&self, key_id: &str) -> Result<(), StoreError> {
        api_key_management::record_api_key_usage(self, key_id).await
    }

    pub async fn get_api_key_usage(&self, key_id: &str, tenant_id: &str) -> Result<Option<ApiKeyInfo>, StoreError> {
        api_key_management::get_api_key_usage(self, key_id, tenant_id).await
    }

    pub async fn log_api_usage(&self, request: &LogApiUsageRequest) -> Result<String, StoreError> {
        let client = self.get_conn().await?;
        let log_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        // Resolve tenant_id:
        // 1. Explicitly provided in request
        // 2. Provider's tenant (if this is a consumer key)
        // 3. Key owner's tenant
        // 4. Default tenant for user email
        let mut tenant_id: Option<String> = request.tenant_id.clone();

        if tenant_id.is_none() {
            let key_row = client.query_opt(
                "SELECT provider_tenant_id, tenant_id FROM api_keys WHERE id = $1", 
                &[&request.key_id]
            ).await.to_store_error()?;
            
            if let Some(row) = key_row {
                // If this is a consumer key (provider_tenant_id set), the activity
                // belongs to the PROVIDER's tenant (e.g. Cvenom).
                tenant_id = row.get::<_, Option<String>>(0).or_else(|| row.get::<_, Option<String>>(1));
            }
        }
        
        if tenant_id.is_none() {
             let user_tenant_row = client.query_opt(
                "SELECT default_tenant_id FROM user_preferences WHERE email = $1", 
                &[&request.email]
            ).await.to_store_error()?;
            tenant_id = user_tenant_row.map(|r| r.get(0));
        }
        
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
            output_tokens, total_tokens, model_used, metadata, tenant_id, consumer_id
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18::jsonb, $19, $20)",
            &[
                &log_id as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.key_id as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.email as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.endpoint_path as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.method as &(dyn tokio_postgres::types::ToSql + Sync),
                &now as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.status_code as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.response_time_ms as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.request_size_bytes as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.response_size_bytes as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.ip_address as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.user_agent as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.usage.as_ref().map(|u| u.estimated) as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.usage.as_ref().map(|u| u.input_tokens) as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.usage.as_ref().map(|u| u.output_tokens) as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.usage.as_ref().map(|u| u.total_tokens) as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.usage.as_ref().map(|u| u.model.clone()) as &(dyn tokio_postgres::types::ToSql + Sync),
                &metadata_json as &(dyn tokio_postgres::types::ToSql + Sync),
                &tenant_id as &(dyn tokio_postgres::types::ToSql + Sync),
                &request.consumer_id as &(dyn tokio_postgres::types::ToSql + Sync),
            ],
        )
        .await
        .to_store_error()?;

        Ok(log_id)
    }

    pub async fn get_api_usage_logs(
        &self,
        key_id: &str,
        tenant_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<ApiUsageLog>, StoreError> {
        let client = self.get_conn().await?;
        let limit = limit.unwrap_or(50).min(100);

        let rows = client
            .query(
                "SELECT id, key_id, email, endpoint_path, method, timestamp,
            response_status, response_time_ms, request_size, response_size,
            ip_address, user_agent, usage_estimated, input_tokens,
            output_tokens, total_tokens, model_used, metadata, consumer_id
            FROM api_usage_logs
            WHERE key_id = $1 AND tenant_id = $3
            ORDER BY timestamp DESC
            LIMIT $2",
                &[
                    &key_id as &(dyn tokio_postgres::types::ToSql + Sync), 
                    &limit as &(dyn tokio_postgres::types::ToSql + Sync),
                    &tenant_id as &(dyn tokio_postgres::types::ToSql + Sync)
                ],
            )
            .await
            .to_store_error()?;

        let mut logs = Vec::new();
        for row in rows {
            // Get metadata as string first, then parse to JSON
            let metadata_str: Option<String> = row.get(17);
            let metadata: Option<serde_json::Value> = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

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
                consumer_id: row.get(18),
            });
        }

        Ok(logs)
    }

    pub async fn update_credit_balance(&self, tenant_id: &str, email: &str, amount: i64, action_type: &str, description: Option<&str>) -> Result<i64, StoreError> {
        api_key_management::update_credit_balance(self, tenant_id, email, amount, action_type, description).await
    }

    pub async fn get_credit_balance(&self, tenant_id: &str) -> Result<i64, StoreError> {
        api_key_management::get_credit_balance(self, tenant_id).await
    }

    pub async fn get_credit_transactions(&self, tenant_id: &str, limit: i64) -> Result<Vec<crate::endpoint_store::models::CreditTransaction>, StoreError> {
        api_key_management::get_credit_transactions(self, tenant_id, limit).await
    }

    /// Returns Stripe top-up payments for a tenant (action_type = 'stripe_topup'),
    /// shaped for the PaymentHistory frontend interface.
    pub async fn get_payment_history(&self, tenant_id: &str) -> Result<Vec<serde_json::Value>, StoreError> {
        let client = self.get_conn().await?;

        let rows = client
            .query(
                "SELECT id, amount, description, created_at \
                 FROM credit_transactions \
                 WHERE tenant_id = $1 AND action_type = 'stripe_topup' \
                 ORDER BY created_at DESC \
                 LIMIT 50",
                &[&tenant_id],
            )
            .await
            .to_store_error()?;

        let payments: Vec<serde_json::Value> = rows.iter().map(|row| {
            let id: i64 = row.get(0);
            let amount: i64 = row.get(1);
            let description: Option<String> = row.get(2);
            let created_at: chrono::DateTime<chrono::Utc> = row.get(3);
            serde_json::json!({
                "id": id.to_string(),
                "amount": amount,
                "currency": "usd",
                "status": "succeeded",
                "created": created_at.to_rfc3339(),
                "description": description,
            })
        }).collect();

        Ok(payments)
    }

    pub async fn manage_single_endpoint(
        &self,
        email: &str,
        endpoint: &Endpoint,
    ) -> Result<String, StoreError> {
        manage_single_endpoint::manage_single_endpoint(self, email, endpoint).await
    }

    // ── MCP tools ─────────────────────────────────────────────────────────────

    pub async fn upsert_mcp_tool(
        &self,
        tenant_id: &str,
        req: &mcp_tools_management::UpsertMcpToolRequest,
    ) -> Result<mcp_tools_management::McpTool, StoreError> {
        mcp_tools_management::upsert_mcp_tool(self, tenant_id, req).await
    }

    pub async fn list_mcp_tools(
        &self,
        tenant_id: &str,
        user_email: Option<&str>,
    ) -> Result<Vec<mcp_tools_management::McpTool>, StoreError> {
        mcp_tools_management::list_mcp_tools(self, tenant_id, user_email).await
    }

    pub async fn get_mcp_tool(
        &self,
        tenant_id: &str,
        tool_name: &str,
        user_email: Option<&str>,
    ) -> Result<Option<mcp_tools_management::McpTool>, StoreError> {
        mcp_tools_management::get_mcp_tool(self, tenant_id, tool_name, user_email).await
    }

    pub async fn delete_mcp_tool(
        &self,
        tenant_id: &str,
        tool_name: &str,
    ) -> Result<bool, StoreError> {
        mcp_tools_management::delete_mcp_tool(self, tenant_id, tool_name).await
    }

    // ── Downstream auth ───────────────────────────────────────────────────────

    #[allow(dead_code)]
    pub async fn get_downstream_auth(
        &self,
        tenant_id: &str,
    ) -> Result<Option<downstream_auth_management::TenantDownstreamAuth>, StoreError> {
        downstream_auth_management::get_downstream_auth(self, tenant_id).await
    }

    #[allow(dead_code)]
    pub async fn save_downstream_auth(
        &self,
        tenant_id: &str,
        req: &downstream_auth_management::SaveDownstreamAuthRequest,
    ) -> Result<downstream_auth_management::TenantDownstreamAuth, StoreError> {
        downstream_auth_management::save_downstream_auth(self, tenant_id, req).await
    }

    // ── MCP client ID (per-provider OAuth) ────────────────────────────────────

    #[allow(dead_code)]
    pub async fn get_tenant_by_mcp_client_id(
        &self,
        mcp_client_id: &str,
    ) -> Result<Option<(models::Tenant, Option<String>)>, StoreError> {
        tenant_management::get_tenant_by_mcp_client_id(self, mcp_client_id).await
    }

    #[allow(dead_code)]
    pub async fn set_mcp_client_id(
        &self,
        email: &str,
        mcp_client_id: Option<&str>,
        google_client_id: Option<&str>,
    ) -> Result<(), StoreError> {
        tenant_management::set_mcp_client_id(self, email, mcp_client_id, google_client_id).await
    }
}
