use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    error::ErrorUnauthorized,
    Error, HttpMessage,
};
use crate::endpoint_store::EndpointStore;
use futures::future::{ready, LocalBoxFuture, Ready};
use std::rc::Rc;
use std::sync::Arc;
// use std::task::{Context, Poll};

// API Key authentication middleware
pub struct ApiKeyAuth {
    store: Arc<EndpointStore>,
}

impl ApiKeyAuth {
    pub fn new(store: Arc<EndpointStore>) -> Self {
        Self { store }
    }
}

impl<S, B> Transform<S, ServiceRequest> for ApiKeyAuth
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = ApiKeyAuthMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(ApiKeyAuthMiddleware {
            service: Rc::new(service),
            store: self.store.clone(),
        }))
    }
}

pub struct ApiKeyAuthMiddleware<S> {
    service: Rc<S>,
    store: Arc<EndpointStore>,
}

impl<S, B> Service<ServiceRequest> for ApiKeyAuthMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();
        let store = self.store.clone();

        Box::pin(async move {
            // Skip authentication for non-API paths
            let path = req.path();
            if !path.starts_with("/api/") || path == "/api/health" {
                return service.call(req).await;
            }

            // Skip authentication for key management endpoints
            if path.contains("/user/key") || path.contains("/user/preferences") {
                return service.call(req).await;
            }

            // Get API key from header
            if let Some(api_key) = req.headers().get("X-API-Key") {
                let api_key = api_key.to_str().unwrap_or("");
                
                // Validate the API key
                match store.validate_api_key(api_key).await {
                    Ok(Some(email)) => {
                        // Record API key usage
                        let _ = store.record_api_key_usage(&email).await;
                        
                        // Add the authenticated email to request extensions
                        req.extensions_mut().insert(email.clone());
                        
                        // Continue with the request
                        service.call(req).await
                    }
                    _ => {
                        // Invalid API key
                        tracing::warn!("Invalid API key provided");
                        Err(ErrorUnauthorized("Invalid API key"))
                    }
                }
            } else {
                // Missing API key
                tracing::warn!("Missing API key");
                Err(ErrorUnauthorized("Missing API key"))
            }
        })
    }
}
