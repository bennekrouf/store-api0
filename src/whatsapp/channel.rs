// src/whatsapp/channel.rs
//
// WhatsApp channel management — one channel per tenant.
//
// Gateway-proxied (no auth on store side — gateway has already verified Firebase JWT):
//   POST   /api/whatsapp/channel          body: { email, phone_number_id, wa_token, verify_token, system_prompt?, app_secret? }
//   GET    /api/whatsapp/channel/{email}
//   DELETE /api/whatsapp/channel/{email}
//
// Bridge-internal (X-Internal-Secret):
//   GET    /api/internal/whatsapp/channel/{phone_number_id}
//   GET    /api/internal/whatsapp/channel/by-tenant/{tenant_id}

use crate::app_log;
use crate::endpoint_store::downstream_auth_management::{seal_for, unseal_or_plain};
use crate::endpoint_store::EndpointStore;
use crate::endpoint_store::tenant_management::get_default_tenant;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

// What each secret is sealed for: bound into the ciphertext, so one cannot be
// opened as another.
pub const WA_TOKEN_PURPOSE: &str = "whatsapp_wa_token";
pub const VERIFY_TOKEN_PURPOSE: &str = "whatsapp_verify_token";
pub const APP_SECRET_PURPOSE: &str = "whatsapp_app_secret";

/// Every bridge lookup reads the same columns. The plaintext ones stay until the
/// legacy backfill has emptied them (endpoint_store::encrypt_legacy).
const CHANNEL_SELECT: &str = "SELECT tenant_id, phone_number_id, system_prompt,
        wa_token, wa_token_enc, verify_token, verify_token_enc, app_secret, app_secret_enc
   FROM whatsapp_channels WHERE ";

struct ChannelSecrets {
    wa_token: String,
    verify_token: String,
    app_secret: Option<String>,
}

/// None when a stored secret cannot be read. A sealed App Secret that will not
/// open must not look like "none stored", which would fall back to the platform's.
fn open_secrets(row: &tokio_postgres::Row, tenant_id: &str) -> Option<ChannelSecrets> {
    let wa_token = unseal_or_plain(row.get(4), row.get(3), tenant_id, WA_TOKEN_PURPOSE)?;
    let verify_token = unseal_or_plain(row.get(6), row.get(5), tenant_id, VERIFY_TOKEN_PURPOSE)?;
    let sealed_app_secret: Option<Vec<u8>> = row.get(8);
    let had_sealed_app_secret = sealed_app_secret.is_some();
    let app_secret = unseal_or_plain(sealed_app_secret, row.get(7), tenant_id, APP_SECRET_PURPOSE);
    if had_sealed_app_secret && app_secret.is_none() {
        return None;
    }
    Some(ChannelSecrets { wa_token, verify_token, app_secret })
}

fn channel_response(row: &tokio_postgres::Row) -> HttpResponse {
    let tenant_id: &str = row.get(0);
    match open_secrets(row, tenant_id) {
        Some(secrets) => HttpResponse::Ok().json(serde_json::json!({
            "success":         true,
            "tenant_id":       tenant_id,
            "phone_number_id": row.get::<_, &str>(1),
            "wa_token":        secrets.wa_token,
            "verify_token":    secrets.verify_token,
            "system_prompt":   row.get::<_, &str>(2),
            "app_secret":      secrets.app_secret,
        })),
        None => HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "Channel secrets could not be read"})),
    }
}

fn check_internal_secret(req: &HttpRequest) -> bool {
    let expected = match std::env::var("API0_INTERNAL_SECRET") {
        Ok(s) if !s.is_empty() => s,
        _ => return false,
    };
    req.headers()
        .get("X-Internal-Secret")
        .and_then(|v| v.to_str().ok())
        .map(|v| v == expected)
        .unwrap_or(false)
}

#[derive(Deserialize)]
pub struct RegisterChannelRequest {
    pub email: String,
    pub phone_number_id: String,
    pub wa_token: String,
    pub verify_token: String,
    pub system_prompt: Option<String>,
    /// The Meta app's App Secret, which signs every webhook POST. Absent or blank
    /// keeps the stored one: the form cannot show it back, so a person updating
    /// their persona must not have to paste the secret again to avoid wiping it.
    pub app_secret: Option<String>,
}

