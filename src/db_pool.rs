use mobc::{Connection as MobcConnection, Manager, Pool};
use rusqlite::Connection;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

pub type MobcSQLitePool = Pool<SQLiteConnectionManager>;
pub type MobcSQLiteConnection = MobcConnection<SQLiteConnectionManager>;

#[derive(Debug, Error)]
pub enum DbPoolError {
    #[error("SQLite error: {0}")]
    SQLiteError(#[from] rusqlite::Error),

    #[error("Pool error: {0:?}")]
    PoolError(mobc::Error<SQLiteConnectionManager>),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

impl From<mobc::Error<SQLiteConnectionManager>> for DbPoolError {
    fn from(err: mobc::Error<SQLiteConnectionManager>) -> Self {
        DbPoolError::PoolError(err)
    }
}

#[derive(Clone, Debug)]
pub struct SQLiteConnectionManager {
    db_path: Arc<String>,
}

impl SQLiteConnectionManager {
    pub fn file<P: AsRef<Path>>(path: P) -> Result<Self, DbPoolError> {
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }

        let path_str = path.as_ref().to_string_lossy().to_string();
        Ok(Self {
            db_path: Arc::new(path_str),
        })
    }
}

#[async_trait::async_trait]
impl Manager for SQLiteConnectionManager {
    type Connection = Connection;
    type Error = rusqlite::Error;

    async fn connect(&self) -> Result<Self::Connection, Self::Error> {
        let conn = Connection::open(self.db_path.as_str())?;
        // Enable foreign keys only - this is the most critical one
        conn.execute("PRAGMA foreign_keys=ON", [])?;
        Ok(conn)
    }

    async fn check(&self, conn: Self::Connection) -> Result<Self::Connection, Self::Error> {
        conn.execute("SELECT 1", [])?;
        Ok(conn)
    }
}

pub fn create_db_pool<P: AsRef<Path>>(
    db_path: P,
    max_pool_size: u64,
    max_idle_timeout: Option<Duration>,
) -> Result<MobcSQLitePool, DbPoolError> {
    let manager = SQLiteConnectionManager::file(db_path)?;

    let mut builder = Pool::builder().max_open(max_pool_size);

    if let Some(timeout) = max_idle_timeout {
        builder = builder.max_idle_lifetime(Some(timeout));
    }

    Ok(builder.build(manager))
}
