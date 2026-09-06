// src/endpoint_store/models.rs
//
// The catalog, tenant, key, credit and usage payloads now live in the shared
// `api0-types` crate so the store, the gateway and the SDK cannot drift apart.
// This module re-exports them under their historical paths.
//
// Some re-exports are unused inside the store itself but are part of the
// module's public surface (grpc/http handlers and tests reach for them).
#![allow(unused_imports)]

pub use api0_types::catalog::{
    generate_uuid, ApiGroup, ApiGroupWithEndpoints, ApiStorage, Endpoint, Parameter,
    UpdatePreferenceRequest, UserPreferences,
};
pub use api0_types::keys::{ApiKeyInfo, GenerateKeyRequest, KeyPreference, Tenant, TenantUser};
pub use api0_types::usage::{
    ApiUsageLog, CreditTransaction, LogApiUsageRequest, LogApiUsageResponse, TokenUsage,
    UpdateCreditRequest,
};
