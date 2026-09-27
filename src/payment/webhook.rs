// src/payment/webhook.rs
//
// Stripe webhooks. Stripe calls the gateway (POST /api/stripe/webhook), which
// has no Stripe secrets and passes the raw body and Stripe-Signature header on
// unchanged, here, over the internal route:
//
//     POST /api/internal/stripe/webhook   { "payload": "<raw body>", "signature": "<header>" }
//
// The signature is checked against STRIPE_WEBHOOK_SECRET. Then the object the
// event is about is fetched from Stripe again rather than parsed out of the
// event: its current state is what counts, and async-stripe parses what its own
// API version returns, whatever version the webhook endpoint was created with.
//
// Handled:
//   payment_intent.succeeded    → api0 credits (payment::topup)
//   checkout.session.completed  → desktop app licence (payment::license)
//   charge.refunded             → licence revoked, when fully refunded
// Anything else, or an event for a payment that is neither, is acknowledged
// with 200 and ignored. 500 only when Stripe should retry.

use crate::app_log;
use crate::email::{send_async, EmailKind};
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::license::{self, IssueError};
use crate::payment::service::PaymentService;
use crate::payment::topup::{self, TopUp, TopUpError};
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use std::sync::Arc;

/// How old a signed event may be, as Stripe's own libraries allow.
const TOLERANCE_SECS: i64 = 300;

#[derive(Deserialize)]
pub struct ForwardedWebhook {
    pub payload: String,
    pub signature: String,
}

/// Stripe's scheme: header `t=<unix>,v1=<hex hmac>[,v1=…]`, the HMAC-SHA256 of
/// `"<t>.<payload>"` keyed with the endpoint secret.
pub fn verify_signature(payload: &str, header: &str, secret: &str, now: i64) -> Result<(), &'static str> {
    let mut timestamp = None;
    let mut candidates = Vec::new();
    for part in header.split(',') {
        match part.trim().split_once('=') {
            Some(("t", t)) => timestamp = t.parse::<i64>().ok(),
            Some(("v1", sig)) => candidates.push(sig),
            _ => {}
        }
    }
    let t = timestamp.ok_or("no timestamp")?;
    if (now - t).abs() > TOLERANCE_SECS {
        return Err("timestamp outside tolerance");
    }
    for sig in candidates {
        let Ok(expected) = hex::decode(sig) else { continue };
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| "bad secret")?;
        mac.update(t.to_string().as_bytes());
        mac.update(b".");
        mac.update(payload.as_bytes());
        if mac.verify_slice(&expected).is_ok() {
            return Ok(());
        }
    }
    Err("no matching signature")
}

pub async fn stripe_webhook_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    payment_service: web::Data<Arc<PaymentService>>,
    body: web::Json<ForwardedWebhook>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let secret = match std::env::var("STRIPE_WEBHOOK_SECRET") {
        Ok(s) if !s.is_empty() => s,
        _ => {
            app_log!(error, "STRIPE_WEBHOOK_SECRET is unset — rejecting a Stripe webhook");
            return HttpResponse::InternalServerError().finish();
        }
    };
    if let Err(why) = verify_signature(&body.payload, &body.signature, &secret, chrono::Utc::now().timestamp()) {
        app_log!(warn, reason = why, "Rejected a Stripe webhook with a bad signature");
        return HttpResponse::BadRequest().json(serde_json::json!({ "success": false, "message": "Bad signature" }));
    }

    let event: serde_json::Value = match serde_json::from_str(&body.payload) {
        Ok(v) => v,
        Err(_) => return HttpResponse::BadRequest().finish(),
    };
    let kind = event["type"].as_str().unwrap_or_default();
    let object = &event["data"]["object"];
    let object_id = object["id"].as_str().unwrap_or_default();
    app_log!(info, event_id = %event["id"].as_str().unwrap_or_default(), kind = %kind, object_id = %object_id, "Stripe webhook");

    let retry = match kind {
        "payment_intent.succeeded" => on_payment_succeeded(&store, &payment_service, object_id).await,
        "checkout.session.completed" | "checkout.session.async_payment_succeeded" => {
            on_checkout_completed(&store, &payment_service, object_id).await
        }
        "charge.refunded" => on_charge_refunded(&store, object).await,
        _ => false,
    };

    if retry {
        HttpResponse::InternalServerError().json(serde_json::json!({ "success": false }))
    } else {
        HttpResponse::Ok().json(serde_json::json!({ "success": true }))
    }
}

