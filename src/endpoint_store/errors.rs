// src/endpoint_store/errors.rs
use crate::db_pool::{DuckDBConnectionManager, DbPoolError};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(String), 
    #[error("Connection pool error: {0}")]
    Pool(String),
}

impl From<duckdb::Error> for StoreError {
    fn from(err: duckdb::Error) -> Self {
        StoreError::Database(err.to_string())
    }
}

impl From<mobc::Error<DuckDBConnectionManager>> for StoreError {
    fn from(err: mobc::Error<DuckDBConnectionManager>) -> Self {
        StoreError::Pool(format!("Failed to create connection pool: {:?}", err))
    }
}

impl From<DbPoolError> for StoreError {
    fn from(err: DbPoolError) -> Self {
        match err {
            DbPoolError::DuckDBError(e) => StoreError::Database(e.to_string()),
            DbPoolError::PoolError(e) => StoreError::Pool(format!("Pool error: {:?}", e)),
            DbPoolError::IoError(e) => StoreError::Pool(format!("IO error: {:?}", e)),
        }
    }
}
