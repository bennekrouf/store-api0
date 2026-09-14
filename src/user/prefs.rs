// src/user/prefs.rs
//
// Per-user, per-front-end preferences document. Gateway-proxied: the gateway
// verifies the Firebase token and derives both path segments from it, so a
// request here already names the right owner. Internal (X-Internal-Secret).
//
//   GET   /api/internal/user-prefs/{app}/{email}   — read (empty object if none)
//   PUT   /api/internal/user-prefs/{app}/{email}   — replace the document
//   PATCH /api/internal/user-prefs/{app}/{email}   — shallow-merge into it

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use std::sync::Arc;

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

fn unauthorized() -> HttpResponse {
    HttpResponse::Unauthorized().json(serde_json::json!({"success": false, "error": "Unauthorized"}))
}

fn db_error() -> HttpResponse {
    HttpResponse::InternalServerError().json(serde_json::json!({"success": false, "error": "DB error"}))
}

fn ok(prefs: serde_json::Value, updated_at: Option<chrono::DateTime<chrono::Utc>>) -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "prefs": prefs,
        "updated_at": updated_at.map(|t| t.to_rfc3339()),
    }))
}

fn keys(path: web::Path<(String, String)>) -> (String, String) {
    let (app, email) = path.into_inner();
    (app, email.to_lowercase())
}

// GET /api/internal/user-prefs/{app}/{email}
pub async fn get_user_prefs(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return unauthorized();
    }
    let (app, email) = keys(path);

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return db_error(),
    };

    match client
        .query_opt(
            "SELECT prefs, updated_at FROM app_user_prefs WHERE app = $1 AND email = $2",
            &[&app, &email],
        )
        .await
    {
        Ok(Some(row)) => ok(row.get(0), Some(row.get(1))),
        // No document yet reads as an empty one; the client never has to
        // special-case first use.
        Ok(None) => ok(serde_json::json!({}), None),
        Err(e) => {
            app_log!(error, error = %e, app = %app, email = %email, "Failed to get user prefs");
            db_error()
        }
    }
}

// PUT /api/internal/user-prefs/{app}/{email}
pub async fn put_user_prefs(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
    body: web::Json<serde_json::Value>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return unauthorized();
    }
    let (app, email) = keys(path);
    let prefs = body.into_inner();
    if !prefs.is_object() {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"success": false, "error": "Preferences must be a JSON object"}));
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return db_error(),
    };

    match client
        .query_one(
            "INSERT INTO app_user_prefs (app, email, prefs, updated_at)
             VALUES ($1, $2, $3, NOW())
             ON CONFLICT (app, email)
             DO UPDATE SET prefs = EXCLUDED.prefs, updated_at = NOW()
             RETURNING prefs, updated_at",
            &[&app, &email, &prefs],
        )
        .await
    {
        Ok(row) => {
            app_log!(info, app = %app, email = %email, "User prefs replaced");
            ok(row.get(0), Some(row.get(1)))
        }
        Err(e) => {
            app_log!(error, error = %e, app = %app, email = %email, "Failed to put user prefs");
            db_error()
        }
    }
}

// PATCH /api/internal/user-prefs/{app}/{email}
//
// Shallow merge (jsonb ||): top-level keys in the body overwrite or add,
// everything else is kept. Deleting a key is a PUT of the full document.
pub async fn patch_user_prefs(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<(String, String)>,
    body: web::Json<serde_json::Value>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return unauthorized();
    }
    let (app, email) = keys(path);
    let patch = body.into_inner();
    if !patch.is_object() {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"success": false, "error": "Preferences must be a JSON object"}));
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return db_error(),
    };

    match client
        .query_one(
            "INSERT INTO app_user_prefs (app, email, prefs, updated_at)
             VALUES ($1, $2, $3, NOW())
             ON CONFLICT (app, email)
             DO UPDATE SET prefs = app_user_prefs.prefs || EXCLUDED.prefs, updated_at = NOW()
             RETURNING prefs, updated_at",
            &[&app, &email, &patch],
        )
        .await
    {
        Ok(row) => {
            app_log!(info, app = %app, email = %email, "User prefs merged");
            ok(row.get(0), Some(row.get(1)))
        }
        Err(e) => {
            app_log!(error, error = %e, app = %app, email = %email, "Failed to patch user prefs");
            db_error()
        }
    }
}
