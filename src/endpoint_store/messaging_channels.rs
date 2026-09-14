// src/endpoint_store/messaging_channels.rs
//
// A tenant's bot on a messaging platform. The credential is sealed on the way
// in and opened only for the bridge, which is the one caller that has to
// present it to the platform.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::infra::secret_box::{self, SecretContext};
use chrono::Utc;
use serde::Serialize;

const SECRET_PURPOSE: &str = "messaging_channel_credential";

/// What a person sees about their channel. Never the credential.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelSummary {
    pub channel: String,
    pub channel_ref: String,
    pub tenant_id: String,
    pub display_ref: String,
    pub system_prompt: String,
    pub created_at: String,
}

/// What the bridge needs to serve a channel.
pub struct ChannelForBridge {
    pub tenant_id: String,
    pub credential: String,
    pub webhook_secret: String,
    pub system_prompt: String,
}

pub struct RegisterChannel<'a> {
    pub channel: &'a str,
    pub channel_ref: &'a str,
    pub credential: &'a str,
    pub display_ref: &'a str,
    pub webhook_secret: &'a str,
    pub system_prompt: &'a str,
}

pub async fn register_channel(
    store: &EndpointStore,
    tenant_id: &str,
    req: RegisterChannel<'_>,
) -> Result<ChannelSummary, StoreError> {
    if !secret_box::is_configured() {
        return Err(StoreError::InvalidInput(
            "API0_ENCRYPTION_KEY is not set — the bot token could not be protected".into(),
        ));
    }

    let sealed = secret_box::seal(req.credential, &SecretContext { tenant_id, purpose: SECRET_PURPOSE })
        .map_err(|e| StoreError::InvalidInput(format!("could not store the bot token: {}", e)))?;

    let client = store.get_admin_conn().await?;

    // One bot per platform per tenant: registering again replaces it.
    let row = client
        .query_one(
            "INSERT INTO messaging_channels
                (channel, channel_ref, tenant_id, credential_enc, display_ref,
                 webhook_secret, system_prompt, created_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
             ON CONFLICT (channel, tenant_id) DO UPDATE SET
                channel_ref    = EXCLUDED.channel_ref,
                credential_enc = EXCLUDED.credential_enc,
                display_ref    = EXCLUDED.display_ref,
                webhook_secret = EXCLUDED.webhook_secret,
                system_prompt  = EXCLUDED.system_prompt
             RETURNING channel, channel_ref, tenant_id, display_ref, system_prompt, created_at",
            &[
                &req.channel,
                &req.channel_ref,
                &tenant_id,
                &sealed,
                &req.display_ref,
                &req.webhook_secret,
                &req.system_prompt,
            ],
        )
        .await
        .to_store_error()?;

    app_log!(info, channel = %req.channel, tenant_id = %tenant_id, "Registered a messaging channel");
    Ok(row_to_summary(row))
}

/// Resolve a channel from what the platform sends — the bridge's lookup.
pub async fn channel_for_bridge(
    store: &EndpointStore,
    channel: &str,
    channel_ref: &str,
) -> Result<Option<ChannelForBridge>, StoreError> {
    let client = store.get_admin_conn().await?;
    let row = client
        .query_opt(
            "SELECT tenant_id, credential_enc, webhook_secret, system_prompt
             FROM messaging_channels WHERE channel = $1 AND channel_ref = $2",
            &[&channel, &channel_ref],
        )
        .await
        .to_store_error()?;

    let row = match row {
        Some(r) => r,
        None => return Ok(None),
    };
    let tenant_id: String = row.get(0);
    let sealed: Vec<u8> = row.get(1);

    let credential = secret_box::open(&sealed, &SecretContext { tenant_id: &tenant_id, purpose: SECRET_PURPOSE })
        .map_err(|e| {
            app_log!(error, channel = %channel, error = %e, "Channel credential would not open");
            StoreError::Database("channel credential could not be read".into())
        })?;

    Ok(Some(ChannelForBridge {
        tenant_id,
        credential,
        webhook_secret: row.get(2),
        system_prompt: row.get(3),
    }))
}

pub async fn list_channels(
    store: &EndpointStore,
    tenant_id: &str,
) -> Result<Vec<ChannelSummary>, StoreError> {
    let client = store.get_admin_conn().await?;
    let rows = client
        .query(
            "SELECT channel, channel_ref, tenant_id, display_ref, system_prompt, created_at
             FROM messaging_channels WHERE tenant_id = $1 ORDER BY channel",
            &[&tenant_id],
        )
        .await
        .to_store_error()?;
    Ok(rows.into_iter().map(row_to_summary).collect())
}

/// Remove a channel, returning its credential so the caller can tell the
/// platform to stop sending updates.
pub async fn delete_channel(
    store: &EndpointStore,
    tenant_id: &str,
    channel: &str,
) -> Result<Option<String>, StoreError> {
    let client = store.get_admin_conn().await?;
    let row = client
        .query_opt(
            "DELETE FROM messaging_channels WHERE tenant_id = $1 AND channel = $2
             RETURNING credential_enc",
            &[&tenant_id, &channel],
        )
        .await
        .to_store_error()?;

    match row {
        Some(r) => {
            let sealed: Vec<u8> = r.get(0);
            Ok(secret_box::open(&sealed, &SecretContext { tenant_id, purpose: SECRET_PURPOSE }).ok())
        }
        None => Ok(None),
    }
}

fn row_to_summary(row: tokio_postgres::Row) -> ChannelSummary {
    ChannelSummary {
        channel:       row.get(0),
        channel_ref:   row.get(1),
        tenant_id:     row.get(2),
        display_ref:   row.get(3),
        system_prompt: row.get(4),
        created_at:    row.get::<_, chrono::DateTime<Utc>>(5).to_rfc3339(),
    }
}
