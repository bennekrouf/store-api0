use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{
    ApiGroup, ApiGroupWithEndpoints, Endpoint, EndpointStore, Parameter, StoreError,
};
// use duckdb::ToSql;
use std::collections::HashMap;
use rusqlite::ToSql;

type DbTransaction<'a> = rusqlite::Transaction<'a>;

/// Gets all API groups and endpoints for a user
pub async fn get_api_groups_by_email(
    store: &EndpointStore,
    email: &str,
) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
    tracing::info!(email = %email, "Starting to fetch API groups and endpoints");
    let mut conn = store.get_conn().await?;
    let tx = conn.transaction().to_store_error()?;

    // Check if user has custom groups
    let has_custom: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM user_groups WHERE email = ?)",
            [email],
            |row| row.get(0),
        )
        .to_store_error()?;

    let result = if has_custom {
        tracing::info!(email = %email, "Fetching custom groups and endpoints");
        fetch_custom_groups_with_endpoints(&tx, email)?
    } else {
        tracing::info!(email = %email, "Fetching default groups and endpoints");
        fetch_default_groups_with_endpoints(&tx)?
    };

    tracing::info!(
        group_count = result.len(),
        email = %email,
        "Successfully fetched API groups and endpoints"
    );

    tx.commit().to_store_error()?;
    Ok(result)
}

/// Fetches custom API groups and endpoints for a specific user
fn fetch_custom_groups_with_endpoints(
    tx: &DbTransaction,
    email: &str,
) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
    tracing::debug!(email = %email, "Fetching custom groups and endpoints");

    // Get user's custom groups
    let groups_query = r#"
        SELECT g.id, g.name, g.description, g.base
        FROM api_groups g
        INNER JOIN user_groups ug ON g.id = ug.group_id
        WHERE ug.email = ?
    "#;

    let groups = fetch_groups(tx, groups_query, &[&email])?;
    let mut result = Vec::new();

    for group in groups {
        let endpoints = fetch_custom_endpoints(tx, email, &group.id)?;

        tracing::debug!(
            group_id = %group.id,
            endpoint_count = endpoints.len(),
            "Added endpoints to custom group"
        );

        result.push(ApiGroupWithEndpoints { group, endpoints });
    }

    Ok(result)
}

/// Fetches default API groups and endpoints
fn fetch_default_groups_with_endpoints(
    tx: &DbTransaction,
) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
    tracing::debug!("Fetching default groups and endpoints 2");

    // Get default groups
    let groups_query = r#"
        SELECT g.id, g.name, g.description, g.base
        FROM api_groups g
        WHERE g.is_default = true
    "#;

    let groups = fetch_groups(tx, groups_query, &[])?;
    let mut result = Vec::new();

    for group in groups {
        tracing::info!(
            group_id = %group.id,
            group_name = %group.name,
            "Processing default group FOR LOOP !!!!"
        );
        let endpoints = fetch_default_endpoints(tx, &group.id)?;

        tracing::debug!(
            group_id = %group.id,
            endpoint_count = endpoints.len(),
            "Added endpoints to default group"
        );

        result.push(ApiGroupWithEndpoints { group, endpoints });
    }

    Ok(result)
}

/// Helper function to fetch API groups using the provided query and parameters
fn fetch_groups(
    tx: &DbTransaction,
    query: &str,
    params: &[&dyn ToSql],
) -> Result<Vec<ApiGroup>, StoreError> {
    let mut stmt = tx.prepare(query).to_store_error()?;

    let groups = stmt
        .query_map(params, |row| {
            Ok(ApiGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                base: row.get(3)?,
            })
        })
        .to_store_error()?;

    let mut result = Vec::new();
    for group_result in groups {
        match group_result {
            Ok(g) => result.push(g),
            Err(e) => {
                tracing::error!(error = %e, "Failed to get API group");
                continue;
            }
        }
    }

    Ok(result)
}

