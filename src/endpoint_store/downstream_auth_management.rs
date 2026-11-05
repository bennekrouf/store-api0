// src/endpoint_store/downstream_auth_management.rs
// CRUD for tenant_downstream_auth table.
//
// A tenant holds one row per named connection. The row named
// [`DEFAULT_CONNECTION`] is the tenant default — the row every tenant had
// before connections existed — and a tool that names no connection uses it.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::infra::secret_box::{self, SecretContext};
use crate::endpoint_store::{EndpointStore, StoreError};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The connection a tool uses when it names none.
pub const DEFAULT_CONNECTION: &str = "default";

const MAX_CONNECTION_NAME: usize = 40;

/// A connection name as stored: trimmed and lower-cased, empty meaning the
/// default. Names are slugs because they travel in URLs and in credential kinds
/// (`oauth_refresh@arm`), where an `@`, a `/` or a space would be ambiguous.
pub fn normalize_connection(raw: Option<&str>) -> Result<String, StoreError> {
    let name = raw.map(str::trim).unwrap_or("").to_lowercase();
    if name.is_empty() {
        return Ok(DEFAULT_CONNECTION.to_string());
    }
    let mut chars = name.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    let rest_ok = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if !first_ok || !rest_ok || name.len() > MAX_CONNECTION_NAME {
        return Err(StoreError::InvalidInput(format!(
            "'{}' is not a valid connection name: use lower-case letters, digits, '-' or '_', at most {} characters",
            name, MAX_CONNECTION_NAME
        )));
    }
    Ok(name)
}

