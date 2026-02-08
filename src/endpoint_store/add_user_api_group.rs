use crate::app_log;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{ApiGroupWithEndpoints, EndpointStore, StoreError};
/// Adds a single API group for a user
pub async fn add_user_api_group(
    store: &EndpointStore,
    email: &str,
    api_group: &ApiGroupWithEndpoints,
) -> Result<usize, StoreError> {
    use crate::endpoint_store::tenant_management;

    // Resolve tenant
    let tenant = tenant_management::get_default_tenant(store, email).await?;
    let tenant_id = tenant.id.clone();

    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let group_id = &api_group.group.id;

    // 1. Insert/Update API Group
    // We update if exists to handle idempotent uploads usually
    // BUT now we must ensure tenant_id is set.
    
    // Check existence
    let existing_group = tx.query_opt("SELECT tenant_id FROM api_groups WHERE id = $1", &[group_id]).await.to_store_error()?;
    
    if let Some(row) = existing_group {
        // Group exists. Verify ownership?
        let existing_tenant_id: Option<String> = row.get(0);
        if let Some(t_id) = existing_tenant_id {
            if t_id != tenant_id {
                 // Warn or handle mismatch
                 app_log!(warn, email=%email, group=%group_id, "Group exists under different tenant");
            }
        }
    }

    tx.execute(
        "INSERT INTO api_groups (id, name, description, base, tenant_id) 
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (id) DO UPDATE SET 
            name = EXCLUDED.name,
            description = EXCLUDED.description,
            base = EXCLUDED.base,
            tenant_id = COALESCE(api_groups.tenant_id, EXCLUDED.tenant_id)", 
        &[
            group_id,
            &api_group.group.name,
            &api_group.group.description,
            &api_group.group.base,
            &tenant_id
        ],
    )
    .await
    .to_store_error()?;

    // 2. Insert User-Group Association (Legacy/Redundant but good for quick lookup if we keep user_groups table)
    // The schema has `user_groups`.
    tx.execute(
        "INSERT INTO user_groups (email, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        &[&email, group_id],
    )
    .await
    .to_store_error()?;

    let mut endpoint_count = 0;

    for endpoint in &api_group.endpoints {
        let endpoint_exists_row = tx
            .query_opt("SELECT 1 FROM endpoints WHERE id = $1", &[&endpoint.id])
            .await
            .to_store_error()?;

        if endpoint_exists_row.is_none() {
            tx.execute(
                "INSERT INTO endpoints (id, text, description, verb, base, path, group_id, suggested_sentence) 
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
                &[
                    &endpoint.id,
                    &endpoint.text,
                    &endpoint.description,
                    &endpoint.verb,
                    &endpoint.base,
                    &endpoint.path,
                    group_id,
                    &endpoint.suggested_sentence,
                ],
            )
            .await
            .to_store_error()?;
        } else {
            tx.execute(
                "UPDATE endpoints SET text = $1, description = $2, verb = $3, base = $4, path = $5, group_id = $6, suggested_sentence = $7 WHERE id = $8",
                &[
                    &endpoint.text,
                    &endpoint.description,
                    &endpoint.verb,
                    &endpoint.base,
                    &endpoint.path,
                    group_id,
                    &endpoint.suggested_sentence,
                    &endpoint.id,
                ],
            )
            .await
            .to_store_error()?;
        }

        tx.execute(
            "INSERT INTO user_endpoints (email, endpoint_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            &[&email, &endpoint.id],
        )
        .await
        .to_store_error()?;

        // Clean up existing parameters
        tx.execute(
            "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
            &[&endpoint.id],
        )
        .await
        .to_store_error()?;

        tx.execute(
            "DELETE FROM parameters WHERE endpoint_id = $1",
            &[&endpoint.id],
        )
        .await
        .to_store_error()?;

        // Add parameters
        for param in &endpoint.parameters {
            let required = param.required.parse::<bool>().unwrap_or(false);

            tx.execute(
                "INSERT INTO parameters (endpoint_id, name, description, required) 
                VALUES ($1, $2, $3, $4)",
                &[&endpoint.id, &param.name, &param.description, &required],
            )
            .await
            .to_store_error()?;

            for alt in &param.alternatives {
                tx.execute(
                    "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) 
                    VALUES ($1, $2, $3)",
                    &[&endpoint.id, &param.name, alt],
                )
                .await
                .to_store_error()?;
            }
        }

        endpoint_count += 1;
    }

    app_log!(info,
        email = %email,
        group_id = %group_id,
        endpoint_count = endpoint_count,
        "API group successfully added"
    );

    tx.commit().await.to_store_error()?;
    Ok(endpoint_count)
}
