// src/endpoint_store/idp_management.rs
//
// A tenant's inbound identity provider: where its people sign in.
//
// The client secret is sealed on the way in and opened only for the gateway,
// which is the single caller that needs it. Nothing here is reachable from a
// browser — see the route guards in http_server.rs.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::infra::secret_box::{self, SecretContext};
use serde::{Deserialize, Serialize};

const SECRET_PURPOSE: &str = "idp_client_secret";

/// What the gateway needs to run an OIDC sign-in for a tenant.
#[derive(Debug, Clone, Serialize)]
pub struct TenantIdp {
    pub tenant_id: String,
    pub tenant_name: String,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
}

#[derive(Debug, Deserialize)]
pub struct SaveIdpRequest {
    pub issuer: String,
    pub client_id: String,
    /// Omit to keep the stored secret — so an admin can change the issuer
    /// without re-entering a secret they cannot read back.
    pub client_secret: Option<String>,
}

/// Resolve a tenant's IdP from the OAuth client id its connector uses.
pub async fn get_idp_by_mcp_client_id(
    store: &EndpointStore,
    mcp_client_id: &str,
) -> Result<Option<TenantIdp>, StoreError> {
    let client = store.get_admin_conn().await?;

    let row = client
        .query_opt(
            "SELECT id, name, idp_issuer, idp_client_id, idp_client_secret
             FROM tenants
             WHERE mcp_client_id = $1
               AND idp_issuer IS NOT NULL AND idp_client_id IS NOT NULL",
            &[&mcp_client_id],
        )
        .await
        .to_store_error()?;

    let row = match row {
        Some(r) => r,
        None => return Ok(None),
    };

    let tenant_id: String = row.get(0);
    let sealed: Option<Vec<u8>> = row.get(4);

    // A configured issuer with no openable secret is a broken configuration, not
    // a tenant without an IdP. Saying "no IdP" here would silently fall back to
    // another sign-in method, which is the failure this whole area is prone to.
    let client_secret = match sealed {
        Some(bytes) => secret_box::open(
            &bytes,
            &SecretContext { tenant_id: &tenant_id, purpose: SECRET_PURPOSE },
        )
        .map_err(|e| {
            app_log!(error, tenant_id = %tenant_id, error = %e, "IdP client secret would not open");
            StoreError::Database("stored IdP secret could not be read".into())
        })?,
        None => {
            return Err(StoreError::InvalidInput(
                "this workspace has an issuer configured but no client secret".into(),
            ))
        }
    };

    Ok(Some(TenantIdp {
        tenant_id,
        tenant_name: row.get(1),
        issuer: row.get(2),
        client_id: row.get(3),
        client_secret,
    }))
}

pub async fn save_idp(
    store: &EndpointStore,
    tenant_id: &str,
    req: &SaveIdpRequest,
) -> Result<(), StoreError> {
    if !secret_box::is_configured() {
        return Err(StoreError::InvalidInput(
            "API0_ENCRYPTION_KEY is not set — the client secret could not be protected".into(),
        ));
    }

    let issuer = req.issuer.trim().trim_end_matches('/').to_string();
    if !issuer.starts_with("https://") {
        return Err(StoreError::InvalidInput("issuer must be an https URL".into()));
    }

    let client = store.get_admin_conn().await?;

    match req.client_secret.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(secret) => {
            let sealed = secret_box::seal(
                secret,
                &SecretContext { tenant_id, purpose: SECRET_PURPOSE },
            )
            .map_err(|e| StoreError::InvalidInput(format!("could not store the secret: {}", e)))?;

            client
                .execute(
                    "UPDATE tenants SET idp_issuer = $2, idp_client_id = $3,
                            idp_client_secret = $4 WHERE id = $1",
                    &[&tenant_id, &issuer, &req.client_id, &sealed],
                )
                .await
                .to_store_error()?;
        }
        None => {
            client
                .execute(
                    "UPDATE tenants SET idp_issuer = $2, idp_client_id = $3 WHERE id = $1",
                    &[&tenant_id, &issuer, &req.client_id],
                )
                .await
                .to_store_error()?;
        }
    }

    app_log!(info, tenant_id = %tenant_id, issuer = %issuer, "Saved tenant IdP");
    Ok(())
}

// ── In-flight sign-ins ───────────────────────────────────────────────────────

pub async fn remember_auth_request(
    store: &EndpointStore,
    state_nonce: &str,
    tenant_id: &str,
    verifier: &str,
) -> Result<(), StoreError> {
    let client = store.get_admin_conn().await?;

    // Opportunistic sweep. These live for minutes; anything older is abandoned.
    let _ = client
        .execute(
            "DELETE FROM idp_auth_requests WHERE created_at < NOW() - INTERVAL '15 minutes'",
            &[],
        )
        .await;

    client
        .execute(
            "INSERT INTO idp_auth_requests (state_nonce, tenant_id, verifier)
             VALUES ($1, $2, $3)
             ON CONFLICT (state_nonce) DO NOTHING",
            &[&state_nonce, &tenant_id, &verifier],
        )
        .await
        .to_store_error()?;

    Ok(())
}

/// Take the verifier for a sign-in and delete the row.
///
/// Single use: a replayed callback finds nothing, which is the point.
pub async fn consume_auth_request(
    store: &EndpointStore,
    state_nonce: &str,
) -> Result<Option<String>, StoreError> {
    let client = store.get_admin_conn().await?;

    let row = client
        .query_opt(
            "DELETE FROM idp_auth_requests
             WHERE state_nonce = $1 AND created_at > NOW() - INTERVAL '15 minutes'
             RETURNING verifier",
            &[&state_nonce],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| r.get(0)))
}
