use duckdb::ToSql;
use duckdb::{Connection, Result as DuckResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};
//use thiserror::Error;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(#[from] duckdb::Error),
    #[error("Lock error")]
    Lock,
    #[error("Initialization error: {0}")]
    Init(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub alternatives: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: String,
    pub text: String,
    pub description: String,
    pub parameters: Vec<Parameter>,
}

#[derive(Debug, Clone)]
pub struct EndpointStore {
    conn: Arc<Mutex<Connection>>,
}

impl EndpointStore {
    pub fn new<P: AsRef<Path>>(db_path: P) -> DuckResult<Self> {
        let conn = Connection::open(db_path)?;

        // Create tables if they don't exist
        conn.execute_batch(
            "
    CREATE TABLE IF NOT EXISTS endpoints (
        id VARCHAR PRIMARY KEY,
        text VARCHAR NOT NULL,
        description VARCHAR NOT NULL,
        is_default BOOLEAN NOT NULL DEFAULT true
    );
    
    CREATE TABLE IF NOT EXISTS user_endpoints (
        email VARCHAR NOT NULL,
        endpoint_id VARCHAR NOT NULL,
        FOREIGN KEY (endpoint_id) REFERENCES endpoints(id),
        PRIMARY KEY (email, endpoint_id)
    );
    
    CREATE TABLE IF NOT EXISTS parameters (
        endpoint_id VARCHAR,
        name VARCHAR NOT NULL,
        description VARCHAR NOT NULL,
        required BOOLEAN NOT NULL,
        FOREIGN KEY (endpoint_id) REFERENCES endpoints(id)
    );
    
    CREATE TABLE IF NOT EXISTS parameter_alternatives (
        endpoint_id VARCHAR,
        parameter_name VARCHAR,
        alternative VARCHAR NOT NULL,
        FOREIGN KEY (endpoint_id) REFERENCES endpoints(id)
    );
    ",
        )?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn initialize_if_empty(
        &mut self,
        default_endpoints: &[Endpoint],
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        // Check if we already have default endpoints
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM endpoints WHERE is_default = true",
            [],
            |row| row.get(0),
        )?;

        if count > 0 {
            return Ok(());
        }

        let tx = conn.transaction()?;

        // Insert new endpoints
        for endpoint in default_endpoints {
            tx.execute(
                "INSERT INTO endpoints (id, text, description, is_default) VALUES (?, ?, ?, true)",
                &[&endpoint.id, &endpoint.text, &endpoint.description],
            )?;

            for param in &endpoint.parameters {
                tx.execute(
                    "INSERT INTO parameters (endpoint_id, name, description, required) 
                    VALUES (?, ?, ?, ?)",
                    &[
                        &endpoint.id,
                        &param.name,
                        &param.description,
                        &param.required.to_string(),
                    ],
                )?;

                for alt in &param.alternatives {
                    tx.execute(
                        "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) 
                        VALUES (?, ?, ?)",
                        &[&endpoint.id, &param.name, alt],
                    )?;
                }
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn get_endpoints_by_email(&self, email: &str) -> Result<Vec<Endpoint>, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        // Check if user has custom endpoints
        let has_custom: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM user_endpoints WHERE email = ?)",
            [email],
            |row| row.get(0),
        )?;

        let query = if has_custom {
            // Get user's custom endpoints
            r#"
        SELECT 
            e.id,
            e.text,
            e.description,
            p.name as param_name,
            p.description as param_description,
            p.required,
            STRING_AGG(pa.alternative, ',') as alternatives
        FROM endpoints e
        INNER JOIN user_endpoints ue ON e.id = ue.endpoint_id
        LEFT JOIN parameters p ON e.id = p.endpoint_id
        LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
        WHERE ue.email = ?
        GROUP BY e.id, e.text, e.description, p.name, p.description, p.required
        "#
        } else {
            // Get default endpoints
            r#"
        SELECT 
            e.id,
            e.text,
            e.description,
            p.name as param_name,
            p.description as param_description,
            p.required,
            STRING_AGG(pa.alternative, ',') as alternatives
        FROM endpoints e
        LEFT JOIN parameters p ON e.id = p.endpoint_id
        LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
        WHERE e.is_default = true
        GROUP BY e.id, e.text, e.description, p.name, p.description, p.required
        "#
        };

        let mut stmt = conn.prepare(query)?;
        let params: &[&dyn ToSql] = if has_custom { &[&email] } else { &[] };

        let rows = stmt.query_map(params, |row| {
            Ok((
                row.get::<_, String>(0)?,         // id
                row.get::<_, String>(1)?,         // text
                row.get::<_, String>(2)?,         // description
                row.get::<_, Option<String>>(3)?, // param_name
                row.get::<_, Option<String>>(4)?, // param_description
                row.get::<_, Option<bool>>(5)?,   // required
                row.get::<_, Option<String>>(6)?, // alternatives as comma-separated string
            ))
        })?;

        let mut endpoints_map = std::collections::HashMap::new();

        for row in rows {
            let (id, text, description, param_name, param_desc, required, alternatives_str) = row?;

            let endpoint = endpoints_map.entry(id.clone()).or_insert_with(|| Endpoint {
                id,
                text,
                description,
                parameters: Vec::new(),
            });

            if let (Some(name), Some(desc), Some(req)) = (param_name, param_desc, required) {
                let alternatives = alternatives_str
                    .map(|s| s.split(',').map(String::from).collect())
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
}