// POST /api/whatsapp/channel
pub async fn register_channel(
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<RegisterChannelRequest>,
) -> impl Responder {
    let tenant = match get_default_tenant(&store, &body.email).await {
        Ok(t) => t,
        Err(e) => {
            app_log!(error, email = %body.email, error = %e, "Tenant resolution failed");
            return HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": "Tenant resolution failed"}));
        }
    };

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    let system_prompt = body.system_prompt.as_deref().unwrap_or("");
    let app_secret = body
        .app_secret
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let sealed = (|| -> Result<_, crate::endpoint_store::StoreError> {
        Ok((
            seal_for(&body.wa_token, &tenant.id, WA_TOKEN_PURPOSE)?,
            seal_for(&body.verify_token, &tenant.id, VERIFY_TOKEN_PURPOSE)?,
            app_secret.map(|s| seal_for(s, &tenant.id, APP_SECRET_PURPOSE)).transpose()?,
        ))
    })();
    let (wa_token_enc, verify_token_enc, app_secret_enc) = match sealed {
        Ok(s) => s,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "The channel secrets could not be protected"})),
    };

    // A kept App Secret keeps its current form, plaintext included, until the
    // backfill reaches it: blanking it here would lose it.
    match client.execute(
        "INSERT INTO whatsapp_channels
            (phone_number_id, tenant_id, system_prompt, wa_token_enc, verify_token_enc, app_secret_enc)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (tenant_id) DO UPDATE
           SET phone_number_id  = EXCLUDED.phone_number_id,
               system_prompt    = EXCLUDED.system_prompt,
               wa_token         = NULL,
               wa_token_enc     = EXCLUDED.wa_token_enc,
               verify_token     = NULL,
               verify_token_enc = EXCLUDED.verify_token_enc,
               app_secret       = CASE WHEN EXCLUDED.app_secret_enc IS NULL
                                       THEN whatsapp_channels.app_secret END,
               app_secret_enc   = COALESCE(EXCLUDED.app_secret_enc, whatsapp_channels.app_secret_enc)",
        &[&body.phone_number_id, &tenant.id, &system_prompt, &wa_token_enc, &verify_token_enc, &app_secret_enc],
    ).await {
        Ok(_) => {
            app_log!(info, tenant_id = %tenant.id, "WA channel registered");
            HttpResponse::Ok().json(serde_json::json!({"success": true}))
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": e.to_string()})),
    }
}

// GET /api/whatsapp/channel/{email}
pub async fn get_channel(
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    let email = path.into_inner();
    let tenant = match get_default_tenant(&store, &email).await {
        Ok(t) => t,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "Tenant error"})),
    };

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    match client.query_opt(
        "SELECT phone_number_id, system_prompt, created_at,
                (app_secret IS NOT NULL OR app_secret_enc IS NOT NULL), label
           FROM whatsapp_channels WHERE tenant_id = $1",
        &[&tenant.id],
    ).await {
        // tenant_id travels at the top level, channel or not: the gateway builds
        // the webhook URL from it, so the URL the guide shows and the URL the
        // test probes come from one place.
        Ok(Some(row)) => {
            let created_at: chrono::DateTime<chrono::Utc> = row.get(2);
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "tenant_id": tenant.id,
                "channel": {
                    "phone_number_id": row.get::<_, &str>(0),
                    "system_prompt":   row.get::<_, &str>(1),
                    "created_at":      created_at.to_rfc3339(),
                    // Whether one is stored — never the secret.
                    "has_app_secret":  row.get::<_, bool>(3),
                    "label":           row.get::<_, &str>(4),
                }
            }))
        }
        Ok(None) => HttpResponse::Ok()
            .json(serde_json::json!({"success": true, "tenant_id": tenant.id, "channel": null})),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": e.to_string()})),
    }
}

