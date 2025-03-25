use crate::endpoint_store::{EndpointStore, StoreError, ApiGroupWithEndpoints};
use crate::endpoint_store::db_helpers::ResultExt;

/// Gets or creates API groups for a user
pub async fn get_or_create_user_api_groups(
    store: &EndpointStore,
    email: &str,
) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
    let mut conn = store.get_conn()?;
    let tx = conn.transaction().to_store_error()?;

    // Check if user has custom groups
    let has_custom: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM user_groups WHERE email = ?)",
        [email],
        |row| row.get(0),
    ).to_store_error()?;

    // If user doesn't have custom groups, create them from defaults
    if !has_custom {
        tracing::info!(email = %email, "User has no API groups, creating defaults");

        // Get default groups
        let default_groups = store.get_default_api_groups()?;

        // Debug log to check if defaults are found
        tracing::info!(
            email = %email,
            group_count = default_groups.len(),
            "Found default API groups to create"
        );

        if default_groups.is_empty() {
            tracing::warn!(email = %email, "No default API groups found");
            // You might want to create at least one basic default group here
        }

        for group in &default_groups {
            // Associate group with user
            tx.execute(
                "INSERT OR IGNORE INTO user_groups (email, group_id) VALUES (?, ?)",
                &[email, &group.group.id],
            ).to_store_error()?;

            // Associate each endpoint with user
            for endpoint in &group.endpoints {
                tx.execute(
                    "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                    &[email, &endpoint.id],
                ).to_store_error()?;
            }
        }

        tracing::info!(
            email = %email,
            count = default_groups.len(),
            "Created default API groups for user"
        );
    }

    tx.commit().to_store_error()?;
    // Now get and return the user's groups using the same connection
    store.get_api_groups_by_email(email) 
}
