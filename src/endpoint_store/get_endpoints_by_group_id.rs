use crate::endpoint_store::{EndpointStore, StoreError, Endpoint, Parameter};
use crate::endpoint_store::db_helpers::ResultExt;
use std::collections::HashMap;

/// Gets endpoints for a specific group ID
pub(crate) async fn get_endpoints_by_group_id(
    store: &EndpointStore,
    group_id: &str,
) -> Result<Vec<Endpoint>, StoreError> {
    let conn = store.get_conn().await?;
    
    tracing::debug!(
        group_id = %group_id,
        "Fetching endpoints for group"
    );

    // Query to get endpoints with parameters for the specific group
    let endpoints_query = r#"
        SELECT 
            e.id, e.text, e.description, e.verb, e.base, e.path,
            p.name, p.description, p.required, 
            STRING_AGG(pa.alternative, ',') as alternatives,
            e.is_default as is_default
        FROM endpoints e
        LEFT JOIN parameters p ON e.id = p.endpoint_id
        LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
        WHERE e.group_id = ?
        GROUP BY 
            e.id, e.text, e.description, e.verb, e.base, e.path, 
            p.name, p.description, p.required
    "#;

    let mut stmt = conn.prepare(endpoints_query).to_store_error()?;

    let endpoint_rows_iter = stmt
        .query_map([group_id], |row| {
            Ok((
                row.get::<_, String>(0)?,       // e.id
                row.get::<_, String>(1)?,       // e.text
                row.get::<_, String>(2)?,       // e.description
                row.get::<_, String>(3)?,       // e.verb
                row.get::<_, String>(4)?,       // e.base
                row.get::<_, String>(5)?,       // e.path
                row.get::<_, Option<String>>(6)?, // p.name
                row.get::<_, Option<String>>(7)?, // p.description
                row.get::<_, Option<String>>(8)?,   // p.required
                row.get::<_, Option<String>>(9)?, // alternatives
                row.get::<_, Option<String>>(10)?,  // is_default
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
        let endpoint = endpoints_map.entry(id.clone()).or_insert_with(|| {
            tracing::trace!(
                endpoint_id = %id,
                endpoint_text = %text,
                "Creating endpoint object"
            );

            Endpoint {
                id,
                text,
                description,
                verb,
                base,
                path,
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
                "Adding parameter to endpoint"
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
        "Successfully retrieved endpoints for group"
    );

    Ok(result)
}
