use duckdb::Connection;
use mobc::{Connection as MobcConnection, Manager, Pool};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

pub type MobcDuckDBPool = Pool<DuckDBConnectionManager>;
pub type MobcDuckDBConnection = MobcConnection<DuckDBConnectionManager>;

// Custom error type for the database pool
#[derive(Debug, Error)]
pub enum DbPoolError {
    #[error("DuckDB error: {0}")]
    DuckDBError(#[from] duckdb::Error),
    
    #[error("Pool error: {0:?}")]
    PoolError(mobc::Error<DuckDBConnectionManager>),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

// Implement From manually for mobc::Error
impl From<mobc::Error<DuckDBConnectionManager>> for DbPoolError {
    fn from(err: mobc::Error<DuckDBConnectionManager>) -> Self {
        DbPoolError::PoolError(err)
    }
}

// Rest of the code remains the same...
#[derive(Clone, Debug)]
pub struct DuckDBConnectionManager {
    db_path: Arc<String>,
}

impl DuckDBConnectionManager {
    pub fn file<P: AsRef<Path>>(path: P) -> Result<Self, DbPoolError> {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        
        let path_str = path.as_ref().to_string_lossy().to_string();
        Ok(Self {
            db_path: Arc::new(path_str),
        })
    }

    // pub fn memory() -> Self {
    //     Self {
    //         db_path: Arc::new(":memory:".to_string()),
    //     }
    // }
}

#[async_trait::async_trait]
impl Manager for DuckDBConnectionManager {
    type Connection = Connection;
    type Error = duckdb::Error;

    async fn connect(&self) -> Result<Self::Connection, Self::Error> {
        Connection::open(self.db_path.as_str())
    }

    async fn check(&self, conn: Self::Connection) -> Result<Self::Connection, Self::Error> {
        conn.execute_batch("SELECT 1")?;
        Ok(conn)
    }
}

pub fn create_db_pool<P: AsRef<Path>>(
    db_path: P, 
    max_pool_size: u64,
    max_idle_timeout: Option<Duration>
) -> Result<MobcDuckDBPool, DbPoolError> {
    let manager = DuckDBConnectionManager::file(db_path)?;
    
    let mut builder = Pool::builder()
        .max_open(max_pool_size);
    
    if let Some(timeout) = max_idle_timeout {
        builder = builder.max_idle_lifetime(Some(timeout));
    }
    
    Ok(builder.build(manager))
}
