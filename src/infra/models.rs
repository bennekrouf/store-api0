// src/infra/models.rs
//
// Re-exports of the shared request/response payloads from `api0-types`.
// `UploadResponse` is the store's own upload shape (import counts); the
// gateway's differently-shaped public response is `api0_types::UploadResponse`.

pub use api0_types::catalog::{AddApiGroupRequest, UpdateApiGroupRequest};
pub use api0_types::keys::{ValidateKeyRequest, ValidateKeyResponse};
pub use api0_types::upload::{
    ReferenceData, StoreUploadResponse as UploadResponse, UploadReferenceDataRequest,
    UploadReferenceDataResponse, UploadRequest,
};
