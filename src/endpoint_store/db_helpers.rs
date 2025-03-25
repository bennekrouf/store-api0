use crate::endpoint_store::StoreError;

pub trait ResultExt<T, E> {
    fn to_store_error(self) -> Result<T, StoreError>;
}

impl<T, E: std::fmt::Display> ResultExt<T, E> for Result<T, E> {
    fn to_store_error(self) -> Result<T, StoreError> {
        self.map_err(|e| StoreError::Database(e.to_string()))
    }
}