/// Fetches custom endpoints for a specific group and user
fn fetch_custom_endpoints(
    tx: &DbTransaction,
    email: &str,
    group_id: &str,
) -> Result<Vec<Endpoint>, StoreError> {
    let endpoints_query = r#"
        SELECT 
            e.id, e.text, e.description, e.verb, e.base, e.path, 
            p.name, p.description, p.required, 
            STRING_AGG(pa.alternative, ',') as alternatives, 
            e.is_default as is_default
        FROM endpoints e
        INNER JOIN user_endpoints ue ON e.id = ue.endpoint_id
        LEFT JOIN parameters p ON e.id = p.endpoint_id
        LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
        WHERE ue.email = ? AND e.group_id = ?
        GROUP BY 
            e.id, e.text, e.description, e.verb, e.base, e.path, 
            p.name, p.description, p.required
    "#;

    tracing::debug!(
        email = %email,
        group_id = %group_id,
        "Fetching custom endpoints"
    );

    let mut stmt = tx.prepare(endpoints_query).to_store_error()?;

    let endpoint_rows_iter = stmt
        .query_map([email, group_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        })
        .to_store_error()?;

    let mut endpoint_rows = Vec::new();
    for row_result in endpoint_rows_iter {
        endpoint_rows.push(row_result.to_store_error()?);
    }

    // Process endpoint rows into endpoints with parameters
    let mut endpoints_map = HashMap::new();

    for (
        id,
        text,
        description,
        verb,
        base,
        path_value,
        param_name,
        param_desc,
        required,
        alternatives_str,
        is_default,
    ) in endpoint_rows
    {
        let endpoint = endpoints_map.entry(id.clone()).or_insert_with(|| {
            tracing::debug!(
                endpoint_id = %id,
                endpoint_text = %text,
                "Creating custom endpoint object"
            );

            Endpoint {
                id,
                text,
                description,
                verb,
                base,
                path: path_value,
                parameters: Vec::new(),
                group_id: group_id.to_string(),
                is_default,
            }
        });

        if let (Some(name), Some(desc), Some(req)) = (param_name, param_desc, required) {
            let alternatives = alternatives_str
                .map(|s| s.split(',').map(String::from).collect::<Vec<_>>())
                .unwrap_or_default();

            tracing::trace!(
                endpoint_id = %endpoint.id,
                param_name = %name,
                "Adding parameter to custom endpoint"
            );

            endpoint.parameters.push(Parameter {
                name,
                description: desc,
                required: req,
                alternatives,
            });
        }
    }

    let result: Vec<Endpoint> = endpoints_map.into_values().collect();

    tracing::debug!(
        group_id = %group_id,
        endpoint_count = result.len(),
        "Successfully retrieved custom endpoints for group"
    );

    Ok(result)
}

/// Fetches default endpoints for a specific group
fn fetch_default_endpoints(
    tx: &DbTransaction,
    group_id: &str,
) -> Result<Vec<Endpoint>, StoreError> {
    let endpoints_query = r#"
        SELECT 
            e.id, e.text, e.description, e.verb, e.base, e.path,
            p.name, p.description, p.required, STRING_AGG(pa.alternative, ',') as alternatives,
            e.is_default as is_default
        FROM endpoints e
        LEFT JOIN parameters p ON e.id = p.endpoint_id
        LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
        WHERE e.group_id = ? AND e.is_default = true
        GROUP BY 
            e.id, e.text, e.description, e.verb, e.base, e.path, 
            p.name, p.description, p.required
    "#;

    fetch_endpoints(tx, endpoints_query, &[&group_id], group_id)
}

/// Helper function to fetch and process endpoints
fn fetch_endpoints(
    tx: &DbTransaction,
    query: &str,
    params: &[&dyn ToSql],
    group_id: &str,
) -> Result<Vec<Endpoint>, StoreError> {
    let mut stmt = tx.prepare(query).to_store_error()?;

    let endpoint_rows_iter = stmt
        .query_map(params, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        })
        .to_store_error()?;

    let mut endpoint_rows = Vec::new();
    for row_result in endpoint_rows_iter {
        endpoint_rows.push(row_result.to_store_error()?);
    }

    // Process endpoint rows into endpoints with parameters
    let mut endpoints_map = HashMap::new();

    for (
        id,
        text,
        description,
        verb,
        base,
        path,
        param_name,
        param_desc,
        required,
        alternatives_str,
        is_default,
    ) in endpoint_rows
    {
        let endpoint = endpoints_map.entry(id.clone()).or_insert_with(|| Endpoint {
            id,
            text,
            description,
            verb,
            base,
            path,
            parameters: Vec::new(),
            group_id: group_id.to_string(),
            is_default,
        });

        if let (Some(name), Some(desc), Some(req)) = (param_name, param_desc, required) {
            let alternatives = alternatives_str
                .map(|s| s.split(',').map(String::from).collect::<Vec<_>>())
                .unwrap_or_default();

            endpoint.parameters.push(Parameter {
                name,
                description: desc,
                required: req,
                alternatives,
            });
        }
    }

    Ok(endpoints_map.into_values().collect())
}
