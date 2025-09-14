use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};

/// Get all authorized domains (system-wide for CORS)
pub async fn get_all_authorized_domains(store: &EndpointStore) -> Result<Vec<String>, StoreError> {
    let conn = store.get_conn().await?;

    tracing::debug!("Fetching all authorized domains");

    let mut stmt = conn
        .prepare("SELECT DISTINCT domain FROM domains WHERE verified = true OR email = 'system' ORDER BY domain")
        .to_store_error()?;

    let domain_iter = stmt
        .query_map([], |row| Ok(row.get::<_, String>(0)?))
        .to_store_error()?;

    let mut domains = Vec::new();
    for domain_result in domain_iter {
        domains.push(domain_result.to_store_error()?);
    }

    tracing::debug!(
        domain_count = domains.len(),
        "Retrieved authorized domains from database"
    );

    // Add default system domains if none exist
    if domains.is_empty() {
        tracing::info!("No domains found in database, returning default system domains");
        domains = vec![
            "https://studio.cvenom.com".to_string(),
            "https://app.api0.ai".to_string(),
            "http://localhost:3000".to_string(), // For development
            "http://localhost:5173".to_string(), // Vite dev server
        ];
    }

    Ok(domains)
}

/// Initialize default system domains
pub async fn initialize_system_domains(store: &EndpointStore) -> Result<(), StoreError> {
    let conn = store.get_conn().await?;

    // Check if system domains already exist
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM domains WHERE email = 'system'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    if count > 0 {
        tracing::debug!("System domains already initialized");
        return Ok(());
    }

    tracing::info!("Initializing default system domains");

    let system_domains = vec![
        "https://studio.cvenom.com",
        "https://app.api0.ai",
        "http://localhost:3000",
        "http://localhost:5173",
    ];

    for domain in system_domains {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        conn.execute(
            "INSERT OR IGNORE INTO domains (id, email, domain, verified, added_at) VALUES (?, 'system', ?, true, ?)",
            &[&id, domain, &now],
        ).to_store_error()?;
    }

    tracing::info!("System domains initialized successfully");
    Ok(())
}
