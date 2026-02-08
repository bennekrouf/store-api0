use graflog::app_span;

use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{
    ApiGroup, ApiGroupWithEndpoints, Endpoint, EndpointStore, Parameter, StoreError,
};
use std::collections::HashMap;
/// Gets all API groups and endpoints for a user
pub async fn get_api_groups_by_email(
    store: &EndpointStore,
    email: &str,
) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
    app_log!(info, email = %email, "Starting to fetch API groups and endpoints");
    let client = store.get_conn().await?;

    app_log!(info, email = %email, "Fetching custom groups and endpoints");
    let result = fetch_custom_groups_with_endpoints(&client, email).await?;

    app_log!(info,
        group_count = result.len(),
        email = %email,
        "Successfully fetched API groups and endpoints"
    );

    Ok(result)
}

/// Fetches custom API groups and endpoints for a specific user
async fn fetch_custom_groups_with_endpoints(
    client: &deadpool_postgres::Object,
    email: &str,
) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
    app_log!(debug, email = %email, "Fetching custom groups and endpoints");

    let groups_query = r#"
        SELECT g.id, g.name, g.description, g.base, g.tenant_id
        FROM api_groups g
        INNER JOIN user_groups ug ON g.id = ug.group_id
        WHERE ug.email = $1
    "#;

    let rows = client
        .query(groups_query, &[&email])
        .await
        .to_store_error()?;

    let mut result = Vec::new();

    for row in rows {
        let group = ApiGroup {
            id: row.get(0),
            name: row.get(1),
            description: row.get(2),
            base: row.get(3),
            tenant_id: row.get::<_, Option<String>>(4).unwrap_or_default(),
        };

        let endpoints = fetch_custom_endpoints(client, email, &group.id).await?;

        app_log!(debug,
            group_id = %group.id,
            endpoint_count = endpoints.len(),
            "Added endpoints to custom group"
        );

        result.push(ApiGroupWithEndpoints { group, endpoints });
    }

    Ok(result)
}

/// Fetches custom endpoints for a specific group and user
async fn fetch_custom_endpoints(
    client: &deadpool_postgres::Object,
    email: &str,
    group_id: &str,
) -> Result<Vec<Endpoint>, StoreError> {
    let endpoints_query = r#"
        SELECT 
            e.id, e.text, e.description, e.verb, e.base, e.path, e.suggested_sentence,
            p.name, p.description, p.required, 
            string_agg(pa.alternative, ',') as alternatives
        FROM endpoints e
        INNER JOIN user_endpoints ue ON e.id = ue.endpoint_id
        LEFT JOIN parameters p ON e.id = p.endpoint_id
        LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
        WHERE ue.email = $1 AND e.group_id = $2
        GROUP BY 
            e.id, e.text, e.description, e.verb, e.base, e.path, e.suggested_sentence,
            p.name, p.description, p.required
    "#;

    app_log!(debug,
        email = %email,
        group_id = %group_id,
        "Fetching custom endpoints"
    );

    let rows = client
        .query(endpoints_query, &[&email, &group_id])
        .await
        .to_store_error()?;

    let mut endpoints_map: HashMap<String, Endpoint> = HashMap::new();

    for row in rows {
        let id: String = row.get(0);
        let text: String = row.get(1);
        let description: String = row.get(2);
        let verb: String = row.get(3);
        let base: String = row.get(4);
        let path_value: String = row.get(5);
        let suggested_sentence: String = row.get(6);
        let param_name: Option<String> = row.get(7);
        let param_desc: Option<String> = row.get(8);
        let required: Option<bool> = row.get(9);
        let alternatives_str: Option<String> = row.get(10);

        let endpoint = endpoints_map.entry(id.clone()).or_insert_with(|| {
            app_log!(debug,
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
                suggested_sentence,
                parameters: Vec::new(),
                group_id: group_id.to_string(),
            }
        });

        if let (Some(name), Some(desc), Some(req)) = (param_name, param_desc, required) {
            let alternatives = alternatives_str
                .map(|s| s.split(',').map(String::from).collect::<Vec<_>>())
                .unwrap_or_default();

            app_span!(
                "fetch_custom_endpoints",
                endpoint_id = %endpoint.id,
                param_name = %name,
                "Adding parameter to custom endpoint"
            );

            endpoint.parameters.push(Parameter {
                name,
                description: desc,
                required: req.to_string(),
                alternatives,
            });
        }
    }

    let result: Vec<Endpoint> = endpoints_map.into_values().collect();

    app_log!(debug,
        group_id = %group_id,
        endpoint_count = result.len(),
        "Successfully retrieved custom endpoints for group"
    );

    Ok(result)
}