/// Returns whether Stripe should retry.
async fn on_payment_succeeded(store: &Arc<EndpointStore>, stripe: &PaymentService, intent_id: &str) -> bool {
    let intent = match stripe.confirm_payment(intent_id).await {
        Ok(i) => i,
        Err(e) => {
            app_log!(error, error = %e, payment_intent_id = %intent_id, "Webhook: could not fetch the PaymentIntent");
            return true;
        }
    };
    match topup::credit_intent(store, &intent, None).await {
        Ok((email, TopUp::Credited { credits, new_balance })) => {
            send_async(Arc::clone(store), email, EmailKind::PaymentReceipt {
                amount_dollars: intent.amount_received as f64 / 100.0,
                credits_added: credits,
                new_balance,
            });
            false
        }
        Ok((_, TopUp::AlreadyCredited { .. })) => false,
        // Licence purchases and other sites' payments on the same account.
        Err(TopUpError::NotATopUp) => false,
        Err(e @ TopUpError::Store(_)) => {
            app_log!(error, error = %e, payment_intent_id = %intent_id, "Webhook: credits could not be added");
            true
        }
        Err(e) => {
            app_log!(warn, error = %e, payment_intent_id = %intent_id, "Webhook: payment not credited");
            false
        }
    }
}

async fn on_checkout_completed(store: &Arc<EndpointStore>, stripe: &PaymentService, session_id: &str) -> bool {
    let session = match stripe.retrieve_checkout_session(session_id).await {
        Ok(s) => s,
        Err(e) => {
            app_log!(error, error = %e, session_id = %session_id, "Webhook: could not fetch the Checkout session");
            return true;
        }
    };
    match license::issue_for_session(store, &session).await {
        Ok(_) => false,
        // Not a licence session, or paid by a method that settles later: its
        // checkout.session.async_payment_succeeded event will issue it.
        Err(IssueError::NotIssuable(_)) => false,
        Err(e) => {
            app_log!(error, error = %e, session_id = %session_id, "Webhook: licence could not be issued");
            true
        }
    }
}

async fn on_charge_refunded(store: &EndpointStore, charge: &serde_json::Value) -> bool {
    // A partial refund (a goodwill discount) leaves the licence alone.
    if charge["refunded"].as_bool() != Some(true) {
        return false;
    }
    let Some(intent_id) = charge["payment_intent"].as_str() else { return false };
    match license::revoke_for_payment(store, intent_id).await {
        Ok(0) => false,
        Ok(n) => {
            app_log!(info, payment_intent_id = %intent_id, revoked = n, "Licence revoked after a full refund");
            false
        }
        Err(e) => {
            app_log!(error, error = %e, payment_intent_id = %intent_id, "Webhook: licence could not be revoked");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(payload: &str, secret: &str, t: i64) -> String {
        let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{}.{}", t, payload).as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn accepts_a_valid_signature_among_several() {
        let header = format!("t=1000,v1={},v1={}", "00".repeat(32), sign("{}", "whsec_x", 1000));
        assert_eq!(verify_signature("{}", &header, "whsec_x", 1010), Ok(()));
    }

    #[test]
    fn rejects_a_changed_body_a_wrong_secret_and_an_old_event() {
        let header = format!("t=1000,v1={}", sign("{\"a\":1}", "whsec_x", 1000));
        assert!(verify_signature("{\"a\":2}", &header, "whsec_x", 1000).is_err());
        assert!(verify_signature("{\"a\":1}", &header, "whsec_y", 1000).is_err());
        assert!(verify_signature("{\"a\":1}", &header, "whsec_x", 1000 + TOLERANCE_SECS + 1).is_err());
        assert!(verify_signature("{\"a\":1}", "v1=abc", "whsec_x", 1000).is_err());
    }
}
