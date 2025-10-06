use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};

/// Get all authorized domains (system-wide for CORS)
pub async fn get_all_authorized_domains(store: &EndpointStore) -> Result<Vec<String>, StoreError> {
    let client = store.get_conn().await?;

    tracing::debug!("Fetching all authorized domains");

    let rows = client
        .query(
            "SELECT DISTINCT domain FROM domains WHERE verified = true OR email = 'system' ORDER BY domain",
            &[],
        )
        .await
        .to_store_error()?;

    let domains: Vec<String> = rows.iter().map(|row| row.get(0)).collect();

    tracing::debug!(
        domain_count = domains.len(),
        "Retrieved authorized domains from database"
    );

    // Add default system domains if none exist
    if domains.is_empty() {
        tracing::info!("No domains found in database, returning default system domains");
        return Ok(vec![
            "https://studio.cvenom.com".to_string(),
            "https://app.api0.ai".to_string(),
            "http://localhost:3000".to_string(),
            "http://localhost:5173".to_string(),
        ]);
    }

    Ok(domains)
}

/// Initialize default system domains
pub async fn initialize_system_domains(store: &EndpointStore) -> Result<(), StoreError> {
    let client = store.get_conn().await?;

    // Check if system domains already exist
    let count_row = client
        .query_one("SELECT COUNT(*) FROM domains WHERE email = 'system'", &[])
        .await
        .to_store_error()?;

    let count: i64 = count_row.get(0);

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
        let now = chrono::Utc::now();

        client
            .execute(
                "INSERT INTO domains (id, email, domain, verified, added_at) VALUES ($1, 'system', $2, true, $3) ON CONFLICT DO NOTHING",
                &[&id, &domain, &now],
            )
            .await  // <-- Added .await here
            .to_store_error()?;
    }

    tracing::info!("System domains initialized successfully");
    Ok(())
}
