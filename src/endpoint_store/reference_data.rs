use super::EndpointStore;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::StoreError;
use crate::infra::models::ReferenceData;
use chrono::Utc;
use graflog::app_log;
use uuid::Uuid;

impl EndpointStore {
    pub async fn save_reference_data(
        &self,
        email: &str,
        name: &str,
        data: &serde_json::Value,
    ) -> Result<ReferenceData, StoreError> {
        let client = self.get_admin_conn().await?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        app_log!(info, "Saving reference data for {}", email);

        client
            .execute(
                "INSERT INTO reference_data (id, email, name, data, created_at)
            VALUES ($1, $2, $3, $4, $5)",
                &[
                    &id as &(dyn tokio_postgres::types::ToSql + Sync),
                    &email as &(dyn tokio_postgres::types::ToSql + Sync),
                    &name as &(dyn tokio_postgres::types::ToSql + Sync),
                    &data as &(dyn tokio_postgres::types::ToSql + Sync),
                    &now as &(dyn tokio_postgres::types::ToSql + Sync),
                ],
            )
            .await
            .to_store_error()?;

        Ok(ReferenceData {
            id,
            email: email.to_string(),
            name: name.to_string(),
            data: data.clone(),
            created_at: now,
        })
    }
}
