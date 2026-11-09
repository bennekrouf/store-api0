// src/endpoint_store/encrypt_legacy.rs
//
// One-shot backfill: seal the secrets written before infra::secret_box existed.
//
// Everything saved since the sealed columns landed is already encrypted — this
// only reaches rows that predate it. It is deliberately not run at startup:
// rewriting live credentials in place is the kind of thing that wants a database
// backup taken first and a human watching, not a side effect of a deploy.
//
// Safe to run repeatedly. A row whose sealed column is already populated is
// skipped, so a partial run can simply be run again.

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::messaging_channels::WEBHOOK_SECRET_PURPOSE;
use crate::endpoint_store::{EndpointStore, StoreError};
use crate::infra::secret_box::{self, SecretContext};
use crate::whatsapp::channel::{APP_SECRET_PURPOSE, VERIFY_TOKEN_PURPOSE, WA_TOKEN_PURPOSE};
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Default, Serialize)]
pub struct BackfillReport {
    pub bearer_tokens: usize,
    pub custom_headers: usize,
    pub service_accounts: usize,
    pub static_headers: usize,
    pub whatsapp_secrets: usize,
    pub webhook_secrets: usize,
    /// Rows that could not be sealed. Their plaintext is left exactly as it was,
    /// so a failure here loses nothing and can be retried.
    pub failures: Vec<String>,
}