// DELETE /api/whatsapp/channel/{email}
pub async fn delete_channel(
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    let email = path.into_inner();
    let tenant = match get_default_tenant(&store, &email).await {
        Ok(t) => t,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "Tenant error"})),
    };

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    match client.execute(
        "DELETE FROM whatsapp_channels WHERE tenant_id = $1",
        &[&tenant.id],
    ).await {
        Ok(_) => HttpResponse::Ok().json(serde_json::json!({"success": true})),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": e.to_string()})),
    }
}

// GET /api/internal/whatsapp/channel/{phone_number_id}
pub async fn lookup_channel_internal(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }
    let phone_number_id = path.into_inner();
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };
    match client.query_opt(
        format!("{CHANNEL_SELECT}phone_number_id = $1").as_str(),
        &[&phone_number_id],
    ).await {
        Ok(Some(row)) => channel_response(&row),
        Ok(None) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "Channel not found"})),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": e.to_string()})),
    }
}

// GET /api/internal/whatsapp/channel/by-tenant/{tenant_id}
pub async fn lookup_channel_by_tenant_internal(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }
    let tenant_id = path.into_inner();
    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };
    match client.query_opt(
        format!("{CHANNEL_SELECT}tenant_id = $1").as_str(),
        &[&tenant_id],
    ).await {
        Ok(Some(row)) => channel_response(&row),
        Ok(None) => HttpResponse::NotFound()
            .json(serde_json::json!({"success": false, "error": "Channel not found"})),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": e.to_string()})),
    }
}

/// Against a real database — see tenant_members::db_tests for how to run. Also
/// needs API0_ENCRYPTION_KEY.
#[cfg(test)]
mod db_tests {
    use super::*;
    use crate::endpoint_store::channel_identities::{create_link_code, redeem_link_code};
    use crate::endpoint_store::encrypt_legacy::encrypt_legacy_secrets;
    use crate::endpoint_store::messaging_channels::{channel_for_bridge, register_channel as register_bot, RegisterChannel};

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL and API0_ENCRYPTION_KEY"]
    async fn channel_secrets_are_sealed_and_older_rows_keep_working() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = Arc::new(EndpointStore::new(&url).await.expect("store"));
        EndpointStore::new(&url).await.expect("the schema applies twice");
        let run = uuid::Uuid::new_v4().simple().to_string()[..8].to_string();
        let email = format!("channels-{run}@example.com");
        let tenant = get_default_tenant(&store, &email).await.unwrap();
        let t = tenant.id.as_str();
        let c = store.get_admin_conn().await.unwrap();
        let phone = format!("p-{run}");

        let read = |c_row: tokio_postgres::Row| {
            let s = open_secrets(&c_row, t).expect("secrets open");
            (s.wa_token, s.verify_token, s.app_secret)
        };
        let select = format!("{CHANNEL_SELECT}tenant_id = $1");
        let plain = "SELECT wa_token, verify_token, app_secret FROM whatsapp_channels WHERE tenant_id = $1";

        // A row from before sealing still reads.
        c.execute(
            "INSERT INTO whatsapp_channels (phone_number_id, tenant_id, wa_token, verify_token, app_secret)
             VALUES ($1, $2, 'old-wa', 'old-verify', 'old-app')",
            &[&phone, &t],
        ).await.unwrap();
        assert_eq!(
            read(c.query_one(select.as_str(), &[&t]).await.unwrap()),
            ("old-wa".into(), "old-verify".into(), Some("old-app".into()))
        );

        // The backfill seals it and empties the plaintext; a second run does nothing.
        let first = encrypt_legacy_secrets(&store).await.unwrap();
        assert!(first.failures.is_empty(), "{:?}", first.failures);
        assert!(first.whatsapp_secrets >= 3);
        let row = c.query_one(plain, &[&t]).await.unwrap();
        for i in 0..3 {
            assert_eq!(row.get::<_, Option<String>>(i), None);
        }
        assert_eq!(
            read(c.query_one(select.as_str(), &[&t]).await.unwrap()),
            ("old-wa".into(), "old-verify".into(), Some("old-app".into()))
        );
        assert_eq!(encrypt_legacy_secrets(&store).await.unwrap().whatsapp_secrets, 0);

