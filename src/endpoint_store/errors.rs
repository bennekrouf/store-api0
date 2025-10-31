#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("Connection pool error: {0}")]
    Pool(String),
    // #[error("Database connection timeout: {0}")]
    // ConnectionTimeout(String),
    // #[error("Database unavailable: {0}")]
    // DatabaseUnavailable(String),
}

impl From<tokio_postgres::Error> for StoreError {
    fn from(err: tokio_postgres::Error) -> Self {
        let error_str = err.to_string();
        if error_str.contains("connection") || error_str.contains("timeout") {
            StoreError::Pool(format!("Database unavailable: {}", error_str))
        } else {
            StoreError::Database(error_str)
        }
    }
}

impl From<deadpool_postgres::PoolError> for StoreError {
    fn from(err: deadpool_postgres::PoolError) -> Self {
        let error_str = format!("{:?}", err);
        if error_str.contains("timeout") || error_str.contains("closed") {
            StoreError::Pool(format!("Database unavailable: {}", error_str))
        } else {
            StoreError::Pool(format!("Database pool error: {}", error_str))
        }
    }
}
