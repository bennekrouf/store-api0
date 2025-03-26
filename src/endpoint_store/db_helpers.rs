use crate::endpoint_store::StoreError;
use crate::db_pool::DuckDBConnectionManager;

use mobc::Error as MobcError;

pub trait ResultExt<T> {
    fn to_store_error(self) -> Result<T, StoreError>;
}

impl<T> ResultExt<T> for Result<T, MobcError<DuckDBConnectionManager>> {
    fn to_store_error(self) -> Result<T, StoreError> {
        self.map_err(|e| StoreError::Pool(format!("Database pool error: {:?}", e)))
    }
}

impl<T> ResultExt<T> for Result<T, duckdb::Error> {
    fn to_store_error(self) -> Result<T, StoreError> {
        self.map_err(|e| StoreError::Database(e.to_string()))
    }
}

// Add a macro for executing async functions with a connection
#[macro_export]
macro_rules! with_conn {
    ($store:expr, |$conn:ident| $block:expr) => {
        {
            let $conn = $store.get_conn().await?;
            $block
        }
    };
}

// Add a macro for transaction handling
#[macro_export]
macro_rules! with_transaction {
    ($conn:expr, |$tx:ident| $block:expr) => {
        {
            let $tx = $conn.transaction().to_store_error()?;
            let result = $block;
            $tx.commit().to_store_error()?;
            result
        }
    };
}
