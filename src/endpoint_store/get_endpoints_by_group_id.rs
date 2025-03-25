use crate::endpoint_store::{EndpointStore, StoreError, Endpoint, Parameter};
use crate::endpoint_store::db_helpers::ResultExt;
use std::collections::HashMap;

/// Gets all endpoints for a specific group
pub(crate) fn get_endpoints_by_group_id(
    store: &EndpointStore,
    group_id: &str,
) -> Result<Vec<Endpoint>, StoreError> {
    let mut conn = store.get_conn()?;
    let tx = conn.transaction().to_store_error()?;

        tracing::debug!(
            group_id = %group_id,
            "Fetching endpoints for group"
        );

        // Check if there are any endpoints for this group
        let endpoint_count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM endpoints WHERE group_id = ?",
            [group_id],
            |row| row.get(0),
        ).to_store_error()?;

        tracing::debug!(
            group_id = %group_id,
            count = endpoint_count,
            "Found endpoints for group"
        );

        if endpoint_count == 0 {
            tracing::warn!(
                group_id = %group_id,
                "No endpoints found for group"
            );
            return Ok(Vec::new());
        }

        let mut stmt = match tx.prepare(r#"
            SELECT 
                e.id,
                e.text,
                e.description,
                e.verb,
                e.base,
                e.path,
                p.name as param_name,
                p.description as param_description,
                p.required,
                STRING_AGG(pa.alternative, ',') as alternatives
            FROM endpoints e
            LEFT JOIN parameters p ON e.id = p.endpoint_id
            LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
            WHERE e.group_id = ?
            GROUP BY e.id, e.text, e.description, e.verb, e.base, e.path, p.name, p.description, p.required
        "#) {
            Ok(stmt) => stmt,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    group_id = %group_id,
                    "Failed to prepare statement for fetching endpoints"
                );
                return Err(StoreError::Database(e.to_string()));
            }
        };

        let rows = match stmt.query_map([group_id], |row| {
            let id: String = row.get(0)?;
            tracing::trace!(
                endpoint_id = %id,
                "Processing endpoint row from database"
            );

            Ok((
                id,
                row.get::<_, String>(1)?,         // text
                row.get::<_, String>(2)?,         // description
                row.get::<_, String>(3)?,         // verb
                row.get::<_, String>(4)?,         // base
                row.get::<_, String>(5)?,         // path
                row.get::<_, Option<String>>(6)?, // param_name
                row.get::<_, Option<String>>(7)?, // param_description
                row.get::<_, Option<bool>>(8)?,   // required
                row.get::<_, Option<String>>(9)?, // alternatives
            ))
        }) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    group_id = %group_id,
                    "Failed to query endpoints for group"
                );
                return Err(StoreError::Database(e.to_string()));
            }
        };

        // Process rows into endpoints
        let mut endpoints_map = HashMap::new();
        for row_result in rows {
            match row_result {
                Ok((
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
                )) => {
                    let endpoint = endpoints_map.entry(id.clone()).or_insert_with(|| {
                        tracing::debug!(
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
                            path: path_value,
                            parameters: Vec::new(),
                            group_id: group_id.to_string(),
                        }
                    });

                    if let (Some(name), Some(desc), Some(req)) = (param_name, param_desc, required)
                    {
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
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        group_id = %group_id,
                        "Failed to process endpoint row"
                    );
                    return Err(StoreError::Database(e.to_string()));
                }
            }
        }

        let result: Vec<Endpoint> = endpoints_map.into_values().collect();

        tracing::debug!(
            group_id = %group_id,
            endpoint_count = result.len(),
            "Successfully retrieved endpoints for group"
        );

        tx.commit().to_store_error()?;
        Ok(result)
}
