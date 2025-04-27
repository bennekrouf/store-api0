use actix_web::web;
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;

// Helper function to check if a group is a default group
pub async fn check_is_default_group(
    store: &web::Data<Arc<EndpointStore>>,
    group_id: &str,
) -> Result<bool, String> {
    let conn = store.get_conn().await.map_err(|e| e.to_string())?;

    let is_default: bool = conn
        .query_row(
            "SELECT is_default FROM api_groups WHERE id = ?",
            [group_id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;

    Ok(is_default)
}
