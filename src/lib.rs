use duckdb::ToSql;
use duckdb::{Connection, Result as DuckResult};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(#[from] duckdb::Error),
    #[error("Lock error")]
    Lock,
    #[error("Initialization error: {0}")]
    Init(String),
}

// Helper function to provide default verb value
fn default_verb() -> String {
    "GET".to_string()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Parameter {
    pub name: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub alternatives: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Endpoint {
    pub id: String,
    pub text: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default = "default_verb")]
    pub verb: String,
}

#[derive(Debug, Clone)]
pub struct EndpointStore {
    conn: Arc<Mutex<Connection>>,
}

impl EndpointStore {
    pub fn new<P: AsRef<Path>>(db_path: P) -> DuckResult<Self> {
        tracing::info!(
            "Initializing EndpointStore with path: {:?}",
            db_path.as_ref()
        );
        let conn = Connection::open(db_path)?;
        tracing::debug!("DuckDB connection established");

        // Create tables if they don't exist
        conn.execute_batch(
            "
    CREATE TABLE IF NOT EXISTS endpoints (
        id VARCHAR PRIMARY KEY,
        text VARCHAR NOT NULL,
        description VARCHAR NOT NULL,
        is_default BOOLEAN NOT NULL DEFAULT true,
        verb VARCHAR NOT NULL DEFAULT 'GET'
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
        required BOOLEAN NOT NULL DEFAULT false,
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
                "INSERT INTO endpoints (id, text, description, is_default, verb) VALUES (?, ?, ?, true, ?)",
                &[&endpoint.id, &endpoint.text, &endpoint.description, &endpoint.verb],
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
        tracing::info!(email = %email, "Starting to fetch endpoints");

        let conn = self.conn.lock().map_err(|_e| {
            tracing::error!("Failed to acquire database lock");
            StoreError::Lock
        })?;

        // Check if user has custom endpoints
        let has_custom: bool = match conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM user_endpoints WHERE email = ?)",
            [email],
            |row| row.get(0),
        ) {
            Ok(result) => {
                tracing::debug!(email = %email, has_custom = %result, "Checked custom endpoints");
                result
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to check for custom endpoints");
                return Err(StoreError::Database(e));
            }
        };

        let query = if has_custom {
            tracing::debug!(email = %email, "Using custom endpoints query");
            r#"
        SELECT 
            e.id,
            e.text,
            e.description,
            e.verb,
            p.name as param_name,
            p.description as param_description,
            p.required,
            STRING_AGG(pa.alternative, ',') as alternatives
        FROM endpoints e
        INNER JOIN user_endpoints ue ON e.id = ue.endpoint_id
        LEFT JOIN parameters p ON e.id = p.endpoint_id
        LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
        WHERE ue.email = ?
        GROUP BY e.id, e.text, e.description, e.verb, p.name, p.description, p.required
        "#
        } else {
            tracing::debug!("Using default endpoints query");
            r#"
        SELECT 
            e.id,
            e.text,
            e.description,
            e.verb,
            p.name as param_name,
            p.description as param_description,
            p.required,
            STRING_AGG(pa.alternative, ',') as alternatives
        FROM endpoints e
        LEFT JOIN parameters p ON e.id = p.endpoint_id
        LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
        WHERE e.is_default = true
        GROUP BY e.id, e.text, e.description, e.verb, p.name, p.description, p.required
        "#
        };

        let mut stmt = match conn.prepare(query) {
            Ok(stmt) => stmt,
            Err(e) => {
                tracing::error!(error = %e, "Failed to prepare SQL statement");
                return Err(StoreError::Database(e));
            }
        };

        let params: &[&dyn ToSql] = if has_custom { &[&email] } else { &[] };
        tracing::debug!(has_params = has_custom, "Executing query");

        let rows = match stmt.query_map(params, |row| {
            let result = Ok((
                row.get::<_, String>(0)?,         // id
                row.get::<_, String>(1)?,         // text
                row.get::<_, String>(2)?,         // description
                row.get::<_, String>(3)?,         // verb
                row.get::<_, Option<String>>(4)?, // param_name
                row.get::<_, Option<String>>(5)?, // param_description
                row.get::<_, Option<bool>>(6)?,   // required
                row.get::<_, Option<String>>(7)?, // alternatives as comma-separated string
            ));
            if let Err(ref e) = result {
                tracing::error!(error = %e, "Error reading row data");
            }
            result
        }) {
            Ok(rows) => rows,
            Err(e) => {
                tracing::error!(error = %e, "Failed to execute query");
                return Err(StoreError::Database(e));
            }
        };

        let mut endpoints_map = std::collections::HashMap::new();
        for row in rows {
            match row {
                Ok((id, text, description, verb, param_name, param_desc, required, alternatives_str)) => {
                    tracing::trace!(
                        endpoint_id = %id,
                        has_parameter = param_name.is_some(),
                        "Processing endpoint row"
                    );

                    let endpoint = endpoints_map.entry(id.clone()).or_insert_with(|| {
                        tracing::debug!(endpoint_id = %id, "Creating new endpoint entry");
                        Endpoint {
                            id,
                            text,
                            description,
                            verb,
                            parameters: Vec::new(),
                        }
                    });

                    if let (Some(name), Some(desc), Some(req)) = (param_name, param_desc, required)
                    {
                        let alternatives = alternatives_str
                            .map(|s| {
                                let alts = s.split(',').map(String::from).collect::<Vec<_>>();
                                tracing::trace!(
                                    param = %name,
                                    alt_count = alts.len(),
                                    "Processed parameter alternatives"
                                );
                                alts
                            })
                            .unwrap_or_default();

                        endpoint.parameters.push(Parameter {
                            name: name.clone(),
                            description: desc,
                            required: req,
                            alternatives,
                        });
                        tracing::trace!(
                            endpoint_id = %endpoint.id,
                            parameter = %name,
                            required = req,
                            "Added parameter to endpoint"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to process row");
                    return Err(StoreError::Database(e));
                }
            }
        }

        let endpoints = endpoints_map.into_values().collect::<Vec<_>>();
        tracing::info!(
            endpoint_count = endpoints.len(),
            email = %email,
            "Successfully fetched endpoints"
        );

        // Only log endpoint details if we have any
        if !endpoints.is_empty() {
            for endpoint in &endpoints {
                tracing::debug!(
                    id = %endpoint.id,
                    text = %endpoint.text,
                    param_count = endpoint.parameters.len(),
                    "Endpoint details"
                );
            }
        } else {
            tracing::warn!(email = %email, "No endpoints found");
        }

        Ok(endpoints)
    }

    pub async fn replace_user_endpoints(
        &self,
        email: &str,
        endpoints: Vec<Endpoint>,
    ) -> Result<usize, StoreError> {
        let mut conn = self.conn.lock().map_err(|_| StoreError::Lock)?;

        tracing::info!(email = %email, "Starting complete endpoint replacement");

        // First try the force cleanup approach
        match self.force_clean_user_data(email, &mut conn) {
            Ok(_) => {
                tracing::info!(email = %email, "Successfully cleaned up user data");
            }
            Err(e) => {
                tracing::error!(
                    error = %e, 
                    email = %email, 
                    "Failed to clean up user data, will try fallback approach"
                );

                // Try a fallback approach if the force clean fails
                match self.fallback_clean_user_data(email, &mut conn) {
                    Ok(_) => tracing::info!(email = %email, "Fallback cleanup successful"),
                    Err(e) => {
                        tracing::error!(
                            error = %e,
                            email = %email,
                            "Fallback cleanup also failed, proceeding with import anyway"
                        );
                    }
                }
            }
        }

        // Now that we've tried to clean up, proceed with adding the new endpoints
        let tx = conn.transaction()?;
        let mut imported_count = 0;

        for endpoint in &endpoints {
            // Check if endpoint exists
            let exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM endpoints WHERE id = ?)",
                [&endpoint.id],
                |row| row.get(0),
            )?;
            
            if !exists {
                // Create new endpoint
                tracing::debug!(endpoint_id = %endpoint.id, "Creating new endpoint");
                tx.execute(
                    "INSERT INTO endpoints (id, text, description, verb, is_default) VALUES (?, ?, ?, ?, false)",
                    &[&endpoint.id, &endpoint.text, &endpoint.description, &endpoint.verb],
                )?;
            } else {
                // Check if it's a default endpoint
                let is_default: bool = tx.query_row(
                    "SELECT is_default FROM endpoints WHERE id = ?",
                    [&endpoint.id],
                    |row| row.get(0),
                )?;
                
                if !is_default {
                    // Update existing non-default endpoint
                    tracing::debug!(endpoint_id = %endpoint.id, "Updating existing endpoint");
                    tx.execute(
                        "UPDATE endpoints SET text = ?, description = ?, verb = ? WHERE id = ?",
                        &[&endpoint.text, &endpoint.description, &endpoint.verb, &endpoint.id],
                    )?;
                }
            }
            
            // Link to user (ignore if already exists)
            tracing::debug!(email = %email, endpoint_id = %endpoint.id, "Linking endpoint to user");
            tx.execute(
                "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                &[email, &endpoint.id],
            )?;
            
            imported_count += 1;
        }
        
        // Add parameters in a separate loop after all endpoints are created/updated
        for endpoint in &endpoints {
            let is_default: bool = tx.query_row(
                "SELECT is_default FROM endpoints WHERE id = ?",
                [&endpoint.id],
                |row| row.get(0),
            )?;
            
            if !is_default {
                // Try to clean up existing parameters first
                let _ = tx.execute(
                    "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
                    [&endpoint.id],
                );
                
                let _ = tx.execute(
                    "DELETE FROM parameters WHERE endpoint_id = ?",
                    [&endpoint.id],
                );
                
                // Now add the new parameters
                for param in &endpoint.parameters {
                    tracing::debug!(
                        endpoint_id = %endpoint.id, 
                        param = %param.name, 
                        "Adding parameter"
                    );
                    
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
                            "INSERT INTO parameter_alternatives 
                            (endpoint_id, parameter_name, alternative) 
                            VALUES (?, ?, ?)",
                            &[&endpoint.id, &param.name, alt],
                        )?;
                    }
                }
            }
        }
        
        tx.commit()?;
        
        tracing::info!(
            email = %email, 
            count = imported_count, 
            "Successfully imported endpoints"
        );
        
        Ok(imported_count)
    }

    // Fallback cleanup approach - more conservative
    fn fallback_clean_user_data(&self, email: &str, conn: &mut Connection) -> Result<(), StoreError> {
        let tx = conn.transaction()?;
        
        // Get all custom endpoint IDs for this user first
        let mut stmt = tx.prepare(
            "SELECT e.id 
            FROM endpoints e
            JOIN user_endpoints ue ON e.id = ue.endpoint_id
            WHERE ue.email = ? AND e.is_default = false"
        )?;
        
        let endpoint_ids: Vec<String> = stmt
            .query_map([email], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        
        // Remove user-endpoint associations
        for id in &endpoint_ids {
            let _ = tx.execute(
                "DELETE FROM user_endpoints WHERE email = ? AND endpoint_id = ?",
                &[email, id],
            );
        }
        
        // Now check which endpoints are no longer used
        for id in &endpoint_ids {
            let still_used: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM user_endpoints WHERE endpoint_id = ?)",
                [id],
                |row| row.get(0),
            )?;
            
            if !still_used {
                // Remove parameter alternatives first
                let _ = tx.execute(
                    "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
                    [id],
                );
                
                // Then remove parameters
                let _ = tx.execute(
                    "DELETE FROM parameters WHERE endpoint_id = ?",
                    [id],
                );
                
                // Finally remove the endpoint
                let _ = tx.execute(
                    "DELETE FROM endpoints WHERE id = ?",
                    [id],
                );
            }
        }
        
        tx.commit()?;
        Ok(())
    }

    fn force_clean_user_data(&self, email: &str, conn: &mut Connection) -> Result<(), StoreError> {
        // First turn off foreign keys
        conn.execute("PRAGMA foreign_keys=OFF;", [])?;
        
        // Execute the cleaning operations in a transaction
        let tx = conn.transaction()?;
        
        // Create a temporary table to track user's custom endpoints
        tx.execute(
            "CREATE TEMPORARY TABLE user_custom_endpoints AS
            SELECT e.id 
            FROM endpoints e
            JOIN user_endpoints ue ON e.id = ue.endpoint_id
            WHERE ue.email = ? AND e.is_default = false",
            [email],
        )?;
        
        // Delete parameter alternatives
        tx.execute(
            "DELETE FROM parameter_alternatives 
            WHERE endpoint_id IN (SELECT id FROM user_custom_endpoints)",
            [],
        )?;
        
        // Delete parameters
        tx.execute(
            "DELETE FROM parameters
            WHERE endpoint_id IN (SELECT id FROM user_custom_endpoints)",
            [],
        )?;
        
        // Delete user endpoint associations
        tx.execute("DELETE FROM user_endpoints WHERE email = ?", [email])?;
        
        // Delete endpoints that are no longer referenced and not default
        tx.execute(
            "DELETE FROM endpoints 
            WHERE id IN (
                SELECT id FROM user_custom_endpoints
                WHERE id NOT IN (SELECT endpoint_id FROM user_endpoints)
            )",
            [],
        )?;
        
        // Clean up temporary table
        tx.execute("DROP TABLE user_custom_endpoints", [])?;
        
        // Commit the transaction
        tx.commit()?;
        
        // Turn foreign keys back on
        conn.execute("PRAGMA foreign_keys=ON;", [])?;
        
        Ok(())
    }
}
