use super::EndpointStore;
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::StoreError;
use crate::models::ReferenceData;
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
        let client = self.get_conn().await?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();

        app_log!(info, "Saving reference data for {}", email);

        client
            .execute(
                "INSERT INTO reference_data (id, email, name, data, created_at)
            VALUES ($1, $2, $3, $4, $5)",
                &[&id, &email, &name, &data, &now],
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

    pub async fn get_reference_data(
        &self,
        email: &str,
    ) -> Result<Vec<ReferenceData>, StoreError> {
        let client = self.get_conn().await?;

        let rows = client
            .query(
                "SELECT id, email, name, data, created_at
            FROM reference_data
            WHERE email = $1
            ORDER BY created_at DESC",
                &[&email],
            )
            .await
            .to_store_error()?;

        let mut result = Vec::new();
        for row in rows {
            result.push(ReferenceData {
                id: row.get("id"),
                email: row.get("email"),
                name: row.get("name"),
                data: row.get("data"),
                created_at: row.get("created_at"),
            });
        }

        Ok(result)
    }
}
