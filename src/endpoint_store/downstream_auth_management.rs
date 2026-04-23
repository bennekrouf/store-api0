// src/endpoint_store/downstream_auth_management.rs
// CRUD for tenant_downstream_auth table.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantDownstreamAuth {
    pub tenant_id: String,
    pub auth_mode: String, // "none" | "google_sa" | "static_bearer" | "header_injection"
    pub service_account_json: Option<String>,
    pub target_audience: Option<String>,
    pub bearer_token: Option<String>,
    pub custom_headers: Option<Value>, // JSONB: {"Header-Name": "value"}
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveDownstreamAuthRequest {
    pub auth_mode: String,
    pub service_account_json: Option<String>,
    pub target_audience: Option<String>,
    pub bearer_token: Option<String>,
    pub custom_headers: Option<Value>,
}

pub async fn get_downstream_auth(
    store: &EndpointStore,
    tenant_id: &str,
) -> Result<Option<TenantDownstreamAuth>, StoreError> {
    let client = store.get_conn(Some(tenant_id)).await?;

    let row = client
        .query_opt(
            "SELECT tenant_id, auth_mode, service_account_json, target_audience,
                    bearer_token, custom_headers, updated_at
             FROM tenant_downstream_auth WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| TenantDownstreamAuth {
        tenant_id:            r.get(0),
        auth_mode:            r.get(1),
        service_account_json: r.get(2),
        target_audience:      r.get(3),
        bearer_token:         r.get(4),
        custom_headers:       r.get(5),
        updated_at:           r.get::<_, chrono::DateTime<Utc>>(6).to_rfc3339(),
    }))
}

pub async fn save_downstream_auth(
    store: &EndpointStore,
    tenant_id: &str,
    req: &SaveDownstreamAuthRequest,
) -> Result<TenantDownstreamAuth, StoreError> {
    let client = store.get_conn(Some(tenant_id)).await?;
    let now = Utc::now();

    let row = client
        .query_one(
            "INSERT INTO tenant_downstream_auth
                (tenant_id, auth_mode, service_account_json, target_audience,
                 bearer_token, custom_headers, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (tenant_id) DO UPDATE SET
                auth_mode            = EXCLUDED.auth_mode,
                service_account_json = EXCLUDED.service_account_json,
                target_audience      = EXCLUDED.target_audience,
                bearer_token         = EXCLUDED.bearer_token,
                custom_headers       = EXCLUDED.custom_headers,
                updated_at           = EXCLUDED.updated_at
             RETURNING tenant_id, auth_mode, service_account_json, target_audience,
                       bearer_token, custom_headers, updated_at",
            &[
                &tenant_id as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.auth_mode as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.service_account_json as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.target_audience as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.bearer_token as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.custom_headers as &(dyn tokio_postgres::types::ToSql + Sync),
                &now as &(dyn tokio_postgres::types::ToSql + Sync),
            ],
        )
        .await
        .to_store_error()?;

    app_log!(info, tenant_id = %tenant_id, mode = %req.auth_mode, "Saved downstream auth config");

    Ok(TenantDownstreamAuth {
        tenant_id:            row.get(0),
        auth_mode:            row.get(1),
        service_account_json: row.get(2),
        target_audience:      row.get(3),
        bearer_token:         row.get(4),
        custom_headers:       row.get(5),
        updated_at:           row.get::<_, chrono::DateTime<Utc>>(6).to_rfc3339(),
    })
}
