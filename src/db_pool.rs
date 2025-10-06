use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use std::error::Error;
use tokio_postgres::NoTls;

pub type PgPool = Pool;
pub type PgConnection = deadpool_postgres::Object;

pub fn create_pg_pool(database_url: &str) -> Result<PgPool, Box<dyn Error + Send + Sync>> {
    let mut cfg = Config::new();

    // Parse the PostgreSQL URL manually
    cfg.url = Some(database_url.to_string());

    cfg.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    });

    let pool = cfg
        .create_pool(Some(Runtime::Tokio1), NoTls)
        .map_err(|e| format!("Failed to create pool: {}", e))?;

    Ok(pool)
}
