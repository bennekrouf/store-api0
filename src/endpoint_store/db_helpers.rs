use crate::endpoint_store::StoreError;

pub trait ResultExt<T> {
    fn to_store_error(self) -> Result<T, StoreError>;
}

impl<T> ResultExt<T> for Result<T, tokio_postgres::Error> {
    fn to_store_error(self) -> Result<T, StoreError> {
        self.map_err(|e| StoreError::Database(e.to_string()))
    }
}

impl<T> ResultExt<T> for Result<T, deadpool_postgres::PoolError> {
    fn to_store_error(self) -> Result<T, StoreError> {
        self.map_err(|e| StoreError::Pool(format!("Database pool error: {:?}", e)))
    }
}

