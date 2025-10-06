#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("Connection pool error: {0}")]
    Pool(String),
}

impl From<tokio_postgres::Error> for StoreError {
    fn from(err: tokio_postgres::Error) -> Self {
        StoreError::Database(err.to_string())
    }
}

impl From<deadpool_postgres::PoolError> for StoreError {
    fn from(err: deadpool_postgres::PoolError) -> Self {
        StoreError::Pool(format!("Failed to get connection from pool: {:?}", err))
    }
}

