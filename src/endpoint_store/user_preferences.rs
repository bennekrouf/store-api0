// Create a new file src/endpoint_store/user_preferences.rs

use crate::endpoint_store::{EndpointStore, StoreError, UserPreferences};
use crate::endpoint_store::db_helpers::ResultExt;

/// Get user preferences by email
pub async fn get_user_preferences(
    store: &EndpointStore,
    email: &str,
) -> Result<UserPreferences, StoreError> {
    let conn = store.get_conn().await?;
    
    // Check if user has preferences
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM user_preferences WHERE email = ?)",
        [email],
        |row| row.get(0),
    ).to_store_error()?;

    if !exists {
        // Return empty preferences if none exist
        return Ok(UserPreferences {
            email: email.to_string(),
            hidden_defaults: Vec::new(),
        });
    }

    // Get the hidden defaults
    let hidden_defaults_str: String = conn.query_row(
        "SELECT hidden_defaults FROM user_preferences WHERE email = ?",
        [email],
        |row| row.get(0),
    ).to_store_error()?;

    // Parse comma-separated string into Vec<String>
    let hidden_defaults = if hidden_defaults_str.is_empty() {
        Vec::new()
    } else {
        hidden_defaults_str.split(',').map(String::from).collect()
    };

    Ok(UserPreferences {
        email: email.to_string(),
        hidden_defaults,
    })
}

/// Update user preferences
pub async fn update_user_preferences(
    store: &EndpointStore,
    email: &str,
    action: &str,
    endpoint_id: &str,
) -> Result<bool, StoreError> {
    let mut conn = store.get_conn().await?;
    let tx = conn.transaction().to_store_error()?;

    // Get current preferences
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM user_preferences WHERE email = ?)",
        [email],
        |row| row.get(0),
    ).to_store_error()?;

    let hidden_defaults = if exists {
        let hidden_defaults_str: String = tx.query_row(
            "SELECT hidden_defaults FROM user_preferences WHERE email = ?",
            [email],
            |row| row.get(0),
        ).to_store_error()?;

        if hidden_defaults_str.is_empty() {
            Vec::new()
        } else {
            hidden_defaults_str.split(',').map(String::from).collect::<Vec<String>>()
        }
    } else {
        Vec::new()
    };

    // Update the list based on action
    let mut updated_hidden_defaults = hidden_defaults.clone();
    
    match action {
        "hide_default" => {
            if !updated_hidden_defaults.contains(&endpoint_id.to_string()) {
                updated_hidden_defaults.push(endpoint_id.to_string());
            }
        },
        "show_default" => {
            updated_hidden_defaults.retain(|id| id != endpoint_id);
        },
        _ => {
            return Err(StoreError::Database(format!("Invalid action: {}", action)));
        }
    }

    // Convert back to comma-separated string
    let updated_hidden_defaults_str = updated_hidden_defaults.join(",");

    // Insert or update preferences
    if exists {
        tx.execute(
            "UPDATE user_preferences SET hidden_defaults = ? WHERE email = ?",
            &[&updated_hidden_defaults_str, email],
        ).to_store_error()?;
    } else {
        tx.execute(
            "INSERT INTO user_preferences (email, hidden_defaults) VALUES (?, ?)",
            &[email, &updated_hidden_defaults_str],
        ).to_store_error()?;
    }

    tx.commit().to_store_error()?;
    Ok(true)
}

/// Reset user preferences
pub async fn reset_user_preferences(
    store: &EndpointStore,
    email: &str,
) -> Result<bool, StoreError> {
    let conn = store.get_conn().await?;
    
    conn.execute(
        "DELETE FROM user_preferences WHERE email = ?",
        [email],
    ).to_store_error()?;

    Ok(true)
}