/// The sealing purpose — and, for per-user secrets, the stored kind — of a
/// value that belongs to `connection`. The default connection keeps the bare
/// name, so everything sealed before connections existed still opens.
pub fn for_connection(base: &str, connection: &str) -> String {
    if connection == DEFAULT_CONNECTION {
        base.to_string()
    } else {
        format!("{}@{}", base, connection)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantDownstreamAuth {
    pub tenant_id: String,
    /// Which of the tenant's connections this is. "default" for the default.
    pub connection: String,
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
    /// Omitted or empty → the default connection.
    #[serde(default)]
    pub connection: Option<String>,
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

const SELECT_COLUMNS: &str =
    "SELECT tenant_id, auth_mode, service_account_json, target_audience,
            bearer_token, custom_headers, per_user_scheme, per_user_header,
            updated_at, bearer_token_enc, custom_headers_enc,
            service_account_json_enc, per_user_verify_url,
            per_user_identity_pointer, oauth_token_url, oauth_client_id,
            oauth_client_secret_enc, oauth_scope, oauth_client_auth_style,
            oauth_authorize_url, connection_name
     FROM tenant_downstream_auth";

fn row_to_auth(r: tokio_postgres::Row) -> TenantDownstreamAuth {
    let tenant_id: String = r.get(0);
    let connection: String = r.get(20);
    let purpose = |base: &str| for_connection(base, &connection);

    // Same rule as the other two: once a row has a sealed value, that value
    // is the only source. Falling back to plaintext when it fails to open
    // would quietly resurrect the column this change exists to retire.
    let custom_headers = match unseal_or_plain(r.get(10), None, &tenant_id, &purpose("custom_headers")) {
        Some(raw) => serde_json::from_str::<Value>(&raw).ok(),
        None if r.get::<_, Option<Vec<u8>>>(10).is_some() => None,
        None => r.get::<_, Option<Value>>(5),
    };

    TenantDownstreamAuth {
        auth_mode:            r.get(1),
        service_account_json: unseal_or_plain(r.get(11), r.get(2), &tenant_id, &purpose("service_account_json")),
        target_audience:      r.get(3),
        bearer_token:         unseal_or_plain(r.get(9), r.get(4), &tenant_id, &purpose("bearer_token")),
        custom_headers,
        per_user_scheme:      r.get(6),
        per_user_header:      r.get(7),
        per_user_verify_url:  r.get(12),
        per_user_identity_pointer: r.get(13),
        token_url:            r.get(14),
        client_id:            r.get(15),
        // Sealed only: this column is newer than the plaintext fallbacks,
        // so there is no historical value to fall back to.
        client_secret:        unseal_or_plain(r.get(16), None, &tenant_id, &purpose("oauth_client_secret")),
        scope:                r.get(17),
        client_auth_style:    r.get(18),
        authorize_url:        r.get(19),
        updated_at:           r.get::<_, chrono::DateTime<Utc>>(8).to_rfc3339(),
        connection:           connection.clone(),
        tenant_id,
    }
}

/// One connection of a tenant. `connection` is a stored name — see
/// [`normalize_connection`].
pub async fn get_downstream_auth(
    store: &EndpointStore,
    tenant_id: &str,
    connection: &str,
) -> Result<Option<TenantDownstreamAuth>, StoreError> {
    let client = store.get_conn(Some(tenant_id)).await?;

    let row = client
        .query_opt(
            &format!("{} WHERE tenant_id = $1 AND connection_name = $2", SELECT_COLUMNS),
            &[&tenant_id, &connection],
        )
        .await
        .to_store_error()?;

    Ok(row.map(row_to_auth))
}

/// Every connection of a tenant, the default first.
pub async fn list_downstream_connections(
    store: &EndpointStore,
    tenant_id: &str,
) -> Result<Vec<TenantDownstreamAuth>, StoreError> {
    let client = store.get_conn(Some(tenant_id)).await?;

    let rows = client
        .query(
            &format!(
                "{} WHERE tenant_id = $1
                 ORDER BY connection_name <> '{}', connection_name",
                SELECT_COLUMNS, DEFAULT_CONNECTION
            ),
            &[&tenant_id],
        )
        .await
        .to_store_error()?;

    Ok(rows.into_iter().map(row_to_auth).collect())
}

/// Remove a named connection, and every person's secrets for it.
///
/// The default cannot be removed — set it to `none` instead — and neither can a
/// connection a tool still uses: that tool would otherwise run with no
/// credential at all, which reads to its users as a broken backend.
pub async fn delete_downstream_connection(
    store: &EndpointStore,
    tenant_id: &str,
    connection: &str,
) -> Result<bool, StoreError> {
    if connection == DEFAULT_CONNECTION {
        return Err(StoreError::InvalidInput(
            "The default connection cannot be removed; set its auth mode to none instead".into(),
        ));
    }
    let client = store.get_conn(Some(tenant_id)).await?;

    let in_use: i64 = client
        .query_one(
            "SELECT count(*) FROM mcp_tools
             WHERE tenant_id = $1 AND connection_name = $2 AND is_active",
            &[&tenant_id, &connection],
        )
        .await
        .to_store_error()?
        .get(0);
    if in_use > 0 {
        return Err(StoreError::InvalidInput(format!(
            "{} tool(s) still use the connection '{}'; move them to another connection first",
            in_use, connection
        )));
    }

    let n = client
        .execute(
            "DELETE FROM tenant_downstream_auth WHERE tenant_id = $1 AND connection_name = $2",
            &[&tenant_id, &connection],
        )
        .await
        .to_store_error()?;

    // The kinds are '<base>@<connection>'; nothing else ends in that suffix.
    // '_' is a LIKE wildcard and may appear in a name, so it is escaped:
    // otherwise removing 'a_b' would also take the secrets of 'axb'.
    let suffix = format!("%@{}", connection.replace('_', "\\_"));
    client
        .execute(
            "DELETE FROM user_downstream_credentials WHERE tenant_id = $1 AND kind LIKE $2",
            &[&tenant_id, &suffix],
        )
        .await
        .to_store_error()?;

    app_log!(info, tenant_id = %tenant_id, connection = %connection, "Removed a downstream connection");
    Ok(n > 0)
}

pub async fn save_downstream_auth(
    store: &EndpointStore,
    tenant_id: &str,
    req: &SaveDownstreamAuthRequest,
) -> Result<TenantDownstreamAuth, StoreError> {
    let connection = normalize_connection(req.connection.as_deref())?;
    let purpose = |base: &str| for_connection(base, &connection);
    let client = store.get_conn(Some(tenant_id)).await?;
    let now = Utc::now();

    // Seal on the way in, and write NULL to the plaintext columns. Anything
    // saved from this deploy onward is encrypted whether or not the backfill of
    // older rows has been run.
    let bearer_enc = match req.bearer_token.as_deref().filter(|v| !v.is_empty()) {
        Some(v) => Some(seal_for(v, tenant_id, &purpose("bearer_token"))?),
        None => None,
    };
    let sa_enc = match req.service_account_json.as_deref().filter(|v| !v.is_empty()) {
        Some(v) => Some(seal_for(v, tenant_id, &purpose("service_account_json"))?),
        None => None,
    };
    let headers_enc = match req.custom_headers.as_ref().filter(|v| !v.is_null()) {
        Some(v) => Some(seal_for(&v.to_string(), tenant_id, &purpose("custom_headers"))?),
        None => None,
    };
    let client_secret_enc = match req.client_secret.as_deref().filter(|v| !v.is_empty()) {
        Some(v) => Some(seal_for(v, tenant_id, &purpose("oauth_client_secret"))?),
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
                 oauth_authorize_url, connection_name)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
                     $15, $16, $17, $18, $19, $20, $21)
             ON CONFLICT (tenant_id, connection_name) DO UPDATE SET
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
                &connection as &(dyn tokio_postgres::types::ToSql + Sync),
            ],
        )
        .await
        .to_store_error()?;

    app_log!(info, tenant_id = %tenant_id, connection = %connection, mode = %req.auth_mode, "Saved downstream auth config");

    // The plaintext columns are now NULL by design, so the response echoes what
    // the caller submitted rather than what the row literally holds.
    Ok(TenantDownstreamAuth {
        tenant_id:            row.get(0),
        connection,
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
    use super::{for_connection, normalize_connection, unseal_or_plain, DEFAULT_CONNECTION};

    #[test]
    fn an_absent_or_blank_connection_is_the_default() {
        assert_eq!(normalize_connection(None).unwrap(), DEFAULT_CONNECTION);
        assert_eq!(normalize_connection(Some("  ")).unwrap(), DEFAULT_CONNECTION);
        assert_eq!(normalize_connection(Some(" ARM ")).unwrap(), "arm");
        assert_eq!(normalize_connection(Some("log-analytics_2")).unwrap(), "log-analytics_2");
    }

    #[test]
    fn a_connection_name_that_would_be_ambiguous_in_a_kind_is_refused() {
        for bad in ["a@b", "a/b", "a b", "-arm", "_arm", &"x".repeat(41)] {
            assert!(normalize_connection(Some(bad)).is_err(), "{} should be refused", bad);
        }
    }

    #[test]
    fn the_default_connection_keeps_the_bare_purpose() {
        // Everything sealed before connections existed must still open.
        assert_eq!(for_connection("oauth_refresh", DEFAULT_CONNECTION), "oauth_refresh");
        assert_eq!(for_connection("oauth_refresh", "arm"), "oauth_refresh@arm");
    }

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
