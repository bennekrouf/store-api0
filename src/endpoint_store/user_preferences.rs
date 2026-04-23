use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError, UserPreferences};
use crate::infra::db::PgConnection;

/// Get user preferences by email
pub async fn get_user_preferences(
    store: &EndpointStore,
    email: &str,
) -> Result<UserPreferences, StoreError> {
    let client = store.get_admin_conn().await?;
    get_user_preferences_with_conn(&client, email).await
}

pub async fn get_user_preferences_with_conn(
    client: &PgConnection,
    email: &str,
) -> Result<UserPreferences, StoreError> {
    let row = client
        .query_opt(
            "SELECT hidden_defaults, default_tenant_id FROM user_preferences WHERE email = $1",
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
                default_tenant_id: r.get(1),
            })
        }
        None => Ok(UserPreferences {
            email: email.to_string(),
            hidden_defaults: Vec::new(),
            default_tenant_id: None,
        }),
    }
}

/// Update user preferences
pub async fn update_user_preferences(
    store: &EndpointStore,
    email: &str,
    action: &str,
    endpoint_id: &str,
) -> Result<(), StoreError> {
    let mut client = store.get_admin_conn().await?;
    update_user_preferences_with_conn(&mut client, email, action, endpoint_id).await
}

pub async fn update_user_preferences_with_conn(
    client: &PgConnection,
    email: &str,
    action: &str,
    endpoint_id: &str,
) -> Result<(), StoreError> {
    // Note: We don't strictly need a transaction for SET app.bypass_rls if we already have it on the conn,
    // but if we want to be safe with multiple statements, we can.
    // However, deadpool's Object doesn't allow easy re-wrapping in a transaction while keeping RLS session.
    // Actually, it does.
    
    // For simplicity, let's just run queries. RLS bypass is already set on the client if it came from get_admin_conn.
    
    let row = client
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
        client.execute(
            "UPDATE user_preferences SET hidden_defaults = $1 WHERE email = $2",
            &[&updated_hidden_defaults_str, &email],
        )
        .await
        .to_store_error()?;
    } else {
        client.execute(
            "INSERT INTO user_preferences (email, hidden_defaults) VALUES ($1, $2)",
            &[&email, &updated_hidden_defaults_str],
        )
        .await
        .to_store_error()?;
    }

    Ok(())
}

/// Reset user preferences
pub async fn reset_user_preferences(
    store: &EndpointStore,
    email: &str,
) -> Result<(), StoreError> {
    let client = store.get_admin_conn().await?;
    reset_user_preferences_with_conn(&client, email).await
}

pub async fn reset_user_preferences_with_conn(
    client: &PgConnection,
    email: &str,
) -> Result<(), StoreError> {
    client
        .execute("DELETE FROM user_preferences WHERE email = $1", &[&email])
        .await
        .to_store_error()?;

    Ok(())
}