pub async fn encrypt_legacy_secrets(store: &EndpointStore) -> Result<BackfillReport, StoreError> {
    if !secret_box::is_configured() {
        return Err(StoreError::InvalidInput(
            "API0_ENCRYPTION_KEY is not set — nothing would be encrypted".into(),
        ));
    }

    // Admin connection: this crosses every tenant by design, which is exactly
    // what RLS would otherwise prevent.
    let client = store.get_admin_conn().await?;
    let mut report = BackfillReport::default();

    // ── tenant_downstream_auth ───────────────────────────────────────────────
    let rows = client
        .query(
            "SELECT tenant_id, bearer_token, custom_headers, service_account_json
             FROM tenant_downstream_auth
             -- Plaintext predates named connections, so only a default row can
             -- hold any; the bare sealing purposes below are the default's.
             WHERE connection_name = 'default'
               AND ((bearer_token         IS NOT NULL AND bearer_token_enc         IS NULL)
                 OR (custom_headers       IS NOT NULL AND custom_headers_enc       IS NULL)
                 OR (service_account_json IS NOT NULL AND service_account_json_enc IS NULL))",
            &[],
        )
        .await
        .to_store_error()?;

    for row in rows {
        let tenant_id: String = row.get(0);
        let bearer: Option<String> = row.get(1);
        let headers: Option<Value> = row.get(2);
        let sa: Option<String> = row.get(3);

        let sealed_bearer = seal_opt(bearer.as_deref(), &tenant_id, "bearer_token", &mut report);
        let sealed_headers = seal_opt(
            headers.as_ref().map(|v| v.to_string()).as_deref(),
            &tenant_id,
            "custom_headers",
            &mut report,
        );
        let sealed_sa = seal_opt(sa.as_deref(), &tenant_id, "service_account_json", &mut report);

        // COALESCE keeps any column that was already sealed on an earlier run. A
        // plaintext value is cleared only when it was sealed in this pass.
        let updated = client
            .execute(
                "UPDATE tenant_downstream_auth SET
                    bearer_token_enc         = COALESCE(bearer_token_enc, $2),
                    custom_headers_enc       = COALESCE(custom_headers_enc, $3),
                    service_account_json_enc = COALESCE(service_account_json_enc, $4),
                    bearer_token             = CASE WHEN $2 IS NULL THEN bearer_token END,
                    custom_headers           = CASE WHEN $3 IS NULL THEN custom_headers END,
                    service_account_json     = CASE WHEN $4 IS NULL THEN service_account_json END
                 WHERE tenant_id = $1 AND connection_name = 'default'",
                &[&tenant_id, &sealed_bearer, &sealed_headers, &sealed_sa],
            )
            .await;

        match updated {
            Ok(_) => {
                if sealed_bearer.is_some() { report.bearer_tokens += 1; }
                if sealed_headers.is_some() { report.custom_headers += 1; }
                if sealed_sa.is_some() { report.service_accounts += 1; }
            }
            Err(e) => report
                .failures
                .push(format!("tenant_downstream_auth {}: {}", tenant_id, e)),
        }
    }

    // ── mcp_tools.static_headers ─────────────────────────────────────────────
    let rows = client
        .query(
            "SELECT id, tenant_id, static_headers FROM mcp_tools
             WHERE static_headers IS NOT NULL AND static_headers_enc IS NULL",
            &[],
        )
        .await
        .to_store_error()?;

    for row in rows {
        let id: String = row.get(0);
        let tenant_id: String = row.get(1);
        let headers: Option<Value> = row.get(2);

        let sealed = seal_opt(
            headers.as_ref().map(|v| v.to_string()).as_deref(),
            &tenant_id,
            "static_headers",
            &mut report,
        );
        if sealed.is_none() {
            continue;
        }

        match client
            .execute(
                "UPDATE mcp_tools SET static_headers_enc = $2, static_headers = NULL
                 WHERE id = $1",
                &[&id, &sealed],
            )
            .await
        {
            Ok(_) => report.static_headers += 1,
            Err(e) => report.failures.push(format!("mcp_tools {}: {}", id, e)),
        }
    }

    // ── whatsapp_channels ────────────────────────────────────────────────────
    let rows = client
        .query(
            "SELECT phone_number_id, tenant_id, wa_token, verify_token, app_secret
             FROM whatsapp_channels
             WHERE wa_token IS NOT NULL OR verify_token IS NOT NULL OR app_secret IS NOT NULL",
            &[],
        )
        .await
        .to_store_error()?;

    for row in rows {
        let phone_number_id: String = row.get(0);
        let tenant_id: String = row.get(1);
        let wa_token: Option<String> = row.get(2);
        let verify_token: Option<String> = row.get(3);
        let app_secret: Option<String> = row.get(4);

        let sealed_wa = seal_opt(wa_token.as_deref(), &tenant_id, WA_TOKEN_PURPOSE, &mut report);
        let sealed_verify = seal_opt(verify_token.as_deref(), &tenant_id, VERIFY_TOKEN_PURPOSE, &mut report);
        let sealed_app = seal_opt(app_secret.as_deref(), &tenant_id, APP_SECRET_PURPOSE, &mut report);

        match client
            .execute(
                "UPDATE whatsapp_channels SET
                    wa_token_enc     = COALESCE(wa_token_enc, $2),
                    verify_token_enc = COALESCE(verify_token_enc, $3),
                    app_secret_enc   = COALESCE(app_secret_enc, $4),
                    wa_token         = CASE WHEN $2 IS NULL THEN wa_token END,
                    verify_token     = CASE WHEN $3 IS NULL THEN verify_token END,
                    app_secret       = CASE WHEN $4 IS NULL THEN app_secret END
                 WHERE phone_number_id = $1",
                &[&phone_number_id, &sealed_wa, &sealed_verify, &sealed_app],
            )
            .await
        {
            Ok(_) => {
                report.whatsapp_secrets += [&sealed_wa, &sealed_verify, &sealed_app]
                    .iter()
                    .filter(|s| s.is_some())
                    .count();
            }
            Err(e) => report
                .failures
                .push(format!("whatsapp_channels {}: {}", tenant_id, e)),
        }
    }

    // ── messaging_channels.webhook_secret ────────────────────────────────────
    let rows = client
        .query(
            "SELECT channel, channel_ref, tenant_id, webhook_secret
             FROM messaging_channels WHERE webhook_secret IS NOT NULL",
            &[],
        )
        .await
        .to_store_error()?;

    for row in rows {
        let channel: String = row.get(0);
        let channel_ref: String = row.get(1);
        let tenant_id: String = row.get(2);
        let secret: String = row.get(3);

        // Teams stores an empty value, which holds nothing to protect.
        let sealed = if secret.is_empty() {
            None
        } else {
            match seal_opt(Some(&secret), &tenant_id, WEBHOOK_SECRET_PURPOSE, &mut report) {
                Some(bytes) => Some(bytes),
                None => continue,
            }
        };

        match client
            .execute(
                "UPDATE messaging_channels SET
                    webhook_secret_enc = COALESCE(webhook_secret_enc, $3),
                    webhook_secret     = NULL
                 WHERE channel = $1 AND channel_ref = $2",
                &[&channel, &channel_ref, &sealed],
            )
            .await
        {
            Ok(_) => report.webhook_secrets += sealed.is_some() as usize,
            Err(e) => report
                .failures
                .push(format!("messaging_channels {}: {}", tenant_id, e)),
        }
    }

    app_log!(
        info,
        bearer = report.bearer_tokens,
        headers = report.custom_headers,
        service_accounts = report.service_accounts,
        static_headers = report.static_headers,
        whatsapp_secrets = report.whatsapp_secrets,
        webhook_secrets = report.webhook_secrets,
        failures = report.failures.len(),
        "Legacy secret encryption backfill complete"
    );

    Ok(report)
}

fn seal_opt(
    value: Option<&str>,
    tenant_id: &str,
    purpose: &str,
    report: &mut BackfillReport,
) -> Option<Vec<u8>> {
    let value = value?;
    match secret_box::seal(value, &SecretContext { tenant_id, purpose }) {
        Ok(bytes) => Some(bytes),
        Err(e) => {
            // The message names the tenant and the column, never the secret.
            report.failures.push(format!("{} {}: {}", tenant_id, purpose, e));
            None
        }
    }
}
