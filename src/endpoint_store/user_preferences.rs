use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError, UserPreferences};

/// Get user preferences by email
pub async fn get_user_preferences(
    store: &EndpointStore,
    email: &str,
) -> Result<UserPreferences, StoreError> {
    let client = store.get_conn().await?;

    let row = client
        .query_opt(
            "SELECT hidden_defaults FROM user_preferences WHERE email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    match row {
        Some(r) => {
            let hidden_defaults_str: String = r.get(0);
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
        None => Ok(UserPreferences {
            email: email.to_string(),
            hidden_defaults: Vec::new(),
        }),
    }
}

/// Update user preferences
pub async fn update_user_preferences(
    store: &EndpointStore,
    email: &str,
    action: &str,
    endpoint_id: &str,
) -> Result<bool, StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let row = tx
        .query_opt(
            "SELECT hidden_defaults FROM user_preferences WHERE email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    let exists = row.is_some();
    let hidden_defaults = if let Some(r) = row {
        let hidden_defaults_str: String = r.get(0);
        if hidden_defaults_str.is_empty() {
            Vec::new()
        } else {
            hidden_defaults_str
                .split(',')
                .map(String::from)
                .collect::<Vec<String>>()
        }
    } else {
        Vec::new()
    };

    let mut updated_hidden_defaults = hidden_defaults.clone();

    match action {
        "hide_default" => {
            if !updated_hidden_defaults.contains(&endpoint_id.to_string()) {
                updated_hidden_defaults.push(endpoint_id.to_string());
            }
        }
        "show_default" => {
            updated_hidden_defaults.retain(|id| id != endpoint_id);
        }
        _ => {
            return Err(StoreError::Database(format!("Invalid action: {}", action)));
        }
    }

    let updated_hidden_defaults_str = updated_hidden_defaults.join(",");

    if exists {
        tx.execute(
            "UPDATE user_preferences SET hidden_defaults = $1 WHERE email = $2",
            &[&updated_hidden_defaults_str, &email],
        )
        .await
        .to_store_error()?;
    } else {
        tx.execute(
            "INSERT INTO user_preferences (email, hidden_defaults) VALUES ($1, $2)",
            &[&email, &updated_hidden_defaults_str],
        )
        .await
        .to_store_error()?;
    }

    tx.commit().await.to_store_error()?;
    Ok(true)
}

/// Reset user preferences
pub async fn reset_user_preferences(
    store: &EndpointStore,
    email: &str,
) -> Result<bool, StoreError> {
    let client = store.get_conn().await?;

    client
        .execute("DELETE FROM user_preferences WHERE email = $1", &[&email])
        .await
        .to_store_error()?;

    Ok(true)
}

