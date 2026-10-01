// src/api/tenant_members.rs
//
// Workspace membership over HTTP. `email` is always the caller, bound by the
// gateway to the verified identity; every other address is the person acted on.
//
//   GET  /api/user/tenant/members?email=           members + pending invitations
//   POST /api/user/tenant/invites                  { email, invitee, role }
//   POST /api/user/tenant/invites/cancel           { email, invitee }
//   PUT  /api/user/tenant/members/role             { email, member, role }
//   POST /api/user/tenant/members/remove           { email, member }
//   POST /api/user/tenant/leave                    { email }
//
// Internal (X-Internal-Secret), for the platform admin panel:
//   POST /api/internal/tenants/{tenant_id}/members { email, role, admin_email }

use crate::app_log;
use crate::endpoint_store::tenant_members::{self as tm, InviteOutcome, MemberError};
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpRequest, HttpResponse};
use serde::Deserialize;
use std::sync::Arc;

fn respond(result: Result<serde_json::Value, MemberError>) -> HttpResponse {
    match result {
        Ok(body) => HttpResponse::Ok().json(body),
        Err(MemberError::Forbidden(m)) => HttpResponse::Forbidden()
            .json(serde_json::json!({"success": false, "message": m})),
        Err(MemberError::Invalid(m)) => HttpResponse::BadRequest()
            .json(serde_json::json!({"success": false, "message": m})),
        Err(MemberError::Conflict(m)) => HttpResponse::Conflict()
            .json(serde_json::json!({"success": false, "message": m})),
        Err(MemberError::Store(e)) => {
            app_log!(error, error = %e, "Workspace membership request failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"success": false, "message": "Could not update the workspace"}))
        }
    }
}

fn outcome_json(outcome: InviteOutcome) -> serde_json::Value {
    serde_json::json!({
        "success": true,
        "outcome": match outcome {
            InviteOutcome::Added => "added",
            InviteOutcome::Invited => "invited",
        },
    })
}

#[derive(Deserialize)]
pub struct CallerQuery {
    pub email: String,
}

pub async fn list_members(
    store: web::Data<Arc<EndpointStore>>,
    q: web::Query<CallerQuery>,
) -> HttpResponse {
    respond(
        tm::list_members(&store, &q.email.to_lowercase())
            .await
            .map(|mut v| {
                v["success"] = serde_json::Value::Bool(true);
                v
            }),
    )
}

#[derive(Deserialize)]
pub struct InviteBody {
    pub email: String,
    pub invitee: String,
    pub role: String,
}

pub async fn invite(
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<InviteBody>,
) -> HttpResponse {
    respond(
        tm::invite(store.get_ref(), &body.email.to_lowercase(), &body.invitee, &body.role)
            .await
            .map(outcome_json),
    )
}

#[derive(Deserialize)]
pub struct CancelInviteBody {
    pub email: String,
    pub invitee: String,
}

pub async fn cancel_invite(
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<CancelInviteBody>,
) -> HttpResponse {
    respond(
        tm::cancel_invite(&store, &body.email.to_lowercase(), &body.invitee)
            .await
            .map(|_| serde_json::json!({"success": true})),
    )
}

#[derive(Deserialize)]
pub struct RoleBody {
    pub email: String,
    pub member: String,
    pub role: String,
}

pub async fn change_role(
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<RoleBody>,
) -> HttpResponse {
    respond(
        tm::change_role(&store, &body.email.to_lowercase(), &body.member, &body.role)
            .await
            .map(|_| serde_json::json!({"success": true})),
    )
}

#[derive(Deserialize)]
pub struct MemberBody {
    pub email: String,
    pub member: String,
}

pub async fn remove_member(
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<MemberBody>,
) -> HttpResponse {
    respond(
        tm::remove_member(&store, &body.email.to_lowercase(), &body.member)
            .await
            .map(|_| serde_json::json!({"success": true})),
    )
}

#[derive(Deserialize)]
pub struct LeaveBody {
    pub email: String,
}

pub async fn leave(
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<LeaveBody>,
) -> HttpResponse {
    respond(
        tm::leave(&store, &body.email.to_lowercase())
            .await
            .map(|_| serde_json::json!({"success": true})),
    )
}

// ── Internal: platform admin ──────────────────────────────────────────────────

fn internal_secret_ok(req: &HttpRequest) -> bool {
    match std::env::var("API0_INTERNAL_SECRET") {
        Ok(expected) if !expected.is_empty() => req
            .headers()
            .get("X-Internal-Secret")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == expected)
            .unwrap_or(false),
        _ => false,
    }
}

#[derive(Deserialize)]
pub struct AdminAddBody {
    pub email: String,
    pub role: String,
    /// The platform admin doing it, for the log and the invitation email.
    pub admin_email: String,
}

pub async fn admin_add_member(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    path: web::Path<String>,
    body: web::Json<AdminAddBody>,
) -> HttpResponse {
    if !internal_secret_ok(&req) {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"success": false, "error": "Unauthorized"}));
    }
    respond(
        tm::admin_add_member(store.get_ref(), &path.into_inner(), &body.email, &body.role, &body.admin_email)
            .await
            .map(outcome_json),
    )
}
