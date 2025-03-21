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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Parameter {
    pub name: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default)]
    pub required: bool,
    #[serde(default)]
    pub alternatives: Vec<String>,
}

fn default_base_url() -> String {
    "http://localhost:3000".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Endpoint {
    pub id: String,
    pub text: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default = "default_verb")]
    pub verb: String,
    #[serde(default = "default_base_url")]
    pub base_url: String,
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
        verb VARCHAR NOT NULL DEFAULT 'GET',
        base_url VARCHAR NOT NULL DEFAULT 'http://localhost:3000'
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
                "INSERT INTO endpoints (id, text, description, is_default, verb, base_url) VALUES (?, ?, ?, true, ?, ?)",
                &[&endpoint.id, &endpoint.text, &endpoint.description, &endpoint.verb, &endpoint.base_url],
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

    pub async fn get_or_create_user_endpoints(
        &self,
        email: &str,
    ) -> Result<Vec<Endpoint>, StoreError> {
        // Check if user has custom endpoints
        let has_custom: bool = {
            let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM user_endpoints WHERE email = ?)",
                [email],
                |row| row.get(0),
            )?
        };

        // If user doesn't have custom endpoints, create them from defaults
        if !has_custom {
            tracing::info!(email = %email, "User has no endpoints, creating defaults");

            // Get default endpoints
            let default_endpoints: Vec<Endpoint> = {
                let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;

                // Get the default endpoints
                let mut stmt = conn.prepare(r#"
                SELECT 
                    e.id,
                    e.text,
                    e.description,
                    e.verb,
                    e.base_url,
                    p.name as param_name,
                    p.description as param_description,
                    p.required,
                    STRING_AGG(pa.alternative, ',') as alternatives
                FROM endpoints e
                LEFT JOIN parameters p ON e.id = p.endpoint_id
                LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
                WHERE e.is_default = true
                GROUP BY e.id, e.text, e.description, e.verb, e.base_url, p.name, p.description, p.required
            "#)?;

                let rows = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,         // id
                        row.get::<_, String>(1)?,         // text
                        row.get::<_, String>(2)?,         // description
                        row.get::<_, String>(3)?,         // verb
                        row.get::<_, String>(4)?,         // base_url
                        row.get::<_, Option<String>>(5)?, // param_name
                        row.get::<_, Option<String>>(6)?, // param_description
                        row.get::<_, Option<bool>>(7)?,   // required
                        row.get::<_, Option<String>>(8)?, // alternatives
                    ))
                })?;

                // Process rows into endpoints
                let mut endpoints_map = std::collections::HashMap::new();
                for row in rows {
                    let (
                        id,
                        text,
                        description,
                        verb,
                        base_url,
                        param_name,
                        param_desc,
                        required,
                        alternatives_str,
                    ) = row?;

                    let endpoint = endpoints_map.entry(id.clone()).or_insert_with(|| Endpoint {
                        id,
                        text,
                        description,
                        verb,
                        base_url,
                        parameters: Vec::new(),
                    });

                    if let (Some(name), Some(desc), Some(req)) = (param_name, param_desc, required)
                    {
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

                endpoints_map.into_values().collect()
            };

            // Create user associations for each default endpoint
            {
                let mut conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
                let tx = conn.transaction()?;

                for endpoint in &default_endpoints {
                    tx.execute(
                        "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                        &[email, &endpoint.id],
                    )?;
                }

                tx.commit()?;
            }

            tracing::info!(
                email = %email,
                count = default_endpoints.len(),
                "Created default endpoints for user"
            );
        }

        // Now get and return the user's endpoints
        self.get_endpoints_by_email(email)
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
            e.base_url,
            p.name as param_name,
            p.description as param_description,
            CASE WHEN p.required IS NULL THEN 'false' ELSE p.required END as required,
            STRING_AGG(pa.alternative, ',') as alternatives
        FROM endpoints e
        INNER JOIN user_endpoints ue ON e.id = ue.endpoint_id
        LEFT JOIN parameters p ON e.id = p.endpoint_id
        LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
        WHERE ue.email = ?
        GROUP BY e.id, e.text, e.description, e.verb, e.base_url, p.name, p.description, p.required
        "#
        } else {
            tracing::debug!("Using default endpoints query");
            r#"
        SELECT 
            e.id,
            e.text,
            e.description,
            e.verb,
            e.base_url,
            p.name as param_name,
            p.description as param_description,
            CASE WHEN p.required IS NULL THEN 'false' ELSE p.required END as required,
            STRING_AGG(pa.alternative, ',') as alternatives
        FROM endpoints e
        LEFT JOIN parameters p ON e.id = p.endpoint_id
        LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
        WHERE e.is_default = true
        GROUP BY e.id, e.text, e.description, e.verb, e.base_url, p.name, p.description, p.required
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
            let required: String = row.get(7).unwrap_or_else(|_| "false".to_string());
            let required_bool = required == "true" || required == "1";

            Ok((
                row.get::<_, String>(0)?,         // id
                row.get::<_, String>(1)?,         // text
                row.get::<_, String>(2)?,         // description
                row.get::<_, String>(3)?,         // verb
                row.get::<_, String>(4)?,         // base_url
                row.get::<_, Option<String>>(5)?, // param_name
                row.get::<_, Option<String>>(6)?, // param_description
                required_bool,                    // required (converted to bool)
                row.get::<_, Option<String>>(8)?, // alternatives
            ))
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
                Ok((
                    id,
                    text,
                    description,
                    verb,
                    base_url,
                    param_name,
                    param_desc,
                    required,
                    alternatives_str,
                )) => {
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
                            base_url,
                            parameters: Vec::new(),
                        }
                    });

                    if let (Some(name), Some(desc), req) = (param_name, param_desc, required) {
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
                            required,
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
                        "UPDATE endpoints SET text = ?, description = ?, verb = ?, base_url = ? WHERE id = ?",
                        &[
                            &endpoint.text,
                            &endpoint.description,
                            &endpoint.verb,
                            &endpoint.base_url,
                            &endpoint.id,
                        ],
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
                            &(if param.required { "1" } else { "0" }).to_string(),
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

    pub async fn add_user_endpoint(
        &self,
        email: &str,
        endpoint: Endpoint,
    ) -> Result<bool, StoreError> {
        let mut conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        let tx = conn.transaction()?;

        tracing::info!(
            email = %email,
            endpoint_id = %endpoint.id,
            "Starting endpoint addition"
        );

        // Check if endpoint with this ID already exists
        let endpoint_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM endpoints WHERE id = ?)",
            [&endpoint.id],
            |row| row.get(0),
        )?;

        if endpoint_exists {
            // Check if it's a default endpoint
            let is_default: bool = tx.query_row(
                "SELECT is_default FROM endpoints WHERE id = ?",
                [&endpoint.id],
                |row| row.get(0),
            )?;

            if is_default {
                // For default endpoints, just create the association
                tx.execute(
                    "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                    [email, &endpoint.id],
                )?;

                tracing::info!(
                    email = %email,
                    endpoint_id = %endpoint.id,
                    "Added association to existing default endpoint"
                );
            } else {
                // For non-default endpoints, check if the user already has it
                let user_has_endpoint: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM user_endpoints WHERE email = ? AND endpoint_id = ?)",
                [email, &endpoint.id],
                |row| row.get(0),
            )?;

                if user_has_endpoint {
                    // User already has this endpoint, update it
                    tx.execute(
                        "UPDATE endpoints SET text = ?, description = ?, verb = ? WHERE id = ?",
                        &[
                            &endpoint.text,
                            &endpoint.description,
                            &endpoint.verb,
                            &endpoint.id,
                        ],
                    )?;

                    tracing::info!(
                        email = %email,
                        endpoint_id = %endpoint.id,
                        "Updated existing user endpoint"
                    );
                } else {
                    // User doesn't have this endpoint, create the association
                    tx.execute(
                        "INSERT INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                        [email, &endpoint.id],
                    )?;

                    tracing::info!(
                        email = %email,
                        endpoint_id = %endpoint.id,
                        "Added association to existing non-default endpoint"
                    );
                }
            }
        } else {
            // Endpoint doesn't exist, create it
            tx.execute(
                "INSERT INTO endpoints (id, text, description, verb, base_url, is_default) VALUES (?, ?, ?, ?, ?, false)",
                &[
                    &endpoint.id,
                    &endpoint.text,
                    &endpoint.description,
                    &endpoint.verb,
                    &endpoint.base_url,
                ],
            )?;

            // Create the user association
            tx.execute(
                "INSERT INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                [email, &endpoint.id],
            )?;

            tracing::info!(
                email = %email,
                endpoint_id = %endpoint.id,
                "Created new endpoint and association"
            );
        }

        // Clean up existing parameters if it's not a default endpoint
        let is_default: bool = tx.query_row(
            "SELECT is_default FROM endpoints WHERE id = ?",
            [&endpoint.id],
            |row| row.get(0),
        )?;

        if !is_default {
            // Delete existing parameter alternatives
            tx.execute(
                "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
                [&endpoint.id],
            )?;

            // Delete existing parameters
            tx.execute(
                "DELETE FROM parameters WHERE endpoint_id = ?",
                [&endpoint.id],
            )?;

            // Add new parameters
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

                // Add parameter alternatives
                for alt in &param.alternatives {
                    tx.execute(
                    "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) 
                    VALUES (?, ?, ?)",
                    &[&endpoint.id, &param.name, alt],
                )?;
                }
            }

            tracing::debug!(
                endpoint_id = %endpoint.id,
                param_count = endpoint.parameters.len(),
                "Added parameters to endpoint"
            );
        }

        tx.commit()?;

        tracing::info!(
            email = %email,
            endpoint_id = %endpoint.id,
            "Successfully processed endpoint addition"
        );

        Ok(true)
    }

    pub async fn delete_user_endpoint(
        &self,
        email: &str,
        endpoint_id: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;

        tracing::info!(
            email = %email,
            endpoint_id = %endpoint_id,
            "Starting endpoint deletion"
        );

        // First, check if we can find out all the tables with foreign key references to endpoints
        let _tables = vec!["parameter_alternatives", "parameters", "user_endpoints"];

        // Remove just the user-endpoint association first
        match conn.execute(
            "DELETE FROM user_endpoints WHERE email = ? AND endpoint_id = ?",
            [email, endpoint_id],
        ) {
            Ok(_) => {
                tracing::info!(
                    email = %email,
                    endpoint_id = %endpoint_id,
                    "Removed user-endpoint association"
                );
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    endpoint_id = %endpoint_id,
                    "Failed to remove user-endpoint association"
                );
                return Err(StoreError::Database(e));
            }
        }

        // Check if the endpoint is still used by any user
        let still_used: bool = match conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM user_endpoints WHERE endpoint_id = ?)",
            [endpoint_id],
            |row| row.get(0),
        ) {
            Ok(exists) => exists,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    endpoint_id = %endpoint_id,
                    "Failed to check if endpoint is still used"
                );
                return Err(StoreError::Database(e));
            }
        };

        if still_used {
            tracing::info!(
                endpoint_id = %endpoint_id,
                "Endpoint still used by other users, keeping it but removing user association"
            );
            return Ok(true);
        }

        // If we get here, no user is using this endpoint anymore, so we should delete it
        tracing::info!(
            endpoint_id = %endpoint_id,
            "No users left using this endpoint, attempting to delete it"
        );

        // Try to remove related data with explicit error handling for each step
        let mut success = true;

        // Delete from parameter_alternatives
        match conn.execute(
            "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
            [endpoint_id],
        ) {
            Ok(count) => {
                tracing::info!(
                    endpoint_id = %endpoint_id,
                    count = count,
                    "Deleted parameter alternatives"
                );
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    endpoint_id = %endpoint_id,
                    "Failed to delete parameter alternatives"
                );
                success = false;
            }
        }

        // Only proceed if previous step was successful
        if success {
            // Delete from parameters
            match conn.execute(
                "DELETE FROM parameters WHERE endpoint_id = ?",
                [endpoint_id],
            ) {
                Ok(count) => {
                    tracing::info!(
                        endpoint_id = %endpoint_id,
                        count = count,
                        "Deleted parameters"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        endpoint_id = %endpoint_id,
                        "Failed to delete parameters"
                    );
                    success = false;
                }
            }
        }

        // Only proceed if previous steps were successful
        if success {
            // Finally, delete the endpoint itself
            match conn.execute("DELETE FROM endpoints WHERE id = ?", [endpoint_id]) {
                Ok(count) => {
                    if count > 0 {
                        tracing::info!(
                            endpoint_id = %endpoint_id,
                            "Successfully deleted endpoint"
                        );
                    } else {
                        tracing::warn!(
                            endpoint_id = %endpoint_id,
                            "Endpoint not found for deletion"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        endpoint_id = %endpoint_id,
                        "Failed to delete endpoint"
                    );
                    success = false;
                }
            }
        }

        // If any step failed, log a warning
        if !success {
            tracing::warn!(
                endpoint_id = %endpoint_id,
                "Could not completely delete endpoint and all its related data due to constraints"
            );
        }

        // We successfully removed the user-endpoint association at minimum
        Ok(true)
    }

    // Fallback cleanup approach - more conservative
    fn fallback_clean_user_data(
        &self,
        email: &str,
        conn: &mut Connection,
    ) -> Result<(), StoreError> {
        let tx = conn.transaction()?;

        // Get all custom endpoint IDs for this user first
        let mut stmt = tx.prepare(
            "SELECT e.id 
            FROM endpoints e
            JOIN user_endpoints ue ON e.id = ue.endpoint_id
            WHERE ue.email = ? AND e.is_default = false",
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
                let _ = tx.execute("DELETE FROM parameters WHERE endpoint_id = ?", [id]);

                // Finally remove the endpoint
                let _ = tx.execute("DELETE FROM endpoints WHERE id = ?", [id]);
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
