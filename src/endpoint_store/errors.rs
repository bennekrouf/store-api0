use crate::db_pool::SQLiteConnectionManager;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(String), 
    #[error("Connection pool error: {0}")]
    Pool(String),
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        StoreError::Database(err.to_string())
    }
}

impl From<mobc::Error<SQLiteConnectionManager>> for StoreError {
    fn from(err: mobc::Error<SQLiteConnectionManager>) -> Self {
        StoreError::Pool(format!("Failed to create connection pool: {:?}", err))
    }
}
