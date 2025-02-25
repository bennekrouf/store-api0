This file is a merged representation of the entire codebase, combined into a single document by Repomix.

# File Summary

## Purpose
This file contains a packed representation of the entire repository's contents.
It is designed to be easily consumable by AI systems for analysis, code review,
or other automated processes.

## File Format
The content is organized as follows:
1. This summary section
2. Repository information
3. Directory structure
4. Multiple file entries, each consisting of:
  a. A header with the file path (## File: path/to/file)
  b. The full contents of the file in a code block

## Usage Guidelines
- This file should be treated as read-only. Any changes should be made to the
  original repository files, not this packed version.
- When processing this file, use the file path to distinguish
  between different files in the repository.
- Be aware that this file may contain sensitive information. Handle it with
  the same level of security as you would the original repository.

## Notes
- Some files may have been excluded based on .gitignore rules and Repomix's configuration
- Binary files are not included in this packed representation. Please refer to the Repository Structure section for a complete list of file paths, including binary files
- Files matching patterns in .gitignore are excluded
- Files matching default ignore patterns are excluded

## Additional Info

# Directory Structure
```
samples/
  default.yaml
  divess.yaml
src/
  lib.rs
  main.rs
  server.rs
test/
  query.sh
  upload.sh
.gitignore
build.rs
Cargo.toml
config.yaml
endpoint_service.proto
endpoints.yaml
sample_endpoints.yaml
```

# Files

## File: samples/default.yaml
```yaml
endpoints:
  - id: "send_email"
    text: "send email"
    description: "Send an email with possible attachments"
    parameters:
      - name: "to"
        description: "Recipient's email address"
        required: true
        alternatives:
          - "recipient_email"
          - "email_to"
          - "to_email"
          - "destination_email"
      - name: "subject"
        description: "Email subject"
        required: true
        alternatives:
          - "email_title"
          - "mail_subject"
          - "title"
          - "email_subject"
      - name: "body"
        description: "Email content"
        required: true
        alternatives:
          - "email_body"
          - "content"
          - "message"
          - "mail_content"
          - "email_content"
      - name: "attachments"
        description: "Attachments"
        required: false
        alternatives:
          - "files"
          - "attached_files"
          - "email_attachments"

  - id: "create_ticket"
    text: "Create a new support ticket for tracking and resolving customer issues"
    description: "Create a new support ticket for tracking and resolving customer issues"
    parameters:
      - name: "title"
        description: "Ticket title"
        required: true
        alternatives:
          - "ticket_title"
          - "issue_title"
          - "ticket_name"
          - "issue_name"
      - name: "priority"
        description: "Ticket priority (urgent, normal, low)"
        required: true
        alternatives:
          - "ticket_priority"
          - "urgency"
          - "importance"
          - "severity"
      - name: "description"
        description: "Detailed problem description"
        required: true
        alternatives:
          - "ticket_description"
          - "issue_description"
          - "problem_details"
          - "details"
          - "issue_content"

  - id: "schedule_meeting"
    text: "schedule meeting"
    description: "Schedule a meeting or appointment"
    parameters:
      - name: "date"
        description: "Meeting date"
        required: true
        alternatives:
          - "meeting_date"
          - "appointment_date"
          - "scheduled_date"
          - "event_date"
      - name: "time"
        description: "Meeting time"
        required: true
        alternatives:
          - "meeting_time"
          - "appointment_time"
          - "scheduled_time"
          - "start_time"
          - "event_time"
      - name: "participants"
        description: "List of participants"
        required: true
        alternatives:
          - "attendees"
          - "meeting_participants"
          - "invitees"
          - "members"
          - "people"
      - name: "duration"
        description: "Duration in minutes"
        required: true
        alternatives:
          - "meeting_duration"
          - "length"
          - "time_duration"
          - "duration_minutes"
      - name: "topic"
        description: "Meeting topic"
        required: false
        alternatives:
          - "meeting_topic"
          - "subject"
          - "agenda"
          - "meeting_subject"

  - id: "analyze_logs"
    text: "analyze logs"
    description: "Analyze application logs"
    parameters:
      - name: "app_name"
        description: "Application name"
        required: true
        alternatives:
          - "application_name"
          - "app"
          - "application"
          - "service_name"
      - name: "start_date"
        description: "Analysis start date"
        required: true
        alternatives:
          - "from_date"
          - "begin_date"
          - "analysis_start"
          - "log_start_date"
      - name: "end_date"
        description: "Analysis end date"
        required: true
        alternatives:
          - "to_date"
          - "finish_date"
          - "analysis_end"
          - "log_end_date"
      - name: "log_level"
        description: "Log level (ERROR, WARN, INFO, DEBUG)"
        required: false
        alternatives:
          - "level"
          - "severity_level"
          - "logging_level"
          - "debug_level"

  - id: "deploy_app"
    text: "deploy application"
    description: "Deploy an application to production"
    parameters:
      - name: "app_name"
        description: "Application name to deploy"
        required: true
        alternatives:
          - "application_name"
          - "app"
          - "service_name"
          - "deployment_name"
      - name: "version"
        description: "Version to deploy"
        required: true
        alternatives:
          - "app_version"
          - "release_version"
          - "deployment_version"
          - "build_version"
      - name: "environment"
        description: "Target environment (prod, staging, dev)"
        required: true
        alternatives:
          - "env"
          - "target_env"
          - "deployment_env"
          - "target_environment"
      - name: "rollback_version"
        description: "Rollback version in case of error"
        required: false
        alternatives:
          - "backup_version"
          - "fallback_version"
          - "previous_version"
          - "revert_version"

  - id: "generate_report"
    text: "generate report"
    description: "Generate analysis or statistics report"
    parameters:
      - name: "report_type"
        description: "Report type (sales, traffic, performance)"
        required: true
        alternatives:
          - "type"
          - "kind"
          - "report_kind"
          - "analysis_type"
      - name: "period"
        description: "Report period (daily, weekly, monthly)"
        required: true
        alternatives:
          - "time_period"
          - "duration"
          - "report_period"
          - "time_range"
      - name: "format"
        description: "Output format (PDF, Excel, CSV)"
        required: true
        alternatives:
          - "output_format"
          - "file_format"
          - "report_format"
          - "export_format"

  - id: "backup_database"
    text: "backup database"
    description: "Create a database backup"
    parameters:
      - name: "database"
        description: "Database name"
        required: true
        alternatives:
          - "db_name"
          - "db"
          - "database_name"
          - "schema_name"
      - name: "backup_type"
        description: "Backup type (full, incremental)"
        required: true
        alternatives:
          - "type"
          - "backup_mode"
          - "db_backup_type"
          - "backup_method"
      - name: "compression"
        description: "Compression level (none, low, high)"
        required: false
        alternatives:
          - "compression_level"
          - "compress_level"
          - "compress_type"
          - "compression_type"

  - id: "process_payment"
    text: "process payment"
    description: "Process a customer payment"
    parameters:
      - name: "amount"
        description: "Payment amount"
        required: true
        alternatives:
          - "payment_amount"
          - "sum"
          - "total"
          - "price"
      - name: "currency"
        description: "Currency (EUR, USD)"
        required: true
        alternatives:
          - "currency_code"
          - "currency_type"
          - "payment_currency"
          - "money_type"
      - name: "payment_method"
        description: "Payment method (card, transfer, paypal)"
        required: true
        alternatives:
          - "method"
          - "pay_method"
          - "payment_type"
          - "pay_type"
      - name: "customer_id"
        description: "Customer identifier"
        required: true
        alternatives:
          - "client_id"
          - "user_id"
          - "payer_id"
          - "customer_number"
```

## File: samples/divess.yaml
```yaml
endpoints:
  - id: "get_themes_list"
    text: "Get themes list"
    description: "Return all categories and information"
    parameters: []

  - id: "upload_document"
    text: "Upload document"
    description: "Upload a document with event ID"
    parameters:
      - name: "file"
        description: "File to upload"
        required: true
        alternatives:
          - "document"
          - "upload_file"
      - name: "idEvenement"
        description: "Event ID"
        required: true
        alternatives:
          - "event_id"
          - "evenement_id"

  - id: "download_document"
    text: "Download document"
    description: "Download a document by ID and name"
    parameters:
      - name: "id"
        description: "Document ID"
        required: true
        alternatives:
          - "document_id"
          - "doc_id"
      - name: "nom"
        description: "Document name"
        required: true
        alternatives:
          - "name"
          - "filename"

  - id: "delete_document"
    text: "Delete document"
    description: "Delete a document by ID"
    parameters:
      - name: "id"
        description: "Document ID"
        required: true
        alternatives:
          - "document_id"
          - "doc_id"

  - id: "get_events_list"
    text: "Get events list"
    description: "Get list of events for a service"
    parameters:
      - name: "service"
        description: "Service name"
        required: true
        alternatives:
          - "service_id"
      - name: "ids"
        description: "List of event IDs"
        required: true
        alternatives:
          - "event_ids"
          - "evenement_ids"

  - id: "save_event"
    text: "Save event"
    description: "Save a new event"
    parameters:
      - name: "evenement"
        description: "Event object"
        required: true
        alternatives:
          - "event"
          - "event_data"

  - id: "modify_event_relations"
    text: "Modify event relations"
    description: "Modify relationships for an event"
    parameters:
      - name: "evenement"
        description: "Event object"
        required: true
        alternatives:
          - "event"
          - "event_data"

  - id: "get_event"
    text: "Get event"
    description: "Get single event by ID"
    parameters:
      - name: "id"
        description: "Event ID"
        required: true
        alternatives:
          - "event_id"
          - "evenement_id"

  - id: "delete_event"
    text: "Delete event"
    description: "Delete an event"
    parameters:
      - name: "id"
        description: "Event ID"
        required: true
        alternatives:
          - "event_id"
          - "evenement_id"

  - id: "get_journal_prestataire"
    text: "Get provider journal"
    description: "Get journal for a provider within a date range"
    parameters:
      - name: "idPrestataire"
        description: "Provider ID"
        required: true
        alternatives:
          - "provider_id"
      - name: "dateDebut"
        description: "Start date"
        required: true
        alternatives:
          - "start_date"
      - name: "dateFin"
        description: "End date"
        required: true
        alternatives:
          - "end_date"
      - name: "isActivite"
        description: "Activity flag"
        required: true
        alternatives:
          - "is_activity"

  - id: "get_liste_evenements"
    text: "Get events list report"
    description: "Get list of events within a date range"
    parameters:
      - name: "dateDebut"
        description: "Start date"
        required: true
        alternatives:
          - "start_date"
      - name: "dateFin"
        description: "End date"
        required: true
        alternatives:
          - "end_date"

  - id: "get_liste_contact"
    text: "Get contacts list"
    description: "Get list of all contacts"
    parameters: []

  - id: "get_actors_list"
    text: "Get actors list"
    description: "Get list of all actors"
    parameters: []

  - id: "get_actors_by_name"
    text: "Get actors by name"
    description: "Get list of actors by name and date"
    parameters:
      - name: "date"
        description: "Date"
        required: true
        alternatives:
          - "search_date"
      - name: "idActivite"
        description: "Activity ID"
        required: true
        alternatives:
          - "activity_id"

  - id: "get_active_actors"
    text: "Get active actors"
    description: "Get list of active actors"
    parameters:
      - name: "date"
        description: "Date"
        required: true
        alternatives:
          - "search_date"
      - name: "idInspection"
        description: "Inspection ID"
        required: true
        alternatives:
          - "inspection_id"

  - id: "save_actor"
    text: "Save actor"
    description: "Save a new actor"
    parameters:
      - name: "acteurDTO"
        description: "Actor data"
        required: true
        alternatives:
          - "actor"
          - "actor_data"

  - id: "update_actor"
    text: "Update actor"
    description: "Update an existing actor"
    parameters:
      - name: "acteurDTO"
        description: "Actor data"
        required: true
        alternatives:
          - "actor"
          - "actor_data"

  - id: "delete_actor"
    text: "Delete actor"
    description: "Delete an actor"
    parameters:
      - name: "id"
        description: "Actor ID"
        required: true
        alternatives:
          - "actor_id"

  - id: "get_dossier"
    text: "Get dossier"
    description: "Get dossier by ID"
    parameters:
      - name: "id"
        description: "Dossier ID"
        required: true
        alternatives:
          - "dossier_id"

  - id: "get_prestataire_dossiers"
    text: "Get provider dossiers"
    description: "Get dossiers for a provider"
    parameters:
      - name: "idPrestataire"
        description: "Provider ID"
        required: true
        alternatives:
          - "provider_id"
      - name: "typePrestataire"
        description: "Provider type"
        required: false
        alternatives:
          - "provider_type"
      - name: "idSecteur"
        description: "Sector ID"
        required: false
        alternatives:
          - "sector_id"
      - name: "spen"
        description: "SPEN flag"
        required: true
        alternatives:
          - "is_spen"

  - id: "get_inspection_evaluation"
    text: "Get inspection evaluation"
    description: "Get evaluation for an inspection"
    parameters:
      - name: "idInspection"
        description: "Inspection ID"
        required: true
        alternatives:
          - "inspection_id"

  - id: "get_modelerapport_roles"
    text: "Get model report roles"
    description: "Get roles for a model report"
    parameters:
      - name: "idInspection"
        description: "Inspection ID"
        required: true
        alternatives:
          - "inspection_id"

  - id: "get_periode_dossier"
    text: "Get period dossier"
    description: "Get period for last open dossier"
    parameters:
      - name: "idActivite"
        description: "Activity ID"
        required: true
        alternatives:
          - "activity_id"
      - name: "idSecteur"
        description: "Sector ID"
        required: true
        alternatives:
          - "sector_id"

  - id: "get_list_periods"
    text: "Get periods list"
    description: "Get list of all periods plus one"
    parameters:
      - name: "idActivite"
        description: "Activity ID"
        required: true
        alternatives:
          - "activity_id"
      - name: "idSecteur"
        description: "Sector ID"
        required: true
        alternatives:
          - "sector_id"

  - id: "get_period_types"
    text: "Get period types"
    description: "Get list of all period types"
    parameters: []

  - id: "save_period_type"
    text: "Save period type"
    description: "Save a new period type"
    parameters:
      - name: "periodeTypeDTO"
        description: "Period type data"
        required: true
        alternatives:
          - "period_type"
          - "period_data"

  - id: "get_status"
    text: "Get status"
    description: "Get system status"
    parameters: []

  - id: "get_trace"
    text: "Get trace data"
    description: "Get trace data within date range"
    parameters:
      - name: "dateDebut"
        description: "Start date"
        required: true
        alternatives:
          - "start_date"
      - name: "dateFin"
        description: "End date"
        required: true
        alternatives:
          - "end_date"

  - id: "generate_trace"
    text: "Generate trace report"
    description: "Generate trace report for date range"
    parameters:
      - name: "dateDebut"
        description: "Start date"
        required: true
        alternatives:
          - "start_date"
      - name: "dateFin"
        description: "End date"
        required: true
        alternatives:
          - "end_date"
```

## File: src/lib.rs
```rust
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
            tracing::debug!("Using default endpoints query");
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
                row.get::<_, Option<String>>(3)?, // param_name
                row.get::<_, Option<String>>(4)?, // param_description
                row.get::<_, Option<bool>>(5)?,   // required
                row.get::<_, Option<String>>(6)?, // alternatives as comma-separated string
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
                Ok((id, text, description, param_name, param_desc, required, alternatives_str)) => {
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
                    "INSERT INTO endpoints (id, text, description, is_default) VALUES (?, ?, ?, false)",
                    &[&endpoint.id, &endpoint.text, &endpoint.description],
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
                        "UPDATE endpoints SET text = ?, description = ? WHERE id = ?",
                        &[&endpoint.text, &endpoint.description, &endpoint.id],
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
```

## File: src/main.rs
```rust
mod server;
use crate::server::EndpointServiceImpl;
use api_store::{Endpoint, EndpointStore};
use endpoint::endpoint_service_server::EndpointServiceServer;
use serde::{Deserialize, Serialize};
use std::error::Error;
use tonic::transport::Server;
use tonic_reflection::server::Builder;
use tonic_web::GrpcWebLayer;
use tower_http::cors::{Any, CorsLayer};
use grpc_logger::setup_logging;
use grpc_logger::load_config;

use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Registry};
pub mod endpoint {
    tonic::include_proto!("endpoint");
}

#[derive(Debug, Serialize, Deserialize)]
struct EndpointsWrapper {
    endpoints: Vec<Endpoint>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    // Initialize logging configuration
    // let config = load_config("config.yaml")?;
    // setup_logging(&config).await?;

    // Test log generation
    // loop {
    //     tracing::info!("Test semantic log message");
    //     tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    // }

    Registry::default()
        .with(tracing_subscriber::fmt::layer())
        .with(EnvFilter::try_from_default_env().unwrap_or(EnvFilter::new("INFO")))
        .init();
    let mut store = EndpointStore::new("db/endpoints.db")?;

    // Load default endpoints from YAML and initialize DB
    let config_content = std::fs::read_to_string("endpoints.yaml")?;
    let wrapper: EndpointsWrapper = serde_yaml::from_str(&config_content)?;
    let default_endpoints = wrapper.endpoints;
    store.initialize_if_empty(&default_endpoints)?;
    let service = EndpointServiceImpl::new(store);
    let addr = "0.0.0.0:50055".parse()?;

    // Load the file descriptor for reflection
    let descriptor_set = include_bytes!(concat!(env!("OUT_DIR"), "/endpoint_descriptor.bin"));
    let reflection_service = Builder::configure()
        .register_encoded_file_descriptor_set(descriptor_set)
        .build_v1()?;

    // Create CORS layer
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_headers(Any)
        .allow_methods(Any)
        .expose_headers(Any);

    tracing::info!("Starting api-store gRPC server on {}", addr);

    Server::builder()
        .accept_http1(true)
        .max_concurrent_streams(128)
        .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
        .tcp_nodelay(true)
        .layer(cors)
        .layer(GrpcWebLayer::new())
        .add_service(EndpointServiceServer::new(service))
        .add_service(reflection_service)
        .serve_with_shutdown(addr, async {
            tokio::signal::ctrl_c().await.ok();
            tracing::info!("Shutting down api-store server...");
        })
        .await?;

    Ok(())
}
```

## File: src/server.rs
```rust
use crate::endpoint::endpoint_service_server::EndpointService;
use crate::endpoint::{
    Endpoint as ProtoEndpoint, GetEndpointsRequest, GetEndpointsResponse,
    Parameter as ProtoParameter,
};
use crate::Endpoint;
use api_store::EndpointStore;
use std::sync::Arc;
use tonic::{Request, Response, Status};
use tokio_stream::Stream;
use std::pin::Pin;

use crate::endpoint::UploadEndpointsRequest;
use crate::endpoint::UploadEndpointsResponse;
use crate::EndpointsWrapper;

#[derive(Debug, Clone)]
pub struct EndpointServiceImpl {
    store: Arc<EndpointStore>,
}

impl EndpointServiceImpl {
    pub fn new(store: EndpointStore) -> Self {
        Self {
            store: Arc::new(store),
        }
    }
}

#[tonic::async_trait]
impl EndpointService for EndpointServiceImpl {

    type GetDefaultEndpointsStream = Pin<Box<dyn Stream<Item = Result<GetEndpointsResponse, Status>> + Send + 'static>>;

    async fn get_default_endpoints(
        &self,
        request: Request<GetEndpointsRequest>,
    ) -> Result<Response<Self::GetDefaultEndpointsStream>, Status> {
        let email = request.into_inner().email;
        tracing::info!(email = %email, "Received get_default_endpoints request");

        // Clone necessary data for the stream
        let store = self.store.clone();

        // Create the stream
        let stream = async_stream::try_stream! {
            let endpoints = store.get_endpoints_by_email(&email).map_err(|e| {
                tracing::error!(error = %e, "Failed to get endpoints from store");
                Status::internal(e.to_string())
            })?;

            const BATCH_SIZE: usize = 10;
            let mut current_batch = Vec::with_capacity(BATCH_SIZE);

            tracing::info!("Starting endpoint transformation and streaming");

            for endpoint in endpoints {
                let param_count = endpoint.parameters.len();
                tracing::info!(
                    endpoint_id = %endpoint.id,
                    parameter_count = param_count,
                    "Transforming endpoint"
                );

                let proto_endpoint = ProtoEndpoint {
                    id: endpoint.id,
                    text: endpoint.text,
                    description: endpoint.description,
                    parameters: endpoint
                        .parameters
                        .into_iter()
                        .map(|p| ProtoParameter {
                            name: p.name,
                            description: p.description,
                            required: p.required,
                            alternatives: p.alternatives,
                        })
                        .collect(),
                };

                current_batch.push(proto_endpoint);

                // When batch is full, yield it
                if current_batch.len() >= BATCH_SIZE {
                    tracing::info!(
                        batch_size = current_batch.len(),
                        "Sending batch of endpoints"
                    );

                    yield GetEndpointsResponse {
                        endpoints: std::mem::take(&mut current_batch),
                    };
                }
            }

            // Send any remaining endpoints
            if !current_batch.is_empty() {
                tracing::info!(
                    batch_size = current_batch.len(),
                    "Sending final batch of endpoints"
                );

                yield GetEndpointsResponse {
                    endpoints: current_batch,
                };
            }

            tracing::info!("Finished streaming all endpoints");
        };

        Ok(Response::new(Box::pin(stream)))
    }

    async fn upload_endpoints(
        &self,
        request: Request<UploadEndpointsRequest>,
    ) -> Result<Response<UploadEndpointsResponse>, Status> {
        let req = request.into_inner();
        let email = req.email;
        let file_content = String::from_utf8(req.file_content.clone())
            .map_err(|e| Status::invalid_argument(format!("Invalid file content: {}", e)))?;

        tracing::info!(
            email = %email,
            filename = %req.file_name,
            "Processing endpoint upload request"
        );

        // Detect and parse content based on file extension
        let endpoints = if req.file_name.ends_with(".yaml") || req.file_name.ends_with(".yml") {
            // Parse YAML content
            match serde_yaml::from_str::<EndpointsWrapper>(&file_content) {
                Ok(wrapper) => wrapper.endpoints,
                Err(e) => {
                    // Try parsing as a list of endpoints directly
                    match serde_yaml::from_str::<Vec<Endpoint>>(&file_content) {
                        Ok(endpoints) => endpoints,
                        Err(_) => {
                            tracing::error!(
                                error = %e,
                                email = %email,
                                "Failed to parse YAML content"
                            );
                            return Err(Status::invalid_argument(
                                "Invalid YAML format. Expected either a list of endpoints or an object with 'endpoints' field."
                            ));
                        }
                    }
                }
            }
        } else if req.file_name.ends_with(".json") {
            // Parse JSON content
            match serde_json::from_str::<EndpointsWrapper>(&file_content) {
                Ok(wrapper) => wrapper.endpoints,
                Err(e) => {
                    // Try parsing as a list of endpoints directly
                    match serde_json::from_str::<Vec<Endpoint>>(&file_content) {
                        Ok(endpoints) => endpoints,
                        Err(_) => {
                            tracing::error!(
                                error = %e,
                                email = %email,
                                "Failed to parse JSON content"
                            );
                            return Err(Status::invalid_argument(
                                "Invalid JSON format. Expected either a list of endpoints or an object with 'endpoints' field."
                            ));
                        }
                    }
                }
            }
        } else {
            tracing::error!(
                email = %email,
                filename = %req.file_name,
                "Unsupported file format"
            );
            return Err(Status::invalid_argument(
                "Unsupported file format. Please upload a YAML (.yaml/.yml) or JSON (.json) file."
            ));
        };

        // Validate endpoints
        if endpoints.is_empty() {
            tracing::warn!(
                email = %email,
                "No endpoints found in uploaded file"
            );
            return Err(Status::invalid_argument("No endpoints found in uploaded file"));
        }

        // Validate endpoint structure
        for (index, endpoint) in endpoints.iter().enumerate() {
            if endpoint.id.trim().is_empty() {
                return Err(Status::invalid_argument(
                    format!("Endpoint at index {} has an empty ID", index)
                ));
            }
            if endpoint.text.trim().is_empty() {
                return Err(Status::invalid_argument(
                    format!("Endpoint '{}' has an empty text", endpoint.id)
                ));
            }
        }

        // Replace user endpoints
        match self.store.replace_user_endpoints(&email, endpoints).await {
            Ok(count) => {
                tracing::info!(
                    email = %email,
                    imported_count = count,
                    "Successfully imported endpoints"
                );
                Ok(Response::new(UploadEndpointsResponse {
                    success: true,
                    message: "Endpoints successfully imported".to_string(),
                    imported_count: count as i32,
                }))
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    "Failed to import endpoints"
                );
                Err(Status::internal(format!("Failed to import endpoints: {}", e)))
            }
        }
    }
}
```

## File: test/query.sh
```bash
#!/bin/bash

# Configuration
HOST="0.0.0.0:50055" # Match your server address

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Function to test getting endpoints for an email
test_get_endpoints() {
  local email="$1"
  local description="$2"

  echo -e "${BLUE}Testing: $description${NC}"
  echo "Email: $email"
  echo "-----------------"

  REQUEST_PAYLOAD=$(
    cat <<EOF
{
    "email": "$email"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD"
  echo "-----------------"

  response=$(grpcurl -plaintext \
    -d "$REQUEST_PAYLOAD" \
    $HOST \
    endpoint.EndpointService/GetDefaultEndpoints 2>&1)

  if [ $? -eq 0 ]; then
    echo -e "${GREEN}Success:${NC}"
    echo "$response"
  else
    echo -e "${RED}Error:${NC}"
    echo "$response"
  fi
  echo "-----------------"
  echo
}

# Test cases
echo "Testing endpoint service..."

# Test with different email addresses
# test_get_endpoints "user@example.com" "Get endpoints for standard user"
# test_get_endpoints "new.user@company.com" "Get endpoints for new user"
test_get_endpoints "mohamed.bennekrouf@gmail.com" "Get endpoints for admin"

# List available services (for verification)
echo "Checking available services:"
echo "-----------------"
grpcurl -plaintext $HOST list
echo

# Show service description
echo "Service description:"
echo "-----------------"
grpcurl -plaintext $HOST describe endpoint.EndpointService
echo

echo "All tests completed."
```

## File: test/upload.sh
```bash
#!/bin/bash

HOST="0.0.0.0:50055"
EMAIL="mohamed.bennekrouf@gmail.com"
FILE_PATH="../semantic/pickbazar_endpoints.yaml"

# Test uploading endpoints
echo "Testing endpoint upload..."
echo "-----------------"

# Read file content and encode as base64
FILE_CONTENT=$(base64 -w 0 "$FILE_PATH")

REQUEST_PAYLOAD=$(cat <<EOF
{
    "email": "$EMAIL",
    "file_content": "$FILE_CONTENT",
    "file_name": "$FILE_PATH"
}
EOF
)

echo "Request payload:"
echo "$REQUEST_PAYLOAD"
echo "-----------------"

response=$(grpcurl -plaintext \
    -d "$REQUEST_PAYLOAD" \
    $HOST \
    endpoint.EndpointService/UploadEndpoints)

echo "Response:"
echo "$response"
echo "-----------------"

# Test getting updated endpoints
# echo "Verifying uploaded endpoints..."
# echo "-----------------"

GET_REQUEST_PAYLOAD=$(cat <<EOF
{
    "email": "$EMAIL"
}
EOF
)

# response=$(grpcurl -plaintext \
#     -d "$GET_REQUEST_PAYLOAD" \
#     $HOST \
#     endpoint.EndpointService/GetDefaultEndpoints)
#
# echo "Updated endpoints:"
# echo "$response"
```

## File: .gitignore
```
/target
db
```

## File: build.rs
```rust
use std::env;
use std::path::PathBuf;

fn main() {
    // Get the OUT_DIR environment variable at runtime
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    // Construct the path to the descriptor set file
    let descriptor_path = out_dir.join("endpoint_descriptor.bin");

    // Configure and compile the proto files
    tonic_build::configure()
        .protoc_arg("--experimental_allow_proto3_optional")
        .file_descriptor_set_path(descriptor_path)
        .compile_protos(&["endpoint_service.proto"], &["proto"])
        .unwrap_or_else(|e| panic!("Failed to compile proto files: {}", e));
}
```

## File: Cargo.toml
```toml
[package]
name = "api-store"
version = "0.1.0"
edition = "2021"

[dependencies]
chrono = "0.4.39"
duckdb = { version = "1.1.1", features = ["bundled"] }
futures = "0.3.31"
prost = "0.13.4"
serde = { version = "1.0.217", features = ["derive"] }
serde_yaml = "0.9.34"
thiserror = "2.0.11"
tokio = { version = "1.43.0", features = ["full"] }
tokio-stream = { version = "0.1.17", features = ["sync"] }
tonic = "0.12.3"
tonic-reflection = "0.12.3"
tonic-web = "0.12.3"
tower-http = { version = "0.6.2", features = ["cors"] }
tracing = "0.1.41"
tracing-appender = "0.2.3"
tracing-subscriber = { version = "0.3.19", features = ["env-filter"] }
# grpc_logger = "0.10.0"
# grpc_logger = { path = "../grpc-logger" }
async-stream = "0.3.6"
uuid = "1.13.1"
grpc_logger = "0.10.0"
serde_json = "1.0.139"

[build-dependencies]
tonic-build = "0.12.3"
```

## File: config.yaml
```yaml
whoami: "api-store"
output: grpc # console, file
level: trace
grpc:
  address: "127.0.0.1"
  port: 50052
# file_path: "logs"
# file_name: "app.log"
log_fields:
  include_thread_id: false
  include_target: false
  include_file: false
  include_line: false
  include_timestamp: false
debug_mode:
  enabled: true
  test_interval_secs: 100
log_all_messages: false
```

## File: endpoint_service.proto
```protobuf
syntax = "proto3";
package endpoint;

service EndpointService {
    rpc GetDefaultEndpoints (GetEndpointsRequest) returns (stream GetEndpointsResponse);
    rpc UploadEndpoints (UploadEndpointsRequest) returns (UploadEndpointsResponse);
}

message GetEndpointsRequest {
    string email = 1;
}

message Parameter {
    string name = 1;
    string description = 2;
    bool required = 3;
    repeated string alternatives = 4;
}

message Endpoint {
    string id = 1;
    string text = 2;
    string description = 3;
    repeated Parameter parameters = 4;
}

message GetEndpointsResponse {
    repeated Endpoint endpoints = 1;
}

message UploadEndpointsRequest {
    string email = 1;
    bytes file_content = 2;
    string file_name = 3;
}

message UploadEndpointsResponse {
    bool success = 1;
    string message = 2;
    int32 imported_count = 3;
}
```

## File: endpoints.yaml
```yaml
endpoints:
  - id: "send_email"
    text: "send email"
    description: "Send an email with possible attachments"
    parameters:
      - name: "to"
        description: "Recipient's email address"
        required: true
        alternatives:
          - "recipient_email"
          - "email_to"
          - "to_email"
          - "destination_email"
      - name: "subject"
        description: "Email subject"
        required: true
        alternatives:
          - "email_title"
          - "mail_subject"
          - "title"
          - "email_subject"
      - name: "body"
        description: "Email content"
        required: true
        alternatives:
          - "email_body"
          - "content"
          - "message"
          - "mail_content"
          - "email_content"
      - name: "attachments"
        description: "Attachments"
        required: false
        alternatives:
          - "files"
          - "attached_files"
          - "email_attachments"

  - id: "create_ticket"
    text: "Create a new support ticket for tracking and resolving customer issues"
    description: "Create a new support ticket for tracking and resolving customer issues"
    parameters:
      - name: "title"
        description: "Ticket title"
        required: true
        alternatives:
          - "ticket_title"
          - "issue_title"
          - "ticket_name"
          - "issue_name"
      - name: "priority"
        description: "Ticket priority (urgent, normal, low)"
        required: true
        alternatives:
          - "ticket_priority"
          - "urgency"
          - "importance"
          - "severity"
      - name: "description"
        description: "Detailed problem description"
        required: true
        alternatives:
          - "ticket_description"
          - "issue_description"
          - "problem_details"
          - "details"
          - "issue_content"

  - id: "schedule_meeting"
    text: "schedule meeting"
    description: "Schedule a meeting or appointment"
    parameters:
      - name: "date"
        description: "Meeting date"
        required: true
        alternatives:
          - "meeting_date"
          - "appointment_date"
          - "scheduled_date"
          - "event_date"
      - name: "time"
        description: "Meeting time"
        required: true
        alternatives:
          - "meeting_time"
          - "appointment_time"
          - "scheduled_time"
          - "start_time"
          - "event_time"
      - name: "participants"
        description: "List of participants"
        required: true
        alternatives:
          - "attendees"
          - "meeting_participants"
          - "invitees"
          - "members"
          - "people"
      - name: "duration"
        description: "Duration in minutes"
        required: true
        alternatives:
          - "meeting_duration"
          - "length"
          - "time_duration"
          - "duration_minutes"
      - name: "topic"
        description: "Meeting topic"
        required: false
        alternatives:
          - "meeting_topic"
          - "subject"
          - "agenda"
          - "meeting_subject"

  - id: "analyze_logs"
    text: "analyze logs"
    description: "Analyze application logs"
    parameters:
      - name: "app_name"
        description: "Application name"
        required: true
        alternatives:
          - "application_name"
          - "app"
          - "application"
          - "service_name"
      - name: "start_date"
        description: "Analysis start date"
        required: true
        alternatives:
          - "from_date"
          - "begin_date"
          - "analysis_start"
          - "log_start_date"
      - name: "end_date"
        description: "Analysis end date"
        required: true
        alternatives:
          - "to_date"
          - "finish_date"
          - "analysis_end"
          - "log_end_date"
      - name: "log_level"
        description: "Log level (ERROR, WARN, INFO, DEBUG)"
        required: false
        alternatives:
          - "level"
          - "severity_level"
          - "logging_level"
          - "debug_level"

  - id: "deploy_app"
    text: "deploy application"
    description: "Deploy an application to production"
    parameters:
      - name: "app_name"
        description: "Application name to deploy"
        required: true
        alternatives:
          - "application_name"
          - "app"
          - "service_name"
          - "deployment_name"
      - name: "version"
        description: "Version to deploy"
        required: true
        alternatives:
          - "app_version"
          - "release_version"
          - "deployment_version"
          - "build_version"
      - name: "environment"
        description: "Target environment (prod, staging, dev)"
        required: true
        alternatives:
          - "env"
          - "target_env"
          - "deployment_env"
          - "target_environment"
      - name: "rollback_version"
        description: "Rollback version in case of error"
        required: false
        alternatives:
          - "backup_version"
          - "fallback_version"
          - "previous_version"
          - "revert_version"

  - id: "generate_report"
    text: "generate report"
    description: "Generate analysis or statistics report"
    parameters:
      - name: "report_type"
        description: "Report type (sales, traffic, performance)"
        required: true
        alternatives:
          - "type"
          - "kind"
          - "report_kind"
          - "analysis_type"
      - name: "period"
        description: "Report period (daily, weekly, monthly)"
        required: true
        alternatives:
          - "time_period"
          - "duration"
          - "report_period"
          - "time_range"
      - name: "format"
        description: "Output format (PDF, Excel, CSV)"
        required: true
        alternatives:
          - "output_format"
          - "file_format"
          - "report_format"
          - "export_format"

  - id: "backup_database"
    text: "backup database"
    description: "Create a database backup"
    parameters:
      - name: "database"
        description: "Database name"
        required: true
        alternatives:
          - "db_name"
          - "db"
          - "database_name"
          - "schema_name"
      - name: "backup_type"
        description: "Backup type (full, incremental)"
        required: true
        alternatives:
          - "type"
          - "backup_mode"
          - "db_backup_type"
          - "backup_method"
      - name: "compression"
        description: "Compression level (none, low, high)"
        required: false
        alternatives:
          - "compression_level"
          - "compress_level"
          - "compress_type"
          - "compression_type"

  - id: "process_payment"
    text: "process payment"
    description: "Process a customer payment"
    parameters:
      - name: "amount"
        description: "Payment amount"
        required: true
        alternatives:
          - "payment_amount"
          - "sum"
          - "total"
          - "price"
      - name: "currency"
        description: "Currency (EUR, USD)"
        required: true
        alternatives:
          - "currency_code"
          - "currency_type"
          - "payment_currency"
          - "money_type"
      - name: "payment_method"
        description: "Payment method (card, transfer, paypal)"
        required: true
        alternatives:
          - "method"
          - "pay_method"
          - "payment_type"
          - "pay_type"
      - name: "customer_id"
        description: "Customer identifier"
        required: true
        alternatives:
          - "client_id"
          - "user_id"
          - "payer_id"
          - "customer_number"
```

## File: sample_endpoints.yaml
```yaml
endpoints:
  - id: "custom_search"
    text: "search documents"
    description: "Search for documents with specified criteria"
    parameters:
      - name: "query"
        description: "Search query string"
        required: true
        alternatives:
          - "search_term"
          - "keyword"
          - "search_query"
      - name: "filter"
        description: "Filter criteria"
        required: false
        alternatives:
          - "criteria"
          - "constraints"
          - "conditions"

  - id: "custom_export"
    text: "export data"
    description: "Export data in various formats"
    parameters:
      - name: "format"
        description: "Export format (CSV, PDF, XLSX)"
        required: true
        alternatives:
          - "file_format"
          - "export_format"
          - "output_format"
      - name: "selection"
        description: "Data selection criteria"
        required: true
        alternatives:
          - "data_selection"
          - "selected_data"
          - "data_criteria"
```
