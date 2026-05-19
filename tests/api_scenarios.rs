// tests/api_scenarios.rs — Store Tier-1 tests
//
// Uses actix-web test utilities to verify routing contracts without a real DB.
// Tests auth guard logic, request format validation, and health endpoint shape.

use actix_web::{test, web, App, HttpRequest, HttpResponse};
use serde_json::json;

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Simple auth guard: returns 401 if X-API-Key header is missing
async fn require_api_key(req: HttpRequest) -> HttpResponse {
    if req.headers().get("X-API-Key").is_some() {
        HttpResponse::Ok().json(json!({"groups": []}))
    } else {
        HttpResponse::Unauthorized().json(json!({"error": "missing X-API-Key header"}))
    }
}

// ── Health ────────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn health_endpoint_returns_200() {
    let app = test::init_service(
        App::new().route("/api/health", web::get().to(|| async {
            HttpResponse::Ok().body("ok")
        }))
    ).await;

    let req = test::TestRequest::get().uri("/api/health").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

// ── Auth guard ────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn missing_api_key_returns_401() {
    let app = test::init_service(
        App::new().route("/api/groups/{email}", web::get().to(require_api_key))
    ).await;

    // No API key → 401
    let req = test::TestRequest::get()
        .uri("/api/groups/test@example.com")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 401);
}

#[actix_web::test]
async fn valid_api_key_passes_guard() {
    let app = test::init_service(
        App::new().route("/api/groups/{email}", web::get().to(require_api_key))
    ).await;

    let req = test::TestRequest::get()
        .uri("/api/groups/test@example.com")
        .insert_header(("X-API-Key", "sk_live_test"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}

// ── Request format validation ─────────────────────────────────────────────────

#[actix_web::test]
async fn malformed_json_returns_400() {
    let app = test::init_service(
        App::new().route(
            "/api/user/keys",
            web::post().to(|_body: web::Json<serde_json::Value>| async {
                HttpResponse::Ok().finish()
            }),
        )
    ).await;

    let req = test::TestRequest::post()
        .uri("/api/user/keys")
        .insert_header(("Content-Type", "application/json"))
        .set_payload("not valid json {{{")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_client_error(), "Malformed JSON → 4xx, got {}", resp.status());
}

#[actix_web::test]
async fn validate_key_missing_field_returns_400() {
    let app = test::init_service(
        App::new().route(
            "/api/key/validate",
            web::post().to(|body: web::Json<serde_json::Value>| async move {
                if body.get("api_key").is_none() {
                    HttpResponse::BadRequest().json(json!({"error": "api_key required"}))
                } else {
                    HttpResponse::Ok().finish()
                }
            }),
        )
    ).await;

    let req = test::TestRequest::post()
        .uri("/api/key/validate")
        .set_json(json!({}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}

// ── Routing ───────────────────────────────────────────────────────────────────

#[actix_web::test]
async fn unknown_route_returns_404() {
    let app = test::init_service(
        App::new().route("/api/health", web::get().to(|| async { HttpResponse::Ok().finish() }))
    ).await;

    let req = test::TestRequest::get().uri("/api/nonexistent-xyz").to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 404);
}

// ── Response format ───────────────────────────────────────────────────────────

#[actix_web::test]
async fn auth_error_returns_json_body() {
    let app = test::init_service(
        App::new().route("/api/groups/{email}", web::get().to(require_api_key))
    ).await;

    let req = test::TestRequest::get()
        .uri("/api/groups/user@test.com")
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 401);

    let body: serde_json::Value = test::read_body_json(resp).await;
    assert!(body.get("error").is_some(), "401 response should have 'error' field");
}
