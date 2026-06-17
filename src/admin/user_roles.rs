// src/admin/user_roles.rs
//
// Platform-level user role management.
//
// Internal (X-Internal-Secret):
//   GET    /api/internal/user-role/{email}          — lookup role
//   PUT    /api/internal/user-role                   — set role (super_admin only)
//   GET    /api/internal/user-roles                  — list all roles
//   DELETE /api/internal/user-role/{email}           — remove role

use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::Deserialize;
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

const VALID_ROLES: &[&str] = &["super_admin", "admin", "user"];

// GET /api/internal/user-role/{email}
pub async fn get_user_role(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let email = path.into_inner().to_lowercase();

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    match client.query_opt(
        "SELECT role, granted_by, granted_at FROM user_roles WHERE email = $1",
        &[&email],
    ).await {
        Ok(Some(row)) => {
            let role: String = row.get(0);
            let granted_by: Option<String> = row.get(1);
            let granted_at: chrono::DateTime<chrono::Utc> = row.get(2);
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "email": email,
                "role": role,
                "granted_by": granted_by,
                "granted_at": granted_at.to_rfc3339(),
            }))
        }
        Ok(None) => {
            // No explicit role — default to "user"
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "email": email,
                "role": "user",
            }))
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to get user role");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": e.to_string()}))
        }
    }
}

// PUT /api/internal/user-role
#[derive(Deserialize)]
pub struct SetRoleRequest {
    pub email: String,
    pub role: String,
    pub granted_by: String,
}

pub async fn set_user_role(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SetRoleRequest>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let email = body.email.to_lowercase();
    let role = body.role.to_lowercase();

    if !VALID_ROLES.contains(&role.as_str()) {
        return HttpResponse::BadRequest()
            .json(serde_json::json!({
                "success": false,
                "error": format!("Invalid role '{}'. Valid: {:?}", role, VALID_ROLES),
            }));
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    match client.execute(
        "INSERT INTO user_roles (email, role, granted_by, granted_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (email)
         DO UPDATE SET role = EXCLUDED.role, granted_by = EXCLUDED.granted_by, granted_at = NOW()",
        &[&email, &role, &body.granted_by],
    ).await {
        Ok(_) => {
            app_log!(info, email = %email, role = %role, granted_by = %body.granted_by, "User role updated");
            HttpResponse::Ok().json(serde_json::json!({"success": true}))
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to set user role");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": e.to_string()}))
        }
    }
}

// GET /api/internal/user-roles
pub async fn list_user_roles(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    match client.query(
        "SELECT email, role, granted_by, granted_at FROM user_roles ORDER BY granted_at DESC",
        &[],
    ).await {
        Ok(rows) => {
            let roles: Vec<serde_json::Value> = rows.iter().map(|row| {
                let granted_at: chrono::DateTime<chrono::Utc> = row.get(3);
                serde_json::json!({
                    "email": row.get::<_, String>(0),
                    "role": row.get::<_, String>(1),
                    "granted_by": row.get::<_, Option<String>>(2),
                    "granted_at": granted_at.to_rfc3339(),
                })
            }).collect();
            HttpResponse::Ok().json(serde_json::json!({"success": true, "roles": roles}))
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to list user roles");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": e.to_string()}))
        }
    }
}

// DELETE /api/internal/user-role/{email}
pub async fn delete_user_role(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
) -> impl Responder {
    if !check_internal_secret(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }

    let email = path.into_inner().to_lowercase();

    let client = match store.get_admin_conn().await {
        Ok(c) => c,
        Err(_) => return HttpResponse::InternalServerError()
            .json(serde_json::json!({"success": false, "error": "DB error"})),
    };

    match client.execute(
        "DELETE FROM user_roles WHERE email = $1",
        &[&email],
    ).await {
        Ok(count) => {
            app_log!(info, email = %email, "Deleted user role (rows={})", count);
            HttpResponse::Ok().json(serde_json::json!({"success": true, "deleted": count}))
        }
        Err(e) => {
            app_log!(error, error = %e, "Failed to delete user role");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "error": e.to_string()}))
        }
    }
}
