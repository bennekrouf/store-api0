#[derive(Debug, thiserror::Error)]
pub enum StoreError {
     #[error("Database error: {0}")]
    Database(String), 
    #[error("Database transaction error: {0}")]
    DatabaseTransaction(String), 
    #[error("Connection pool error: {0}")]
    Pool(String),
    #[error("Lock error")]
    Lock,
    #[error("Initialization error: {0}")]
    Init(String),
}

impl From<duckdb::Error> for StoreError {
    fn from(err: duckdb::Error) -> Self {
        StoreError::Database(err.to_string())
    }
}

impl From<r2d2::Error> for StoreError {
    fn from(err: r2d2::Error) -> Self {
        StoreError::Pool(err.to_string())
    }
}
