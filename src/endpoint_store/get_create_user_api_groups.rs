use crate::endpoint_store::{EndpointStore, StoreError, ApiGroupWithEndpoints, ApiGroup, Endpoint};
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::generate_id_from_text;

/// Gets or creates API groups for a user
pub async fn get_or_create_user_api_groups(
    store: &EndpointStore,
    email: &str,
) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {

    // Check if user already has API groups
    let existing_groups = store.get_api_groups_by_email(email).await?;
    
    if !existing_groups.is_empty() {
        tracing::info!(
            email = %email,
            group_count = existing_groups.len(),
            "User already has API groups"
        );
        return Ok(existing_groups);
    }

    // Create a default group for new users if they don't have any
    let mut conn = store.get_conn().await?;
    let tx = conn.transaction().to_store_error()?;

    tracing::info!(email = %email, "User has no API groups, creating a default one");

    // Create a basic default group
    let default_group_id = generate_id_from_text("Default API");
    let default_group = ApiGroup {
        id: default_group_id.clone(),
        name: "Default API".to_string(),
        description: "Your default API group".to_string(),
        base: "https://api.example.com".to_string(),
    };

    // Create a sample endpoint for the default group
    let sample_endpoint = Endpoint {
        id: generate_id_from_text("sample-endpoint"),
        text: "Sample endpoint".to_string(),
        description: "A sample endpoint to get you started".to_string(),
        verb: "GET".to_string(),
        base: "https://api.example.com".to_string(),
        path: "/sample".to_string(),
        group_id: default_group_id.clone(),
        parameters: vec![],
    };

    // Insert the default group
    tx.execute(
        "INSERT INTO api_groups (id, name, description, base) VALUES (?, ?, ?, ?)",
        &[
            &default_group.id,
            &default_group.name,
            &default_group.description,
            &default_group.base,
        ],
    ).to_store_error()?;

    // Insert the sample endpoint
    tx.execute(
        "INSERT INTO endpoints (id, text, description, verb, base, path, group_id) VALUES (?, ?, ?, ?, ?, ?, ?)",
        &[
            &sample_endpoint.id,
            &sample_endpoint.text,
            &sample_endpoint.description,
            &sample_endpoint.verb,
            &sample_endpoint.base,
            &sample_endpoint.path,
            &sample_endpoint.group_id,
        ],
    ).to_store_error()?;

    // Associate group with user
    tx.execute(
        "INSERT OR IGNORE INTO user_groups (email, group_id) VALUES (?, ?)",
        &[email, &default_group_id],
    ).to_store_error()?;

    // Associate endpoint with user
    tx.execute(
        "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
        &[email, &sample_endpoint.id],
    ).to_store_error()?;

    tx.commit().to_store_error()?;

    // Create the result
    let default_api_group = ApiGroupWithEndpoints {
        group: default_group,
        endpoints: vec![sample_endpoint],
    };

    tracing::info!(
        email = %email,
        "Created default API group for new user"
    );

    Ok(vec![default_api_group])
}
