// src/endpoint_store/downstream_auth_management.rs
// CRUD for tenant_downstream_auth table.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::infra::secret_box::{self, SecretContext};
use crate::endpoint_store::{EndpointStore, StoreError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantDownstreamAuth {
    pub tenant_id: String,
    // "none" | "google_sa" | "static_bearer" | "header_injection" | "per_user"
    //        | "client_credentials"
    pub auth_mode: String,
    pub service_account_json: Option<String>,
    pub target_audience: Option<String>,
    pub bearer_token: Option<String>,
    pub custom_headers: Option<Value>, // JSONB: {"Header-Name": "value"}
    // per_user: how each user's own secret becomes a header.
    //   scheme "basic_pat" → Authorization: Basic base64(":" + secret)  (Azure DevOps)
    //   scheme "bearer"    → Authorization: Bearer <secret>
    //   scheme "raw"       → <header>: <secret>
    pub per_user_scheme: Option<String>,
    pub per_user_header: Option<String>,
    /// Read-only endpoint that answers "who is this token?" — see schema.sql.
    pub per_user_verify_url: Option<String>,
    /// JSON pointer into that endpoint's response.
    pub per_user_identity_pointer: Option<String>,
    // client_credentials. These names are the gateway's contract — see
    // DownstreamAuth::from_tenant_config, which reads them off this JSON.
    pub token_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scope: Option<String>,
    pub client_auth_style: Option<String>,
    /// per_user_oauth only: where the person is sent to approve access.
    pub authorize_url: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveDownstreamAuthRequest {
    pub auth_mode: String,
    pub service_account_json: Option<String>,
    pub target_audience: Option<String>,
    pub bearer_token: Option<String>,
    pub custom_headers: Option<Value>,
    pub per_user_scheme: Option<String>,
    pub per_user_header: Option<String>,
    pub per_user_verify_url: Option<String>,
    pub per_user_identity_pointer: Option<String>,
    pub token_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub scope: Option<String>,
    pub client_auth_style: Option<String>,
    pub authorize_url: Option<String>,
}

/// Open a sealed column, falling back to the plaintext one for rows the backfill
/// has not reached.
///
/// A sealed value that will not open is *not* silently treated as absent: that
/// would hand the caller a null credential and produce a confusing 401 from some
/// third party. It is logged and surfaced as missing, which at least says the
/// problem is here.
fn unseal_or_plain(
    sealed: Option<Vec<u8>>,
    plain: Option<String>,
    tenant_id: &str,
    purpose: &str,
) -> Option<String> {
    match sealed {
        Some(bytes) => match secret_box::open(&bytes, &SecretContext { tenant_id, purpose }) {
            Ok(v) => Some(v),
            Err(e) => {
                app_log!(error, tenant_id = %tenant_id, purpose = %purpose, error = %e,
                    "Sealed downstream secret would not open");
                None
            }
        },
        None => plain,
    }
}

/// Seal a value for storage, or fail loudly. Returning the plaintext on error
/// would quietly reintroduce exactly what this is meant to remove.
fn seal_for(value: &str, tenant_id: &str, purpose: &str) -> Result<Vec<u8>, StoreError> {
    secret_box::seal(value, &SecretContext { tenant_id, purpose }).map_err(|e| {
        app_log!(error, tenant_id = %tenant_id, purpose = %purpose, error = %e,
            "Could not seal a downstream secret");
        StoreError::InvalidInput(format!("could not store the credential: {}", e))
    })
}

pub async fn get_downstream_auth(
    store: &EndpointStore,
    tenant_id: &str,
) -> Result<Option<TenantDownstreamAuth>, StoreError> {
    let client = store.get_conn(Some(tenant_id)).await?;

    let row = client
        .query_opt(
            "SELECT tenant_id, auth_mode, service_account_json, target_audience,
                    bearer_token, custom_headers, per_user_scheme, per_user_header,
                    updated_at, bearer_token_enc, custom_headers_enc,
                    service_account_json_enc, per_user_verify_url,
                    per_user_identity_pointer, oauth_token_url, oauth_client_id,
                    oauth_client_secret_enc, oauth_scope, oauth_client_auth_style,
                    oauth_authorize_url
             FROM tenant_downstream_auth WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| {
        // Same rule as the other two: once a row has a sealed value, that value
        // is the only source. Falling back to plaintext when it fails to open
        // would quietly resurrect the column this change exists to retire.
        let custom_headers = match unseal_or_plain(r.get(10), None, tenant_id, "custom_headers") {
            Some(raw) => serde_json::from_str::<Value>(&raw).ok(),
            None if r.get::<_, Option<Vec<u8>>>(10).is_some() => None,
            None => r.get::<_, Option<Value>>(5),
        };

        TenantDownstreamAuth {
            tenant_id:            r.get(0),
            auth_mode:            r.get(1),
            service_account_json: unseal_or_plain(r.get(11), r.get(2), tenant_id, "service_account_json"),
            target_audience:      r.get(3),
            bearer_token:         unseal_or_plain(r.get(9), r.get(4), tenant_id, "bearer_token"),
            custom_headers,
            per_user_scheme:      r.get(6),
            per_user_header:      r.get(7),
            per_user_verify_url:  r.get(12),
            per_user_identity_pointer: r.get(13),
            token_url:            r.get(14),
            client_id:            r.get(15),
            // Sealed only: this column is newer than the plaintext fallbacks,
            // so there is no historical value to fall back to.
            client_secret:        unseal_or_plain(r.get(16), None, tenant_id, "oauth_client_secret"),
            scope:                r.get(17),
            client_auth_style:    r.get(18),
            authorize_url:        r.get(19),
            updated_at:           r.get::<_, chrono::DateTime<Utc>>(8).to_rfc3339(),
        }
    }))
}

pub async fn save_downstream_auth(
    store: &EndpointStore,
    tenant_id: &str,
    req: &SaveDownstreamAuthRequest,
) -> Result<TenantDownstreamAuth, StoreError> {
    let client = store.get_conn(Some(tenant_id)).await?;
    let now = Utc::now();

    // Seal on the way in, and write NULL to the plaintext columns. Anything
    // saved from this deploy onward is encrypted whether or not the backfill of
    // older rows has been run.
    let bearer_enc = match req.bearer_token.as_deref().filter(|v| !v.is_empty()) {
        Some(v) => Some(seal_for(v, tenant_id, "bearer_token")?),
        None => None,
    };
    let sa_enc = match req.service_account_json.as_deref().filter(|v| !v.is_empty()) {
        Some(v) => Some(seal_for(v, tenant_id, "service_account_json")?),
        None => None,
    };
    let headers_enc = match req.custom_headers.as_ref().filter(|v| !v.is_null()) {
        Some(v) => Some(seal_for(&v.to_string(), tenant_id, "custom_headers")?),
        None => None,
    };
    let client_secret_enc = match req.client_secret.as_deref().filter(|v| !v.is_empty()) {
        Some(v) => Some(seal_for(v, tenant_id, "oauth_client_secret")?),
        None => None,
    };
    let no_plaintext: Option<String> = None;
    let no_plaintext_json: Option<Value> = None;

    let row = client
        .query_one(
            "INSERT INTO tenant_downstream_auth
                (tenant_id, auth_mode, service_account_json, target_audience,
                 bearer_token, custom_headers, per_user_scheme, per_user_header,
                 updated_at, bearer_token_enc, service_account_json_enc,
                 custom_headers_enc, per_user_verify_url,
                 per_user_identity_pointer, oauth_token_url, oauth_client_id,
                 oauth_client_secret_enc, oauth_scope, oauth_client_auth_style,
                 oauth_authorize_url)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                     $15, $16, $17, $18, $19, $20)
             ON CONFLICT (tenant_id) DO UPDATE SET
                auth_mode                = EXCLUDED.auth_mode,
                service_account_json     = EXCLUDED.service_account_json,
                target_audience          = EXCLUDED.target_audience,
                bearer_token             = EXCLUDED.bearer_token,
                custom_headers           = EXCLUDED.custom_headers,
                per_user_scheme          = EXCLUDED.per_user_scheme,
                per_user_header          = EXCLUDED.per_user_header,
                per_user_verify_url      = EXCLUDED.per_user_verify_url,
                per_user_identity_pointer = EXCLUDED.per_user_identity_pointer,
                updated_at               = EXCLUDED.updated_at,
                bearer_token_enc         = EXCLUDED.bearer_token_enc,
                service_account_json_enc = EXCLUDED.service_account_json_enc,
                custom_headers_enc       = EXCLUDED.custom_headers_enc,
                oauth_token_url          = EXCLUDED.oauth_token_url,
                oauth_client_id          = EXCLUDED.oauth_client_id,
                oauth_client_secret_enc  = EXCLUDED.oauth_client_secret_enc,
                oauth_scope              = EXCLUDED.oauth_scope,
                oauth_client_auth_style  = EXCLUDED.oauth_client_auth_style,
                oauth_authorize_url      = EXCLUDED.oauth_authorize_url
             RETURNING tenant_id, auth_mode, service_account_json, target_audience,
                       bearer_token, custom_headers, per_user_scheme,
                       per_user_header, updated_at",
            &[
                &tenant_id as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.auth_mode as &(dyn tokio_postgres::types::ToSql + Sync),
                &no_plaintext as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.target_audience as &(dyn tokio_postgres::types::ToSql + Sync),
                &no_plaintext as &(dyn tokio_postgres::types::ToSql + Sync),
                &no_plaintext_json as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.per_user_scheme as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.per_user_header as &(dyn tokio_postgres::types::ToSql + Sync),
                &now as &(dyn tokio_postgres::types::ToSql + Sync),
                &bearer_enc as &(dyn tokio_postgres::types::ToSql + Sync),
                &sa_enc as &(dyn tokio_postgres::types::ToSql + Sync),
                &headers_enc as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.per_user_verify_url as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.per_user_identity_pointer as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.token_url as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.client_id as &(dyn tokio_postgres::types::ToSql + Sync),
                &client_secret_enc as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.scope as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.client_auth_style as &(dyn tokio_postgres::types::ToSql + Sync),
                &req.authorize_url as &(dyn tokio_postgres::types::ToSql + Sync),
            ],
        )
        .await
        .to_store_error()?;

    app_log!(info, tenant_id = %tenant_id, mode = %req.auth_mode, "Saved downstream auth config");

    // The plaintext columns are now NULL by design, so the response echoes what
    // the caller submitted rather than what the row literally holds.
    Ok(TenantDownstreamAuth {
        tenant_id:            row.get(0),
        auth_mode:            row.get(1),
        service_account_json: req.service_account_json.clone(),
        target_audience:      row.get(3),
        bearer_token:         req.bearer_token.clone(),
        custom_headers:       req.custom_headers.clone(),
        per_user_scheme:      row.get(6),
        per_user_header:      row.get(7),
        per_user_verify_url:  req.per_user_verify_url.clone(),
        per_user_identity_pointer: req.per_user_identity_pointer.clone(),
        token_url:            req.token_url.clone(),
        client_id:            req.client_id.clone(),
        client_secret:        req.client_secret.clone(),
        scope:                req.scope.clone(),
        client_auth_style:    req.client_auth_style.clone(),
        authorize_url:        req.authorize_url.clone(),
        updated_at:           row.get::<_, chrono::DateTime<Utc>>(8).to_rfc3339(),
    })
}

#[cfg(test)]
mod tests {
    use super::unseal_or_plain;

    #[test]
    fn a_row_with_no_sealed_value_falls_back_to_plaintext() {
        // Rows the backfill has not reached must keep working.
        let got = unseal_or_plain(None, Some("legacy-token".into()), "t1", "bearer_token");
        assert_eq!(got.as_deref(), Some("legacy-token"));
    }

    #[test]
    fn a_sealed_value_that_will_not_open_does_not_fall_back() {
        // The property that matters. A corrupt or wrong-key ciphertext must read
        // as absent, never as "use the plaintext we were trying to retire".
        let got = unseal_or_plain(
            Some(vec![1, 2, 3, 4, 5]),
            Some("legacy-token".into()),
            "t1",
            "bearer_token",
        );
        assert_eq!(got, None);
    }

    #[test]
    fn nothing_stored_reads_as_nothing() {
        assert_eq!(unseal_or_plain(None, None, "t1", "bearer_token"), None);
    }
}
