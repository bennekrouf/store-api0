use crate::endpoint_store::StoreError;
use crate::db_pool::SQLiteConnectionManager;
use mobc::Error as MobcError;

pub trait ResultExt<T> {
    fn to_store_error(self) -> Result<T, StoreError>;
}

impl<T> ResultExt<T> for Result<T, MobcError<SQLiteConnectionManager>> {
    fn to_store_error(self) -> Result<T, StoreError> {
        self.map_err(|e| StoreError::Pool(format!("Database pool error: {:?}", e)))
    }
}

impl<T> ResultExt<T> for Result<T, rusqlite::Error> {
    fn to_store_error(self) -> Result<T, StoreError> {
        self.map_err(|e| StoreError::Database(e.to_string()))
    }
}
