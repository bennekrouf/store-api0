use duckdb::ToSql;
use duckdb::{Connection, Result as DuckResult};
use serde::{Deserialize, Serialize};
use slug::slugify;
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

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
    #[serde(default = "generate_uuid")]
    pub id: String,
    pub text: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default = "default_verb")]
    #[serde(alias = "method")] // Allow 'method' as an alternative name
    pub verb: String,
    #[serde(default = "default_base_url")]
    #[serde(alias = "base")] // Handle camelCase from frontend
    pub base: String,
    #[serde(default = "String::new")]
    pub path: String,
    #[serde(default = "String::new")]
    pub group_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiGroup {
    #[serde(default = "generate_uuid")]
    pub id: String,
    pub name: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default = "default_base_url")]
    pub base: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiGroupWithEndpoints {
    #[serde(flatten)]
    pub group: ApiGroup,
    pub endpoints: Vec<Endpoint>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiStorage {
    pub api_groups: Vec<ApiGroupWithEndpoints>,
}

fn generate_uuid() -> String {
    Uuid::new_v4().to_string()
}

// Generate ID from text using slugify and UUID for uniqueness
pub fn generate_id_from_text(text: &str) -> String {
    let slug = slugify(text);
    if slug.is_empty() {
        return generate_uuid();
    }

    // Create a shorter UUID suffix (first 8 chars)
    let uuid_short = Uuid::new_v4()
        .to_string()
        .split('-')
        .next()
        .unwrap_or("")
        .to_string();
    format!("{}-{}", slug, uuid_short)
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

        // Create tables with the new schema
        conn.execute_batch(include_str!("../sql/schema.sql"))?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub fn initialize_if_empty(
        &mut self,
        default_api_groups: &[ApiGroupWithEndpoints],
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().map_err(|_| StoreError::Lock)?;

        // Check if we already have default endpoints
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM api_groups WHERE is_default = true",
            [],
            |row| row.get(0),
        )?;

        if count > 0 {
            // Default API groups already exist, no need to create them
            tracing::info!("Default API groups already exist. Skipping initialization.");
            return Ok(());
        }

        tracing::info!("Initializing database with default API groups and endpoints");

        // Start a transaction for the entire initialization process
        let tx = conn.transaction()?;

        // Create default API groups and endpoints
        for group_with_endpoints in default_api_groups {
            let group = &group_with_endpoints.group;

            // Insert the API group
            tx.execute(
            "INSERT INTO api_groups (id, name, description, base, is_default) VALUES (?, ?, ?, ?, true)",
            &[
                &group.id,
                &group.name,
                &group.description,
                &group.base,
            ],
        )?;

            tracing::debug!(
                group_id = %group.id,
                group_name = %group.name,
                "Inserted API group"
            );

            // Insert endpoints for this group
            for endpoint in &group_with_endpoints.endpoints {
                // Generate ID if not provided
                let endpoint_id = if endpoint.id.is_empty() {
                    generate_id_from_text(&endpoint.text)
                } else {
                    endpoint.id.clone()
                };

                // Insert endpoint
                tx.execute(
                "INSERT INTO endpoints (id, text, description, verb, base, path, group_id, is_default) 
                 VALUES (?, ?, ?, ?, ?, ?, ?, true)",
                &[
                    &endpoint_id,
                    &endpoint.text,
                    &endpoint.description,
                    &endpoint.verb,
                    &endpoint.base,
                    &endpoint.path,
                    &group.id,
                ],
            )?;

                tracing::debug!(
                    endpoint_id = %endpoint_id,
                    endpoint_text = %endpoint.text,
                    "Inserted endpoint"
                );

                // Insert parameters
                for param in &endpoint.parameters {
                    tx.execute(
                        "INSERT INTO parameters (endpoint_id, name, description, required) 
                     VALUES (?, ?, ?, ?)",
                        &[
                            &endpoint_id,
                            &param.name,
                            &param.description,
                            &param.required.to_string(),
                        ],
                    )?;

                    // Insert parameter alternatives
                    for alt in &param.alternatives {
                        tx.execute(
                        "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) 
                         VALUES (?, ?, ?)",
                        &[&endpoint_id, &param.name, alt],
                    )?;
                    }
                }
            }
        }

        // Create a default user for testing if none exists
        // This is optional but helpful during development
        let default_email = "default@example.com";

        // Associate default groups with the default user
        for group_with_endpoints in default_api_groups {
            // Associate group with default user
            tx.execute(
                "INSERT OR IGNORE INTO user_groups (email, group_id) VALUES (?, ?)",
                &[default_email, &group_with_endpoints.group.id],
            )?;

            // Associate endpoints with default user
            for endpoint in &group_with_endpoints.endpoints {
                tx.execute(
                    "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                    &[default_email, &endpoint.id],
                )?;
            }
        }

        // Commit the transaction
        tx.commit()?;

        tracing::info!(
            group_count = default_api_groups.len(),
            "Successfully initialized database with default API groups and endpoints"
        );

        Ok(())
    }

    // Add this method to ensure any user has access to default groups
    pub async fn ensure_user_has_default_groups(&self, email: &str) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().map_err(|_| StoreError::Lock)?;

        // Check if user already has groups
        let has_groups: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM user_groups WHERE email = ?)",
            [email],
            |row| row.get(0),
        )?;

        if has_groups {
            // User already has group associations
            return Ok(());
        }

        tracing::info!(email = %email, "Creating default group associations for user");

        // Get all default group IDs
        let mut stmt = conn.prepare("SELECT id FROM api_groups WHERE is_default = true")?;

        let group_ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        if group_ids.is_empty() {
            tracing::warn!("No default API groups found in database");
            return Ok(());
        }

        // Create associations in a transaction
        let tx = conn.transaction()?;

        for group_id in &group_ids {
            tx.execute(
                "INSERT INTO user_groups (email, group_id) VALUES (?, ?)",
                &[email, group_id],
            )?;

            // Also associate the user with all endpoints in this group
            let mut endpoint_stmt = tx.prepare("SELECT id FROM endpoints WHERE group_id = ?")?;

            let endpoint_ids: Vec<String> = endpoint_stmt
                .query_map([group_id], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;

            for endpoint_id in &endpoint_ids {
                tx.execute(
                    "INSERT INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                    &[email, endpoint_id],
                )?;
            }
        }

        tx.commit()?;

        tracing::info!(
            email = %email,
            group_count = group_ids.len(),
            "Successfully created default group associations"
        );

        Ok(())
    }

    // Get or create user groups and endpoints
    pub async fn get_or_create_user_api_groups(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        let mut conn = self.conn.lock().map_err(|_| StoreError::Lock)?;

        // Check if user has custom groups
        let has_custom: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM user_groups WHERE email = ?)",
            [email],
            |row| row.get(0),
        )?;

        // If user doesn't have custom groups, create them from defaults
        if !has_custom {
            tracing::info!(email = %email, "User has no API groups, creating defaults");

            // Get default groups
            let default_groups = self.get_default_api_groups(&conn)?;

            // Debug log to check if defaults are found
            tracing::info!(
                email = %email,
                group_count = default_groups.len(),
                "Found default API groups to create"
            );

            if default_groups.is_empty() {
                tracing::warn!(email = %email, "No default API groups found");
                // You might want to create at least one basic default group here
            }

            // Create user associations within a transaction
            let tx = conn.transaction()?;
            for group in &default_groups {
                // Associate group with user
                tx.execute(
                    "INSERT OR IGNORE INTO user_groups (email, group_id) VALUES (?, ?)",
                    &[email, &group.group.id],
                )?;

                // Associate each endpoint with user
                for endpoint in &group.endpoints {
                    tx.execute(
                        "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                        &[email, &endpoint.id],
                    )?;
                }
            }
            tx.commit()?;

            tracing::info!(
                email = %email,
                count = default_groups.len(),
                "Created default API groups for user"
            );
        }

        // Now get and return the user's groups using the same connection
        self.get_api_groups_by_email(email, &conn)
    }

    // Get default API groups and their endpoints
    fn get_default_api_groups(
        &self,
        conn: &Connection,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        tracing::info!("Fetching default API groups from database");

        // First check if there are any default groups
        let default_count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM api_groups WHERE is_default = true",
            [],
            |row| row.get(0),
        )?;

        tracing::info!(
            count = default_count,
            "Found default API groups in database"
        );

        if default_count == 0 {
            tracing::warn!("No default API groups found in database");
            return Ok(Vec::new());
        }

        // Get all default groups
        let mut stmt = match conn
            .prepare("SELECT id, name, description, base FROM api_groups WHERE is_default = true")
        {
            Ok(stmt) => stmt,
            Err(e) => {
                tracing::error!(error = %e, "Failed to prepare statement for fetching default groups");
                return Err(StoreError::Database(e));
            }
        };

        let groups = match stmt.query_map([], |row| {
            Ok(ApiGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                base: row.get(3)?,
            })
        }) {
            Ok(groups) => groups,
            Err(e) => {
                tracing::error!(error = %e, "Failed to query default groups");
                return Err(StoreError::Database(e));
            }
        };

        let mut result = Vec::new();
        for group_result in groups {
            match group_result {
                Ok(group) => {
                    tracing::debug!(
                        group_id = %group.id,
                        group_name = %group.name,
                        "Processing default group"
                    );

                    // For each group, get its endpoints
                    match self.get_endpoints_by_group_id(&group.id, &conn) {
                        Ok(endpoints) => {
                            tracing::debug!(
                                group_id = %group.id,
                                endpoint_count = endpoints.len(),
                                "Retrieved endpoints for group"
                            );

                            result.push(ApiGroupWithEndpoints { group, endpoints });
                        }
                        Err(e) => {
                            tracing::error!(
                                error = %e,
                                group_id = %group.id,
                                "Failed to get endpoints for group"
                            );
                            return Err(e);
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to process group result");
                    return Err(StoreError::Database(e));
                }
            }
        }

        tracing::info!(
            group_count = result.len(),
            "Successfully retrieved default API groups"
        );

        // Log details of each group for debugging
        for (i, group_with_endpoints) in result.iter().enumerate() {
            tracing::debug!(
                index = i,
                group_id = %group_with_endpoints.group.id,
                group_name = %group_with_endpoints.group.name,
                endpoint_count = group_with_endpoints.endpoints.len(),
                "Default group details"
            );
        }

        Ok(result)
    }

    // Get endpoints for a specific group
    fn get_endpoints_by_group_id(
        &self,
        group_id: &str,
        conn: &Connection,
    ) -> Result<Vec<Endpoint>, StoreError> {
        tracing::debug!(
            group_id = %group_id,
            "Fetching endpoints for group"
        );

        //let conn = self.conn.lock().map_err(|_| {
        //    tracing::error!("Failed to acquire database lock in get_endpoints_by_group_id");
        //    StoreError::Lock
        //})?;

        println!("After the conn lock");

        // Check if there are any endpoints for this group
        let endpoint_count: i64 = match conn.query_row(
            "SELECT COUNT(*) FROM endpoints WHERE group_id = ?",
            [group_id],
            |row| row.get(0),
        ) {
            Ok(count) => count,
            Err(e) => {
                tracing::error!(
                    error = %e,
                    group_id = %group_id,
                    "Failed to count endpoints for group"
                );
                return Err(StoreError::Database(e));
            }
        };

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

        let mut stmt = match conn.prepare(r#"
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
            return Err(StoreError::Database(e));
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
                return Err(StoreError::Database(e));
            }
        };

        // Process rows into endpoints
        let mut endpoints_map = std::collections::HashMap::new();
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
                    return Err(StoreError::Database(e));
                }
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

    // Get all API groups and endpoints for a user
    pub fn get_api_groups_by_email(
        &self,
        email: &str,
        conn: &Connection,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        tracing::info!(email = %email, "Starting to fetch API groups and endpoints");

        //let conn = self.conn.lock().map_err(|_| StoreError::Lock)?;

        // Check if user has custom groups
        let has_custom: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM user_groups WHERE email = ?)",
            [email],
            |row| row.get(0),
        )?;

        // Get user's groups (either custom or default)
        let groups_query = if has_custom {
            r#"
            SELECT g.id, g.name, g.description, g.base
            FROM api_groups g
            INNER JOIN user_groups ug ON g.id = ug.group_id
            WHERE ug.email = ?
            "#
        } else {
            r#"
            SELECT g.id, g.name, g.description, g.base
            FROM api_groups g
            WHERE g.is_default = true
            "#
        };

        let mut stmt = conn.prepare(groups_query)?;
        let params: &[&dyn ToSql] = if has_custom { &[&email] } else { &[] };

        let groups = stmt.query_map(params, |row| {
            Ok(ApiGroup {
                id: row.get(0)?,
                name: row.get(1)?,
                description: row.get(2)?,
                base: row.get(3)?,
            })
        })?;

        let mut result = Vec::new();
        for group_result in groups {
            let group = group_result?;

            tracing::debug!(
                group_id = %group.id,
                group_name = %group.name,
                "Processing group"
            );

            // Get endpoints for this group
            let endpoints_query = if has_custom {
                r#"
                SELECT 
                    e.id, e.text, e.description, e.verb, e.base, e.path,
                    p.name, p.description, p.required, STRING_AGG(pa.alternative, ',') as alternatives
                FROM endpoints e
                INNER JOIN user_endpoints ue ON e.id = ue.endpoint_id
                LEFT JOIN parameters p ON e.id = p.endpoint_id
                LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
                WHERE ue.email = ? AND e.group_id = ?
                GROUP BY e.id, e.text, e.description, e.verb, e.base, e.path, p.name, p.description, p.required
                "#
            } else {
                r#"
                SELECT 
                    e.id, e.text, e.description, e.verb, e.base, e.path,
                    p.name, p.description, p.required, STRING_AGG(pa.alternative, ',') as alternatives
                FROM endpoints e
                LEFT JOIN parameters p ON e.id = p.endpoint_id
                LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
                WHERE e.is_default = true AND e.group_id = ?
                GROUP BY e.id, e.text, e.description, e.verb, e.base, e.path, p.name, p.description, p.required
                "#
            };

            let mut endpoints_stmt = conn.prepare(endpoints_query)?;
            let endpoints_params: Vec<&dyn ToSql> = if has_custom {
                vec![&email, &group.id]
            } else {
                vec![&group.id]
            };

            let endpoint_rows = endpoints_stmt.query_map(&endpoints_params[..], |row| {
                Ok((
                    row.get::<_, String>(0)?,         // id
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
            })?;

            // Process endpoint rows
            let mut endpoints_map = std::collections::HashMap::new();
            for row_result in endpoint_rows {
                let (
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
                ) = row_result?;

                let endpoint = endpoints_map.entry(id.clone()).or_insert_with(|| Endpoint {
                    id,
                    text,
                    description,
                    verb,
                    base,
                    path,
                    parameters: Vec::new(),
                    group_id: group.id.clone(),
                });

                if let (Some(name), Some(desc), Some(req)) = (param_name, param_desc, required) {
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

            let group_endpoints: Vec<Endpoint> = endpoints_map.into_values().collect();
            tracing::debug!(
                group_id = %group.id,
                endpoint_count = group_endpoints.len(),
                "Added endpoints to group"
            );

            result.push(ApiGroupWithEndpoints {
                group,
                endpoints: group_endpoints,
            });
        }

        tracing::info!(
            group_count = result.len(),
            email = %email,
            "Successfully fetched API groups and endpoints"
        );

        Ok(result)
    }

    // Replace all user API groups and endpoints
    pub async fn replace_user_api_groups(
        &self,
        email: &str,
        api_groups: Vec<ApiGroupWithEndpoints>,
    ) -> Result<usize, StoreError> {
        let mut conn = self.conn.lock().map_err(|_| StoreError::Lock)?;

        tracing::info!(email = %email, "Starting complete API group replacement");

        // Clean up existing user data
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

                // Fallback approach
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

        // Add new groups and endpoints
        let tx = conn.transaction()?;
        let mut imported_count = 0;

        for group_with_endpoints in &api_groups {
            let group = &group_with_endpoints.group;

            // Generate ID if not provided
            let group_id = if group.id.is_empty() {
                generate_id_from_text(&group.name)
            } else {
                group.id.clone()
            };

            // Check if group exists
            let group_exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM api_groups WHERE id = ?)",
                [&group_id],
                |row| row.get(0),
            )?;

            if !group_exists {
                // Create new group
                tracing::debug!(group_id = %group_id, "Creating new API group");
                tx.execute(
                    "INSERT INTO api_groups (id, name, description, base, is_default) VALUES (?, ?, ?, ?, false)",
                    &[&group_id, &group.name, &group.description, &group.base],
                )?;
            } else {
                // Check if it's a default group
                let is_default: bool = tx.query_row(
                    "SELECT is_default FROM api_groups WHERE id = ?",
                    [&group_id],
                    |row| row.get(0),
                )?;

                if !is_default {
                    // Update existing non-default group
                    tracing::debug!(group_id = %group_id, "Updating existing API group");
                    tx.execute(
                        "UPDATE api_groups SET name = ?, description = ?, base = ? WHERE id = ?",
                        &[&group.name, &group.description, &group.base, &group_id],
                    )?;
                }
            }

            // Link group to user
            tx.execute(
                "INSERT OR IGNORE INTO user_groups (email, group_id) VALUES (?, ?)",
                &[email, &group_id],
            )?;

            // Process endpoints for this group
            for endpoint in &group_with_endpoints.endpoints {
                // Generate ID if not provided
                let endpoint_id = if endpoint.id.is_empty() {
                    generate_id_from_text(&endpoint.text)
                } else {
                    endpoint.id.clone()
                };

                // Check if endpoint exists
                let endpoint_exists: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM endpoints WHERE id = ?)",
                    [&endpoint_id],
                    |row| row.get(0),
                )?;

                if !endpoint_exists {
                    // Create new endpoint
                    tracing::debug!(endpoint_id = %endpoint_id, "Creating new endpoint");
                    tx.execute(
                        "INSERT INTO endpoints (id, text, description, verb, base, path, group_id, is_default) 
                         VALUES (?, ?, ?, ?, ?, ?, ?, false)",
                        &[
                            &endpoint_id,
                            &endpoint.text,
                            &endpoint.description,
                            &endpoint.verb,
                            &endpoint.base,
                            &endpoint.path,
                            &group_id,
                        ],
                    )?;
                } else {
                    // Check if it's a default endpoint
                    let is_default: bool = tx.query_row(
                        "SELECT is_default FROM endpoints WHERE id = ?",
                        [&endpoint_id],
                        |row| row.get(0),
                    )?;

                    if !is_default {
                        // Update existing non-default endpoint
                        tracing::debug!(endpoint_id = %endpoint_id, "Updating existing endpoint");
                        tx.execute(
                            "UPDATE endpoints SET text = ?, description = ?, verb = ?, base = ?, path = ?, group_id = ? WHERE id = ?",
                            &[
                                &endpoint.text,
                                &endpoint.description,
                                &endpoint.verb,
                                &endpoint.base,
                                &endpoint.path,
                                &group_id,
                                &endpoint_id,
                            ],
                        )?;
                    }
                }

                // Link endpoint to user
                tx.execute(
                    "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                    &[email, &endpoint_id],
                )?;

                // Process parameters for non-default endpoints
                let is_default: bool = tx.query_row(
                    "SELECT is_default FROM endpoints WHERE id = ?",
                    [&endpoint_id],
                    |row| row.get(0),
                )?;

                if !is_default {
                    // Clean up existing parameters first
                    tx.execute(
                        "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
                        [&endpoint_id],
                    )?;

                    tx.execute(
                        "DELETE FROM parameters WHERE endpoint_id = ?",
                        [&endpoint_id],
                    )?;

                    // Add new parameters
                    for param in &endpoint.parameters {
                        tx.execute(
                            "INSERT INTO parameters (endpoint_id, name, description, required) 
                             VALUES (?, ?, ?, ?)",
                            &[
                                &endpoint_id,
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
                                &[&endpoint_id, &param.name, alt],
                            )?;
                        }
                    }
                }

                imported_count += 1;
            }
        }

        tx.commit()?;

        tracing::info!(
            email = %email,
            group_count = api_groups.len(),
            endpoint_count = imported_count,
            "Successfully imported API groups and endpoints"
        );

        Ok(imported_count)
    }

    pub async fn add_user_api_group(
        &self,
        email: &str,
        api_group: &ApiGroupWithEndpoints,
    ) -> Result<usize, StoreError> {
        let mut conn = self.conn.lock().map_err(|_| StoreError::Lock)?;
        let tx = conn.transaction()?;

        let group = &api_group.group;
        let group_id = &group.id;

        tracing::info!(
            email = %email,
            group_id = %group_id,
            "Adding API group"
        );

        // Check if group exists
        let group_exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM api_groups WHERE id = ?)",
            [group_id],
            |row| row.get(0),
        )?;

        if !group_exists {
            // Insert new group
            tx.execute(
                "INSERT INTO api_groups (id, name, description, base, is_default) VALUES (?, ?, ?, ?, false)",
                &[
                    group_id,
                    &group.name,
                    &group.description,
                    &group.base,
                ],
            )?;
        } else {
            // Check if it's a default group
            let is_default: bool = tx.query_row(
                "SELECT is_default FROM api_groups WHERE id = ?",
                [group_id],
                |row| row.get(0),
            )?;

            if !is_default {
                // Update existing non-default group
                tx.execute(
                    "UPDATE api_groups SET name = ?, description = ?, base = ? WHERE id = ?",
                    &[&group.name, &group.description, &group.base, group_id],
                )?;
            }
        }

        // Associate group with user
        tx.execute(
            "INSERT OR IGNORE INTO user_groups (email, group_id) VALUES (?, ?)",
            &[email, group_id],
        )?;

        // Add endpoints
        let mut endpoint_count = 0;

        for endpoint in &api_group.endpoints {
            // Check if endpoint exists
            let endpoint_exists: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM endpoints WHERE id = ?)",
                [&endpoint.id],
                |row| row.get(0),
            )?;

            if !endpoint_exists {
                // Insert new endpoint
                tx.execute(
                    "INSERT INTO endpoints (id, text, description, verb, base, path, group_id, is_default) 
                     VALUES (?, ?, ?, ?, ?, ?, ?, false)",
                    &[
                        &endpoint.id,
                        &endpoint.text,
                        &endpoint.description,
                        &endpoint.verb,
                        &endpoint.base,
                        &endpoint.path,
                        group_id,
                    ],
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
                    tx.execute(
                        "UPDATE endpoints SET text = ?, description = ?, verb = ?, base = ?, path = ?, group_id = ? WHERE id = ?",
                        &[
                            &endpoint.text,
                            &endpoint.description,
                            &endpoint.verb,
                            &endpoint.base,
                            &endpoint.path,
                            group_id,
                            &endpoint.id,
                        ],
                    )?;
                }
            }

            // Associate endpoint with user
            tx.execute(
                "INSERT OR IGNORE INTO user_endpoints (email, endpoint_id) VALUES (?, ?)",
                &[email, &endpoint.id],
            )?;

            // Handle parameters for non-default endpoints
            let is_default: bool = tx.query_row(
                "SELECT is_default FROM endpoints WHERE id = ?",
                [&endpoint.id],
                |row| row.get(0),
            )?;

            if !is_default {
                // Clean up existing parameters
                tx.execute(
                    "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
                    [&endpoint.id],
                )?;

                tx.execute(
                    "DELETE FROM parameters WHERE endpoint_id = ?",
                    [&endpoint.id],
                )?;

                // Add parameters
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
            }

            endpoint_count += 1;
        }

        tx.commit()?;

        tracing::info!(
            email = %email,
            group_id = %group_id,
            endpoint_count = endpoint_count,
            "API group successfully added"
        );

        Ok(endpoint_count)
    }

    // Delete an API group and all its endpoints
    pub async fn delete_user_api_group(
        &self,
        email: &str,
        group_id: &str,
    ) -> Result<bool, StoreError> {
        let mut conn = self.conn.lock().map_err(|_| StoreError::Lock)?;

        tracing::info!(
            email = %email,
            group_id = %group_id,
            "Deleting API group"
        );

        // First, get all endpoint IDs for this group
        let mut stmt = conn.prepare(
            "SELECT e.id 
             FROM endpoints e
             JOIN user_endpoints ue ON e.id = ue.endpoint_id
             WHERE ue.email = ? AND e.group_id = ?",
        )?;

        let endpoint_ids: Vec<String> = stmt
            .query_map([email, group_id], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;

        // Start transaction
        let tx = conn.transaction()?;

        // Remove user-group association
        tx.execute(
            "DELETE FROM user_groups WHERE email = ? AND group_id = ?",
            [email, group_id],
        )?;

        // Remove user-endpoint associations for all endpoints in this group
        for endpoint_id in &endpoint_ids {
            tx.execute(
                "DELETE FROM user_endpoints WHERE email = ? AND endpoint_id = ?",
                [email, endpoint_id],
            )?;
        }

        // Check if the group is still associated with any user
        let group_still_used: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM user_groups WHERE group_id = ?)",
            [group_id],
            |row| row.get(0),
        )?;

        // If no user is using this group anymore, delete it and its endpoints
        if !group_still_used {
            // For each endpoint that's no longer used by any user, delete its data
            for endpoint_id in &endpoint_ids {
                let endpoint_still_used: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM user_endpoints WHERE endpoint_id = ?)",
                    [endpoint_id],
                    |row| row.get(0),
                )?;

                if !endpoint_still_used {
                    // Delete parameter alternatives
                    tx.execute(
                        "DELETE FROM parameter_alternatives WHERE endpoint_id = ?",
                        [endpoint_id],
                    )?;

                    // Delete parameters
                    tx.execute(
                        "DELETE FROM parameters WHERE endpoint_id = ?",
                        [endpoint_id],
                    )?;

                    // Delete endpoint
                    tx.execute(
                        "DELETE FROM endpoints WHERE id = ? AND is_default = false",
                        [endpoint_id],
                    )?;
                }
            }

            // Delete the group itself (if it's not a default group)
            tx.execute(
                "DELETE FROM api_groups WHERE id = ? AND is_default = false",
                [group_id],
            )?;
        }

        tx.commit()?;

        tracing::info!(
            email = %email,
            group_id = %group_id,
            endpoint_count = endpoint_ids.len(),
            "API group successfully deleted"
        );

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