        // Saving without an App Secret keeps the stored one and writes no plaintext.
        let save = |wa: &str, app: Option<&str>| RegisterChannelRequest {
            email: email.clone(),
            phone_number_id: phone.clone(),
            wa_token: wa.into(),
            verify_token: "new-verify".into(),
            system_prompt: None,
            app_secret: app.map(str::to_string),
        };
        let _ = register_channel(web::Data::new(store.clone()), web::Json(save("new-wa", None))).await;
        assert_eq!(
            read(c.query_one(select.as_str(), &[&t]).await.unwrap()),
            ("new-wa".into(), "new-verify".into(), Some("old-app".into()))
        );
        let row = c.query_one(plain, &[&t]).await.unwrap();
        for i in 0..3 {
            assert_eq!(row.get::<_, Option<String>>(i), None);
        }

        // A plaintext App Secret survives a save until it is replaced or sealed.
        c.execute(
            "UPDATE whatsapp_channels SET app_secret = 'legacy-app', app_secret_enc = NULL WHERE tenant_id = $1",
            &[&t],
        ).await.unwrap();
        let _ = register_channel(web::Data::new(store.clone()), web::Json(save("new-wa", None))).await;
        assert_eq!(read(c.query_one(select.as_str(), &[&t]).await.unwrap()).2, Some("legacy-app".into()));
        let _ = register_channel(web::Data::new(store.clone()), web::Json(save("new-wa", Some("newest-app")))).await;
        assert_eq!(read(c.query_one(select.as_str(), &[&t]).await.unwrap()).2, Some("newest-app".into()));
        assert_eq!(c.query_one(plain, &[&t]).await.unwrap().get::<_, Option<String>>(2), None);

        // Bot channels: sealed, Teams' empty value stays empty, older rows migrate.
        let bot_ref = format!("bot-{run}");
        register_bot(&store, t, RegisterChannel {
            channel: "telegram", channel_ref: &bot_ref, credential: "cred", display_ref: "bot",
            webhook_secret: "hook", system_prompt: "",
        }).await.unwrap();
        register_bot(&store, t, RegisterChannel {
            channel: "teams", channel_ref: &bot_ref, credential: "cred", display_ref: "bot",
            webhook_secret: "", system_prompt: "",
        }).await.unwrap();
        assert_eq!(channel_for_bridge(&store, "telegram", &bot_ref).await.unwrap().unwrap().webhook_secret, "hook");
        assert_eq!(channel_for_bridge(&store, "teams", &bot_ref).await.unwrap().unwrap().webhook_secret, "");
        let stored: Option<String> = c.query_one(
            "SELECT webhook_secret FROM messaging_channels WHERE channel = 'telegram' AND channel_ref = $1",
            &[&bot_ref],
        ).await.unwrap().get(0);
        assert_eq!(stored, None);

        c.execute(
            "UPDATE messaging_channels SET webhook_secret = 'old-hook', webhook_secret_enc = NULL
              WHERE channel = 'telegram' AND channel_ref = $1",
            &[&bot_ref],
        ).await.unwrap();
        assert_eq!(channel_for_bridge(&store, "telegram", &bot_ref).await.unwrap().unwrap().webhook_secret, "old-hook");
        encrypt_legacy_secrets(&store).await.unwrap();
        assert_eq!(channel_for_bridge(&store, "telegram", &bot_ref).await.unwrap().unwrap().webhook_secret, "old-hook");

        // Link codes: the stored value is not the code, and the code still redeems once.
        let (code, _) = create_link_code(&store, &email).await.unwrap();
        let stored: String = c.query_one(
            "SELECT code FROM channel_link_codes WHERE user_email = $1", &[&email],
        ).await.unwrap().get(0);
        assert_ne!(stored, code);
        let linked = redeem_link_code(&store, "teams", &run, t, &code.to_lowercase()).await.unwrap();
        assert_eq!(linked.user_email, email);
        assert!(redeem_link_code(&store, "teams", &run, t, &code).await.is_err());

        // The unused columns are gone.
        let gone: i64 = c.query_one(
            "SELECT count(*) FROM information_schema.columns
              WHERE (table_name, column_name) IN (('domains', 'verification_token'), ('tenants', 'firebase_api_key'))",
            &[],
        ).await.unwrap().get(0);
        assert_eq!(gone, 0);
    }
}
