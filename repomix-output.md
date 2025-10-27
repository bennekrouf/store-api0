This file is a merged representation of a subset of the codebase, containing files not matching ignore patterns, combined into a single document by Repomix.

# File Summary

## Purpose
This file contains a packed representation of a subset of the repository's contents that is considered the most important context.
It is designed to be easily consumable by AI systems for analysis, code review,
or other automated processes.

## File Format
The content is organized as follows:
1. This summary section
2. Repository information
3. Directory structure
4. Repository files (if enabled)
5. Multiple file entries, each consisting of:
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
- Files matching these patterns are excluded: content, out, **/*.png, **/*.svg, public
- Files matching patterns in .gitignore are excluded
- Files matching default ignore patterns are excluded
- Files are sorted by Git change count (files with more changes are at the bottom)

# Directory Structure
```
.github/
  workflows/
    ci.yml
config-samples/
  config.yaml
samples/
  analyze-job-fit.yaml
  cvenom_api_endpoints_spec.yaml
  default.yaml
  divess.yaml
  gpecs.yaml
  output.yaml
  pickbazar_endpoints.yaml
  sample_endpoints.yaml
sql/
  schema.sql
src/
  endpoint_store/
    add_user_api_group.rs
    api_key_management.rs
    authorized_domains.rs
    cleanup.rs
    db_helpers.rs
    delete_user_api_group.rs
    delete_user_endpoint.rs
    errors.rs
    get_api_groups_by_email.rs
    get_create_user_api_groups.rs
    get_default_api_groups.rs
    manage_single_endpoint.rs
    mod.rs
    models.rs
    replace_user_api_groups.rs
    user_preferences.rs
    utils.rs
  add_api_group.rs
  config.rs
  db_pool.rs
  delete_api_group.rs
  delete_endpoint.rs
  formatter.rs
  generate_api_key.rs
  get_api_groups.rs
  get_api_key_usage.rs
  get_api_keys_status.rs
  get_api_usage_logs.rs
  get_authorized_domains.rs
  get_credit_balance_handler.rs
  get_user_preferences.rs
  grpc_server.rs
  health_check.rs
  http_server.rs
  log_api_usage.rs
  manage_endpoint.rs
  models.rs
  reset_user_preferences.rs
  revoke_all_api_keys_handler.rs
  revoke_api_key_handler.rs
  update_api_group.rs
  update_credit_balance_handler.rs
  update_user_preferences.rs
  upload_api_config.rs
  validate_api_key.rs
test/
  create_key.sh
  curl_api_usage.sh
  get_endpoints_test.sh
  query.sh
  test_api_key.sh
  test_multi_key.sh
  test_user_preferences.sh
  test-log-api-usage.sh
  upload_grpc.sh
  upload_http.sh
.gitignore
API.md
apidocumentation-markdown.md
apikey.md
apinext.md
build.rs
Cargo.toml
config.yaml
endpoint_service.proto
endpoints.md
endpoints.yaml
README.md
stripe.guide
```

# Files

## File: src/endpoint_store/delete_user_endpoint.rs
````rust
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};

/// Deletes a single endpoint for a user
pub async fn delete_user_endpoint(
    store: &EndpointStore,
    email: &str,
    endpoint_id: &str,
) -> Result<bool, StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    tracing::debug!(
        email = %email,
        endpoint_id = %endpoint_id,
        "Starting endpoint deletion process"
    );

    // Check if user has access to this endpoint
    let user_endpoint_row = tx
        .query_opt(
            "SELECT 1 FROM user_endpoints WHERE email = $1 AND endpoint_id = $2",
            &[&email, &endpoint_id],
        )
        .await
        .to_store_error()?;

    if user_endpoint_row.is_none() {
        tracing::debug!(
            email = %email,
            endpoint_id = %endpoint_id,
            "User does not have access to this endpoint"
        );
        return Ok(false);
    }

    // Remove user-endpoint association
    tx.execute(
        "DELETE FROM user_endpoints WHERE email = $1 AND endpoint_id = $2",
        &[&email, &endpoint_id],
    )
    .await
    .to_store_error()?;

    // Check if any other user still uses this endpoint
    let still_used_row = tx
        .query_opt(
            "SELECT 1 FROM user_endpoints WHERE endpoint_id = $1",
            &[&endpoint_id],
        )
        .await
        .to_store_error()?;

    // If no other user uses this endpoint, delete it completely
    if still_used_row.is_none() {
        tracing::debug!(
            endpoint_id = %endpoint_id,
            "No other users reference this endpoint, deleting completely"
        );

        // Delete parameter alternatives
        tx.execute(
            "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
            &[&endpoint_id],
        )
        .await
        .to_store_error()?;

        // Delete parameters
        tx.execute(
            "DELETE FROM parameters WHERE endpoint_id = $1",
            &[&endpoint_id],
        )
        .await
        .to_store_error()?;

        // Delete the endpoint itself
        tx.execute("DELETE FROM endpoints WHERE id = $1", &[&endpoint_id])
            .await
            .to_store_error()?;
    }

    tracing::info!(
        email = %email,
        endpoint_id = %endpoint_id,
        "Endpoint successfully deleted"
    );

    tx.commit().await.to_store_error()?;
    Ok(true)
}
````

## File: src/delete_endpoint.rs
````rust
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

/// Handler for deleting a single endpoint
pub async fn delete_endpoint(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>,
) -> impl Responder {
    let (email, endpoint_id) = path_params.into_inner();

    tracing::info!(
        email = %email,
        endpoint_id = %endpoint_id,
        "Received HTTP delete endpoint request"
    );

    match store.delete_user_endpoint(&email, &endpoint_id).await {
        Ok(deleted) => {
            if deleted {
                tracing::info!(
                    email = %email,
                    endpoint_id = %endpoint_id,
                    "Successfully deleted endpoint"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "Endpoint successfully deleted"
                }))
            } else {
                tracing::warn!(
                    email = %email,
                    endpoint_id = %endpoint_id,
                    "Endpoint not found or not deletable"
                );
                HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "message": "Endpoint not found or is a default endpoint that cannot be deleted"
                }))
            }
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                endpoint_id = %endpoint_id,
                "Failed to delete endpoint"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to delete endpoint: {}", e)
            }))
        }
    }
}
````

## File: .github/workflows/ci.yml
````yaml
name: CI

on:
  push:
    branches: [ master, develop ]
  pull_request:
    branches: [ master, develop ]

env:
  CARGO_TERM_COLOR: always

jobs:
  test:
    name: Test
    runs-on: ubuntu-latest
    
    steps:
    - name: Checkout code
      uses: actions/checkout@v4

    - name: Install Protocol Buffers compiler
      run: |
        sudo apt-get update
        sudo apt-get install -y protobuf-compiler

    - name: Install Rust toolchain
      uses: dtolnay/rust-toolchain@stable
      with:
        components: rustfmt, clippy

    - name: Cache Rust dependencies
      uses: actions/cache@v4
      with:
        path: |
          ~/.cargo/bin/
          ~/.cargo/registry/index/
          ~/.cargo/registry/cache/
          ~/.cargo/git/db/
          target/
        key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
        restore-keys: |
          ${{ runner.os }}-cargo-

    - name: Check formatting
      run: cargo fmt -- --check

    - name: Run Clippy
      run: cargo clippy --all-targets --all-features -- -D warnings

    - name: Build
      run: cargo build --verbose

    - name: Run tests
      run: cargo test --verbose

  build:
    name: Build Release
    runs-on: ubuntu-latest
    needs: test
    
    steps:
    - name: Checkout code
      uses: actions/checkout@v4

    - name: Install Protocol Buffers compiler
      run: |
        sudo apt-get update
        sudo apt-get install -y protobuf-compiler

    - name: Install Rust toolchain
      uses: dtolnay/rust-toolchain@stable

    - name: Cache Rust dependencies
      uses: actions/cache@v4
      with:
        path: |
          ~/.cargo/bin/
          ~/.cargo/registry/index/
          ~/.cargo/registry/cache/
          ~/.cargo/git/db/
          target/
        key: ${{ runner.os }}-cargo-${{ hashFiles('**/Cargo.lock') }}
        restore-keys: |
          ${{ runner.os }}-cargo-

    - name: Build release
      run: cargo build --release --verbose

    - name: Upload artifacts
      uses: actions/upload-artifact@v4
      with:
        name: store-binary
        path: target/release/store
````

## File: samples/analyze-job-fit.yaml
````yaml
# Job fit analysis
curl -X POST http://localhost:4002/api/analyze-job-fit \
  -H "Authorization: Bearer <firebase-id-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "job_url": "https://www.linkedin.com/jobs/view/1234567890",
    "person_name": "john-doe"
  }'
````

## File: samples/cvenom_api_endpoints_spec.yaml
````yaml
# CV Generator API Endpoints Specification

**Base URL:** `http://localhost:4002/api`

## Authentication
All protected endpoints require Firebase Authentication via Bearer token:
```
Authorization: Bearer <firebase-id-token>
```

## Public Endpoints

### GET /health
Health check endpoint.
- **Auth:** None required
- **Response:** `"OK"`

### GET /templates
Get available CV templates.
- **Auth:** None required
- **Response:**
```json
{
  "success": true,
  "templates": [
    {
      "name": "default",
      "description": "Standard CV layout"
    },
    {
      "name": "keyteo", 
      "description": "CV with Keyteo branding and logo at the top of every page"
    },
    {
      "name": "keyteo_full",
      "description": "CV with Keyteo branding featuring structured context and detailed responsibilities sections"
    }
  ]
}
```

### OPTIONS /<path>
CORS preflight handler for all routes.
- **Auth:** None required
- **Response:** HTTP 200

## Protected Endpoints

### POST /generate
Generate CV PDF.
- **Auth:** Required (Firebase + tenant validation)
- **Content-Type:** `application/json`
- **Request Body:**
```json
{
  "person": "john-doe",
  "lang": "en",        // Optional, defaults to "en" 
  "template": "keyteo" // Optional, defaults to "default"
}
```
- **Response:** PDF file (Content-Type: application/pdf)
- **Error Response:**
```json
{
  "success": false,
  "error": "Error message",
  "signup_required": true // Optional
}
```

### POST /create
Create person directory structure.
- **Auth:** Required (Firebase + tenant validation)
- **Content-Type:** `application/json`
- **Request Body:**
```json
{
  "person": "john-doe"
}
```
- **Response:**
```json
{
  "success": true,
  "message": "Person directory created successfully for john-doe",
  "person_dir": "/path/to/tenant/john-doe",
  "created_by": "user@example.com",
  "tenant": "tenant-name"
}
```

### POST /upload-picture
Upload profile picture for a person.
- **Auth:** Required (Firebase + tenant validation)
- **Content-Type:** `multipart/form-data`
- **Form Data:**
  - `person`: string (person name)
  - `file`: file (image file - PNG, JPG, etc.)
- **Response:**
```json
{
  "success": true,
  "message": "Profile picture uploaded successfully for john-doe",
  "file_path": "/path/to/tenant/john-doe/profile.png",
  "tenant": "tenant-name"
}
```

### GET /me
Get current authenticated user and tenant information.
- **Auth:** Required (Firebase + tenant validation)
- **Response:**
```json
{
  "success": true,
  "user": {
    "uid": "firebase-user-id",
    "email": "user@example.com",
    "name": "User Name",
    "picture": "https://profile-pic-url",
    "tenant_name": "tenant-name"
  },
  "message": "User authenticated successfully for tenant: tenant-name"
}
```
- **Error Response (unauthenticated):**
```json
{
  "success": false,
  "error": "Authentication required or user not authorized for any tenant",
  "signup_required": true
}
```

### GET /files/tree
Get tenant's file tree structure.
- **Auth:** Required (Firebase + tenant validation)
- **Response:**
```json
{
  "person-name": {
    "type": "folder",
    "children": {
      "cv_params.toml": {
        "type": "file",
        "size": 1024,
        "modified": "2023-01-01T00:00:00Z"
      },
      "experiences_en.typ": {
        "type": "file",
        "size": 2048,
        "modified": "2023-01-01T00:00:00Z"
      }
    }
  }
}
```

### GET /files/content?path=<file-path>
Get content of a specific file.
- **Auth:** Required (Firebase + tenant validation)
- **Query Parameters:**
  - `path`: File path relative to tenant directory (e.g., "john-doe/cv_params.toml")
- **Security:** Only allows `.typ` and `.toml` files
- **Response:** Raw file content as text
- **Error:** HTTP 403 for unauthorized files, HTTP 404 for missing files

### POST /files/save
Save content to a file.
- **Auth:** Required (Firebase + tenant validation)
- **Content-Type:** `application/json`
- **Request Body:**
```json
{
  "path": "john-doe/cv_params.toml",
  "content": "file content here"
}
```
- **Security:** Only allows `.typ` and `.toml` files
- **Response:**
```json
{
  "success": true,
  "message": "File saved successfully"
}
```



### Evaluate am i a good fit for this job
# Job fit analysis
curl -X POST http://localhost:4002/api/analyze-job-fit \
  -H "Authorization: Bearer <firebase-id-token>" \
  -H "Content-Type: application/json" \
  -d '{
    "job_url": "https://www.linkedin.com/jobs/view/1234567890",
    "person_name": "john-doe"
  }'



## Error Handling

### Authentication Errors
- **HTTP 401:** Missing or invalid Firebase token
- **HTTP 403:** User not authorized for tenant

### Validation Errors  
- **HTTP 400:** Invalid request body or parameters
- **HTTP 404:** Person/file not found

### Server Errors
- **HTTP 500:** Internal server error (database, file system, compilation)

## CORS Configuration
- **Origin:** `*` (all origins allowed)
- **Methods:** `POST, GET, PATCH, OPTIONS`
- **Headers:** `*` (all headers allowed)
- **Credentials:** `true`

## Multi-Tenant Architecture
- Users must be registered in SQLite tenant database
- Each tenant has isolated data directory: `data/tenants/{tenant-name}/`
- Person directories are created within tenant space
- File operations are scoped to tenant's data directory only

## Available Languages
- `en` (English) - default
- `fr` (French)

## Available Templates
- `default` - Standard CV layout
- `keyteo` - Keyteo branded template
- `keyteo_full` - Enhanced Keyteo template with structured experiences

## File Types Supported
- **Editable:** `.typ` (Typst templates), `.toml` (configuration)
- **Uploadable:** Image files (PNG, JPG, etc.) for profile pictures
- **Generated:** PDF files for CV output
````

## File: samples/default.yaml
````yaml
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
````

## File: samples/divess.yaml
````yaml
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
````

## File: samples/output.yaml
````yaml
api_groups:
  - name: "User Service"
    description: "User-related API endpoints"
    base: "https://api.example.com"
    endpoints:
      - text: "Get user"
        description: "Retrieves user information"
        verb: "GET"
        path: "/users/{id}"
        parameters:
          - name: "id"
            description: "User identifier"
            required: true
            alternatives:
              - "user_id"
              - "userId"

  - name: "Auth Service"
    description: "Authentication and session-related API endpoints"
    base: "https://auth.example.com"
    endpoints:
      - text: "Login"
        description: "Authenticates a user"
        verb: "POST"
        path: "/login"
        parameters:
          - name: "username"
            description: "User's login name"
            required: true
            alternatives: []
          - name: "password"
            description: "User's password"
            required: true
            alternatives: []

  - name: "Inspection Service"
    description: "Inspection-related API endpoints"
    base: "https://api.example.com/inspection"
    endpoints:
      - text: "Get inspection evaluation"
        description: "Retrieves evaluation report for an inspection"
        verb: "GET"
        path: "/{inspectionId}"
        parameters:
          - name: "inspectionId"
            description: "Unique identifier for the inspection"
            required: true
            alternatives: []
      
      - text: "Get model report roles"
        description: "Retrieves roles assigned to a model report"
        verb: "GET"
        path: "/model-reports/{reportId}"
        parameters:
          - name: "reportId"
            description: "Unique identifier for the model report"
            required: true
            alternatives: []

  - name: "Dossier Service"
    description: "Dossier-related API endpoints"
    base: "https://api.example.com/dossier"
    endpoints:
      - text: "Get dossier periods"
        description: "Retrieves active periods for a sector in a dossier"
        verb: "GET"
        path: "/{sectorId}"
        parameters:
          - name: "sectorId"
            description: "Unique identifier for the sector"
            required: true
            alternatives: []
      
      - text: "Get last open dossier period"
        description: "Retrieves details of the last open period in a sector's dossier"
        verb: "GET"
        path: "/dossier-periods/{sectorId}"
        parameters:
          - name: "sectorId"
            description: "Unique identifier for the sector"
            required: true
            alternatives: []

  - name: "Period Type Service"
    description: "Period type-related API endpoints"
    base: "https://api.example.com/period-types"
    endpoints:
      - text: "Get period types"
        description: "Retrieves available period types for evaluation"
        verb: "GET"
        path: "/"
        parameters: []

  - name: "System Status Service"
    description: "System status-related API endpoints"
    base: "https://api.example.com/status"
    endpoints:
      - text: "Get system status"
        description: "Retrieves current system operational status"
        verb: "GET"
        path: "/"
        parameters: []

  - name: "Trace Service"
    description: "Trace data-related API endpoints"
    base: "https://api.example.com/trace"
    endpoints:
      - text: "Generate trace data report"
        description: "Generates a trace data report for analysis"
        verb: "POST"
        path: "/{traceId}"
        parameters:
          - name: "traceId"
            description: "Unique identifier for the trace data report"
            required: true
            alternatives: []
      
      - text: "Get trace data"
        description: "Retrieves specific trace data by ID"
        verb: "GET"
        path: "/{traceId}"
        parameters:
          - name: "traceId"
            description: "Unique identifier for the trace data report"
            required: true
            alternatives: []
````

## File: samples/pickbazar_endpoints.yaml
````yaml
endpoints:
  # Authentication & Users
  - id: "register"
    text: "Register new user"
    description: "Create a new user account"
    parameters:
      - name: "name"
        description: "User's full name"
        required: true
      - name: "email"
        description: "User's email address"
        required: true
      - name: "password"
        description: "User's password"
        required: true
      - name: "permission"
        description: "User permission level"
        required: false
        default: "CUSTOMER"

  - id: "login"
    text: "User login"
    description: "Authenticate user and get token"
    parameters:
      - name: "email"
        description: "User's email address"
        required: true
      - name: "password"
        description: "User's password"
        required: true

  # Products
  - id: "create_product"
    text: "Create product"
    description: "Create a new product in the system"
    parameters:
      - name: "name"
        description: "Product name"
        required: true
      - name: "description"
        description: "Product description"
        required: false
      - name: "price"
        description: "Product price"
        required: true
      - name: "categories"
        description: "Product categories IDs"
        required: false
      - name: "variations"
        description: "Product variations"
        required: false
      - name: "shop_id"
        description: "Shop ID"
        required: true

  - id: "get_products"
    text: "Get products list"
    description: "Get paginated list of products"
    parameters:
      - name: "text"
        description: "Search text"
        required: false
      - name: "first"
        description: "Number of items per page"
        required: false
        default: 15
      - name: "page"
        description: "Page number"
        required: false
        default: 1
      - name: "shop_id"
        description: "Filter by shop ID"
        required: false

  # Orders
  - id: "create_order"
    text: "Create order"
    description: "Create a new order"
    parameters:
      - name: "shop_id"
        description: "Shop ID"
        required: true
      - name: "products"
        description: "List of products with quantities"
        required: true
      - name: "amount"
        description: "Total amount"
        required: true
      - name: "customer_contact"
        description: "Customer contact info"
        required: true
      - name: "billing_address"
        description: "Billing address"
        required: true
      - name: "shipping_address"
        description: "Shipping address"
        required: true

  - id: "get_orders"
    text: "Get orders list"
    description: "Get paginated list of orders"
    parameters:
      - name: "first"
        description: "Number of items per page"
        required: false
        default: 15
      - name: "page"
        description: "Page number"
        required: false
        default: 1
      - name: "customer_id"
        description: "Filter by customer ID"
        required: false
      - name: "shop_id"
        description: "Filter by shop ID"
        required: false

  # Shops
  - id: "create_shop"
    text: "Create shop"
    description: "Create a new shop"
    parameters:
      - name: "name"
        description: "Shop name"
        required: true
      - name: "description"
        description: "Shop description"
        required: false
      - name: "cover_image"
        description: "Shop cover image"
        required: false
      - name: "logo"
        description: "Shop logo"
        required: false
      - name: "address"
        description: "Shop address"
        required: false

  - id: "get_shops"
    text: "Get shops list"
    description: "Get paginated list of shops"
    parameters:
      - name: "text"
        description: "Search text"
        required: false
      - name: "first"
        description: "Number of items per page"
        required: false
        default: 15
      - name: "page"
        description: "Page number"
        required: false
        default: 1

  # Categories
  - id: "create_category"
    text: "Create category"
    description: "Create a new product category"
    parameters:
      - name: "name"
        description: "Category name"
        required: true
      - name: "details"
        description: "Category details"
        required: false
      - name: "parent"
        description: "Parent category ID"
        required: false
      - name: "type_id"
        description: "Category type ID"
        required: false

  # Attributes
  - id: "create_attribute"
    text: "Create attribute"
    description: "Create a new product attribute"
    parameters:
      - name: "name"
        description: "Attribute name"
        required: true
      - name: "shop_id"
        description: "Shop ID"
        required: true
      - name: "values"
        description: "Attribute values"
        required: true

  # Reviews
  - id: "create_review"
    text: "Create review"
    description: "Create a product review"
    parameters:
      - name: "product_id"
        description: "Product ID"
        required: true
      - name: "rating"
        description: "Rating value"
        required: true
      - name: "comment"
        description: "Review comment"
        required: true
      - name: "photos"
        description: "Review photos"
        required: false

  # Payments
  - id: "create_payment_intent"
    text: "Create payment intent"
    description: "Create a payment intent for order"
    parameters:
      - name: "tracking_number"
        description: "Order tracking number"
        required: true
      - name: "payment_gateway"
        description: "Payment gateway type"
        required: true
        alternatives:
          - "stripe"
          - "paypal"

  # Withdraws
  - id: "create_withdraw"
    text: "Create withdraw request"
    description: "Create a withdrawal request"
    parameters:
      - name: "amount"
        description: "Withdrawal amount"
        required: true
      - name: "shop_id"
        description: "Shop ID"
        required: true
      - name: "payment_method"
        description: "Payment method"
        required: true
      - name: "details"
        description: "Bank/payment details"
        required: true

  # Settings
  - id: "update_settings"
    text: "Update settings"
    description: "Update application settings"
    parameters:
      - name: "options"
        description: "Settings options"
        required: true
      - name: "language"
        description: "Settings language"
        required: false

  # File Upload
  - id: "upload"
    text: "Upload file"
    description: "Upload file attachment"
    parameters:
      - name: "attachment"
        description: "File to upload"
        required: true
````

## File: samples/sample_endpoints.yaml
````yaml
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
````

## File: src/endpoint_store/utils.rs
````rust
use slug::slugify;
use uuid::Uuid;

/// Generates a random UUID string
pub fn generate_uuid() -> String {
    Uuid::new_v4().to_string()
}

/// Generate ID from text using slugify and UUID for uniqueness
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
````

## File: src/generate_api_key.rs
````rust
use crate::endpoint_store::EndpointStore;
use crate::endpoint_store::GenerateKeyRequest;

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for generating a new API key
pub async fn generate_api_key(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<GenerateKeyRequest>,
) -> impl Responder {
    let email = &request.email;
    let key_name = &request.key_name;

    tracing::info!(
        email = %email,
        key_name = %key_name,
        "Received HTTP generate API key request"
    );

    match store.generate_api_key(email, key_name).await {
        Ok((key, key_prefix, _)) => {
            tracing::info!(
                email = %email,
                key_prefix = %key_prefix,
                "Successfully generated API key"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "API key generated successfully",
                "key": key,
                "keyPrefix": key_prefix,
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to generate API key"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to generate API key: {}", e),
            }))
        }
    }
}
````

## File: src/get_api_keys_status.rs
````rust
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;

// Handler for getting API key status
pub async fn get_api_keys_status(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP get API keys status request");

    match store.get_api_keys_status(&email).await {
        Ok(key_preference) => {
            tracing::info!(
                email = %email,
                has_keys = key_preference.has_keys,
                key_count = key_preference.active_key_count,
                "Successfully retrieved API keys status"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "keyPreference": key_preference,
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to retrieve API keys status"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error: {}", e),
            }))
        }
    }
}
````

## File: src/get_authorized_domains.rs
````rust
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;

#[derive(serde::Serialize)]
pub struct AuthorizedDomainsResponse {
    pub success: bool,
    pub domains: Vec<String>,
}

/// Handler for getting all authorized domains (used by gateway for CORS)
pub async fn get_authorized_domains(store: web::Data<Arc<EndpointStore>>) -> impl Responder {
    tracing::info!("Received HTTP get authorized domains request");

    match store.get_all_authorized_domains().await {
        Ok(domains) => {
            tracing::info!(
                domain_count = domains.len(),
                "Successfully retrieved authorized domains"
            );
            HttpResponse::Ok().json(AuthorizedDomainsResponse {
                success: true,
                domains,
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "Failed to retrieve authorized domains"
            );
            HttpResponse::InternalServerError().json(AuthorizedDomainsResponse {
                success: false,
                domains: vec![],
            })
        }
    }
}
````

## File: src/get_credit_balance_handler.rs
````rust
use crate::endpoint_store::EndpointStore;

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for getting credit balance
pub async fn get_credit_balance_handler(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP get credit balance request");

    match store.get_credit_balance(&email).await {
        Ok(balance) => {
            tracing::info!(
                email = %email,
                balance = balance,
                "Successfully retrieved credit balance"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "balance": balance,
                "message": "Credit balance retrieved successfully",
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to retrieve credit balance"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error retrieving credit balance: {}", e),
            }))
        }
    }
}
````

## File: src/get_user_preferences.rs
````rust
use crate::endpoint_store::EndpointStore;

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for getting user preferences
pub async fn get_user_preferences(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP get user preferences request");

    match store.get_user_preferences(&email).await {
        Ok(preferences) => {
            tracing::info!(
                email = %email,
                hidden_count = preferences.hidden_defaults.len(),
                "Successfully retrieved user preferences"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "preferences": preferences,
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to retrieve user preferences"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error: {}", e),
            }))
        }
    }
}
````

## File: src/health_check.rs
````rust
use actix_web::{HttpResponse, Responder};

// Health check endpoint
pub async fn health_check() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({
        "status": "healthy",
        "service": "api-store-http",
        "version": env!("CARGO_PKG_VERSION")
    }))
}
````

## File: src/reset_user_preferences.rs
````rust
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;

// Handler for resetting user preferences
pub async fn reset_user_preferences(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP reset user preferences request");

    match store.reset_user_preferences(&email).await {
        Ok(_) => {
            tracing::info!(
                email = %email,
                "Successfully reset user preferences"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "User preferences successfully reset",
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to reset user preferences"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to reset user preferences: {}", e),
            }))
        }
    }
}
````

## File: src/revoke_all_api_keys_handler.rs
````rust
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;

// Handler for revoking all API keys for a user
pub async fn revoke_all_api_keys_handler(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP revoke all API keys request");

    match store.revoke_all_api_keys(&email).await {
        Ok(count) => {
            tracing::info!(
                email = %email,
                count = count,
                "Successfully revoked all API keys"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": format!("Successfully revoked {} API keys", count),
                "count": count,
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to revoke all API keys"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to revoke all API keys: {}", e),
            }))
        }
    }
}
````

## File: src/revoke_api_key_handler.rs
````rust
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;
// use actix_web::{web, HttpResponse, Responder};

// Handler for revoking an API key
pub async fn revoke_api_key_handler(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>,
) -> impl Responder {
    let (email, key_id) = path_params.into_inner();
    tracing::info!(email = %email, "Received HTTP revoke API key request");

    match store.revoke_api_key(&email, &key_id).await {
        Ok(revoked) => {
            if revoked {
                tracing::info!(
                    email = %email,
                    "Successfully revoked API key"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "API key revoked successfully",
                }))
            } else {
                tracing::warn!(
                    email = %email,
                    "No API key found to revoke"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "No API key found to revoke",
                }))
            }
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to revoke API key"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to revoke API key: {}", e),
            }))
        }
    }
}
````

## File: src/update_credit_balance_handler.rs
````rust
use crate::endpoint_store::{EndpointStore, UpdateCreditRequest};
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

pub async fn update_credit_balance_handler(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<UpdateCreditRequest>,
) -> impl Responder {
    let email = &request.email;
    let amount = request.amount;

    tracing::info!(
        email = %email,
        amount = amount,
        "Received HTTP update credit balance request"
    );

    match store.update_credit_balance(email, amount).await {
        Ok(new_balance) => {
            tracing::info!(
                email = %email,
                amount = amount,
                new_balance = new_balance,
                "Successfully updated credit balance"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": format!("Credit balance updated by {}", amount),
                "balance": new_balance,
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                amount = amount,
                "Failed to update credit balance"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to update credit balance: {}", e),
            }))
        }
    }
}
````

## File: src/update_user_preferences.rs
````rust
use crate::endpoint_store::EndpointStore;
use crate::endpoint_store::UpdatePreferenceRequest;

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for updating user preferences
pub async fn update_user_preferences(
    store: web::Data<Arc<EndpointStore>>,
    update_data: web::Json<UpdatePreferenceRequest>,
) -> impl Responder {
    let email = &update_data.email;
    let action = &update_data.action;
    let endpoint_id = &update_data.endpoint_id;

    tracing::info!(
        email = %email,
        action = %action,
        endpoint_id = %endpoint_id,
        "Received HTTP update user preferences request"
    );

    match store
        .update_user_preferences(email, action, endpoint_id)
        .await
    {
        Ok(_) => {
            tracing::info!(
                email = %email,
                action = %action,
                endpoint_id = %endpoint_id,
                "Successfully updated user preferences"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "User preferences successfully updated",
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to update user preferences"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to update user preferences: {}", e),
            }))
        }
    }
}
````

## File: src/validate_api_key.rs
````rust
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::{
    endpoint_store::EndpointStore,
    models::{ValidateKeyRequest, ValidateKeyResponse},
};

// Handler for validating an API key
pub async fn validate_api_key(
    store: web::Data<Arc<EndpointStore>>,
    key_data: web::Json<ValidateKeyRequest>,
) -> impl Responder {
    let api_key = &key_data.api_key;

    tracing::info!("Received HTTP validate API key request");

    match store.validate_api_key(api_key).await {
        Ok(Some((key_id, email))) => {
            // Record usage for this key
            if let Err(e) = store.record_api_key_usage(&key_id).await {
                tracing::warn!(
                    error = %e,
                    key_id = %key_id,
                    "Failed to record API key usage but proceeding with validation"
                );
            }

            tracing::info!(
                email = %email,
                key_id = %key_id,
                "Successfully validated API key"
            );
            HttpResponse::Ok().json(ValidateKeyResponse {
                valid: true,
                email: Some(email),
                key_id: Some(key_id),
                message: "API key is valid".to_string(),
            })
        }
        Ok(None) => {
            tracing::warn!("Invalid API key provided");
            HttpResponse::Ok().json(ValidateKeyResponse {
                valid: false,
                email: None,
                key_id: None,
                message: "API key is invalid".to_string(),
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "Failed to validate API key"
            );
            HttpResponse::InternalServerError().json(ValidateKeyResponse {
                valid: false,
                email: None,
                key_id: None,
                message: format!("Error validating API key: {}", e),
            })
        }
    }
}
````

## File: test/create_key.sh
````bash
#!/bin/bash

# Test script for API key management

# Configuration
HOST="127.0.0.1:9090"                     # HTTP server address
TEST_EMAIL="mohamed.bennekrouf@gmail.com" # Test email
KEY_NAME="Test API Key"                   # Name for the API key

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing API Key Management${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Function to test generating a new API key
test_generate_key() {
  echo -e "${YELLOW}Testing: Generate API Key${NC}"
  echo "Email: $TEST_EMAIL, Key Name: $KEY_NAME"
  echo "-----------------"

  REQUEST_PAYLOAD=$(
    cat <<EOF
{
  "email": "$TEST_EMAIL",
  "key_name": "$KEY_NAME"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/key")

  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"

  # Extract and save the API key for later tests
  API_KEY=$(echo "$response" | jq -r '.key')
  if [ "$API_KEY" != "null" ]; then
    echo -e "${GREEN}API Key: $API_KEY${NC}"
    # Save to a temp file for later tests
    echo "$API_KEY" >/tmp/api_key.txt
  else
    echo -e "${RED}Failed to extract API key from response${NC}"
  fi

  echo
}

# Main execution
print_header

# Generate a new API key
echo -e "${BLUE}Generating a new API key:${NC}"
test_generate_key

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"

# Clean up
rm -f /tmp/api_key.txt
````

## File: test/curl_api_usage.sh
````bash
#!/bin/bash

# Configuration
HOST="127.0.0.1:9090"  # HTTP server address
KEY_ID="your-key-id-here"  # The API key ID to record usage for

# Record API key usage
echo "Recording API key usage for key ID: $KEY_ID"
curl -s -X POST -H "Content-Type: application/json" -d "{\"key_id\":\"$KEY_ID\"}" "$HOST/api/key/usage" | jq .
````

## File: test/get_endpoints_test.sh
````bash
#!/bin/bash

# Configuration
HOST="0.0.0.0:50055"                      # Match your server address
TEST_EMAIL="mohamed.bennekrouf@gmail.com" # The email we want to test

# Color codes for output
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
RED='\033[0;31m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing Endpoint Service - Get Endpoints by Email${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Function to check if grpcurl is installed
check_dependencies() {
  if ! command -v grpcurl &>/dev/null; then
    echo -e "${RED}Error: grpcurl is not installed${NC}"
    echo "Please install grpcurl to run this test:"
    echo "  - MacOS: brew install grpcurl"
    echo "  - Linux: Go to https://github.com/fullstorydev/grpcurl/releases"
    exit 1
  fi
}

# Function to test getting endpoints for an email with detailed output
test_get_endpoints_detailed() {
  local email="$1"

  echo -e "${CYAN}Testing Get Endpoints for:${NC} $email"
  echo -e "${YELLOW}Request Payload:${NC}"
  echo '{'
  echo "    \"email\": \"$email\""
  echo '}'
  echo

  # Create a temporary file for the response
  TEMP_FILE=$(mktemp)

  # Run the grpcurl command and capture output
  echo -e "${YELLOW}Sending request to:${NC} $HOST"
  grpcurl -plaintext \
    -d "{\"email\": \"$email\"}" \
    $HOST \
    endpoint.EndpointService/GetEndpoints >"$TEMP_FILE" 2>&1

  # Add a section to the test script to dump the raw response
  grpcurl -plaintext -d "{\"email\": \"$email\"}" $HOST endpoint.EndpointService/GetEndpoints >raw_response.txt
  echo "Raw response saved to raw_response.txt"

  local status=$?

  if [ $status -eq 0 ]; then
    echo -e "${GREEN}Request successful!${NC}"

    # Count the number of endpoints
    local endpoint_count=$(grep -c "\"id\":" "$TEMP_FILE")
    echo -e "${CYAN}Received${NC} $endpoint_count ${CYAN}endpoints${NC}"
    echo

    # Process and format output to be more readable
    echo -e "${CYAN}Endpoint Details:${NC}"

    # Extract and display each endpoint in a formatted way
    local current_id=""
    local current_text=""
    local current_desc=""
    local current_verb=""
    local current_base_url=""
    local current_path=""
    local param_count=0

    while IFS= read -r line; do
      # Extract fields
      if [[ $line =~ \"id\":\ \"([^\"]*)\" ]]; then
        if [ ! -z "$current_id" ]; then
          # Print previous endpoint details
          echo -e "${YELLOW}ID:${NC} $current_id"
          echo -e "${YELLOW}Text:${NC} $current_text"
          echo -e "${YELLOW}Description:${NC} $current_desc"
          echo -e "${YELLOW}HTTP Verb:${NC} $current_verb"
          echo -e "${YELLOW}Base URL:${NC} $current_base_url"
          echo -e "${YELLOW}Path:${NC} $current_path"
          echo -e "${YELLOW}Parameters:${NC} $param_count"
          echo -e "----------------------------------------------"
        fi
        current_id="${BASH_REMATCH[1]}"
        param_count=0
      elif [[ $line =~ \"text\":\ \"([^\"]*)\" ]]; then
        current_text="${BASH_REMATCH[1]}"
      elif [[ $line =~ \"description\":\ \"([^\"]*)\" ]]; then
        current_desc="${BASH_REMATCH[1]}"
      elif [[ $line =~ \"verb\":\ \"([^\"]*)\" ]]; then
        current_verb="${BASH_REMATCH[1]}"
      elif [[ $line =~ \"base\":\ \"([^\"]*)\" ]]; then
        current_base_url="${BASH_REMATCH[1]}"
      elif [[ $line =~ \"path\":\ \"([^\"]*)\" ]]; then
        current_path="${BASH_REMATCH[1]}"
      elif [[ $line =~ \"name\":\ \"([^\"]*)\" ]]; then
        param_count=$((param_count + 1))
      fi
    done <"$TEMP_FILE"

    # Print the last endpoint
    if [ ! -z "$current_id" ]; then
      echo -e "${YELLOW}ID:${NC} $current_id"
      echo -e "${YELLOW}Text:${NC} $current_text"
      echo -e "${YELLOW}Description:${NC} $current_desc"
      echo -e "${YELLOW}HTTP Verb:${NC} $current_verb"
      echo -e "${YELLOW}Base URL:${NC} $current_base_url"
      echo -e "${YELLOW}Path:${NC} $current_path"
      echo -e "${YELLOW}Parameters:${NC} $param_count"
    fi

    echo
    echo -e "${GREEN}Endpoints retrieved successfully!${NC}"
  else
    echo -e "${RED}Error retrieving endpoints:${NC}"
    cat "$TEMP_FILE"
  fi

  # Clean up
  rm "$TEMP_FILE"
}

# Function to dump the raw response for debugging
dump_raw_response() {
  local email="$1"

  echo -e "${CYAN}Getting raw gRPC response for:${NC} $email"

  grpcurl -plaintext \
    -d "{\"email\": \"$email\"}" \
    $HOST \
    endpoint.EndpointService/GetEndpoints

  echo -e "${GREEN}Raw response dump complete${NC}"
  echo
}

# Main execution
print_header
check_dependencies

# Run the main detailed test
test_get_endpoints_detailed "$TEST_EMAIL"

# Uncomment to get raw response for debugging
# echo -e "${BLUE}=======================================================${NC}"
# echo -e "${BLUE}  Raw Response Dump (for debugging)${NC}"
# echo -e "${BLUE}=======================================================${NC}"
# dump_raw_response "$TEST_EMAIL"

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"
````

## File: test/test_multi_key.sh
````bash
#!/bin/bash
# test/test_multi_key.sh

# Configuration
HOST="127.0.0.1:9090"  # HTTP server address
TEST_EMAIL="test@example.com"  # Test email

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing Multiple API Key Management${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Function to check if curl is installed
check_dependencies() {
  if ! command -v curl &>/dev/null; then
    echo -e "${RED}Error: curl is not installed${NC}"
    echo "Please install curl to run this test"
    exit 1
  fi

  if ! command -v jq &>/dev/null; then
    echo -e "${RED}Error: jq is not installed${NC}"
    echo "Please install jq to run this test"
    exit 1
  }
}

# Function to test getting API keys status
test_get_keys_status() {
  echo -e "${YELLOW}Testing: Get API Keys Status${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/keys/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test generating a new API key
test_generate_key() {
  local key_name=$1
  
  echo -e "${YELLOW}Testing: Generate API Key${NC}"
  echo "Email: $TEST_EMAIL, Key Name: $key_name"
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
{
  "email": "$TEST_EMAIL",
  "key_name": "$key_name"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/keys")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  
  # Extract and save the API key and key_id for later tests
  API_KEY=$(echo "$response" | jq -r '.key')
  KEY_ID=$(echo "$response" | jq -r '.keyId')
  
  if [ "$API_KEY" != "null" ] && [ "$KEY_ID" != "null" ]; then
    echo -e "${GREEN}API Key: $API_KEY${NC}"
    echo -e "${GREEN}Key ID: $KEY_ID${NC}"
    
    # Save to a temp file for later tests
    echo "$API_KEY" > /tmp/api_key_${key_name}.txt
    echo "$KEY_ID" > /tmp/key_id_${key_name}.txt
  else
    echo -e "${RED}Failed to extract API key and key ID from response${NC}"
  fi
  
  echo
}

# Function to test validating an API key
test_validate_key() {
  local key_name=$1
  API_KEY=$(cat /tmp/api_key_${key_name}.txt)
  
  echo -e "${YELLOW}Testing: Validate API Key ($key_name)${NC}"
  echo "API Key: ${API_KEY:0:10}..."
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
{
  "api_key": "$API_KEY"
}
EOF
  )
  
  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/key/validate")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test getting credit balance
test_get_credit_balance() {
  echo -e "${YELLOW}Testing: Get Credit Balance${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/credits/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test updating credit balance
test_update_credit_balance() {
  local amount=$1
  
  echo -e "${YELLOW}Testing: Update Credit Balance${NC}"
  echo "Email: $TEST_EMAIL, Amount: $amount"
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
{
  "email": "$TEST_EMAIL",
  "amount": $amount
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/credits")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test revoking a specific API key
test_revoke_key() {
  local key_name=$1
  KEY_ID=$(cat /tmp/key_id_${key_name}.txt)
  
  echo -e "${YELLOW}Testing: Revoke Specific API Key ($key_name)${NC}"
  echo "Email: $TEST_EMAIL, Key ID: $KEY_ID"
  echo "-----------------"

  response=$(curl -s -X DELETE "$HOST/api/user/keys/$TEST_EMAIL/$KEY_ID")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test revoking all API keys
test_revoke_all_keys() {
  echo -e "${YELLOW}Testing: Revoke All API Keys${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X DELETE "$HOST/api/user/keys/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Main execution
print_header
check_dependencies

# Initial state
echo -e "${BLUE}Initial API key status:${NC}"
test_get_keys_status

# Add initial credit
echo -e "${BLUE}Adding initial credit:${NC}"
test_update_credit_balance 100

# Generate multiple API keys
echo -e "${BLUE}Generating first API key:${NC}"
test_generate_key "Production Key"

echo -e "${BLUE}Generating second API key:${NC}"
test_generate_key "Development Key"

echo -e "${BLUE}Generating third API key:${NC}"
test_generate_key "Testing Key"

# Verify key status after generation
echo -e "${BLUE}API keys status after generation:${NC}"
test_get_keys_status

# Test validation with each key
echo -e "${BLUE}Testing validation with first key:${NC}"
test_validate_key "Production Key"

echo -e "${BLUE}Testing validation with second key:${NC}"
test_validate_key "Development Key"

echo -e "${BLUE}Testing validation with third key:${NC}"
test_validate_key "Testing Key"

# Check credit balance
echo -e "${BLUE}Checking credit balance:${NC}"
test_get_credit_balance

# Revoke one specific key
echo -e "${BLUE}Revoking the second key:${NC}"
test_revoke_key "Development Key"

# Check keys status after revoking one
echo -e "${BLUE}API keys status after revoking one key:${NC}"
test_get_keys_status

# Check that credit balance is preserved
echo -e "${BLUE}Verifying credit balance is preserved after revoking key:${NC}"
test_get_credit_balance

# Revoke all remaining keys
echo -e "${BLUE}Revoking all remaining keys:${NC}"
test_revoke_all_keys

# Final state
echo -e "${BLUE}Final API key status:${NC}"
test_get_keys_status

# Final credit balance
echo -e "${BLUE}Final credit balance (should be preserved):${NC}"
test_get_credit_balance

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"

# Clean up
rm -f /tmp/api_key_*.txt /tmp/key_id_*.txt
````

## File: test/test-log-api-usage.sh
````bash
#!/bin/bash
# test/test_api_usage_logging.sh

# Configuration
HOST="127.0.0.1:9090"  # HTTP server address
TEST_EMAIL="test@example.com"  # Test email
KEY_NAME="Test API Key for Logging"  # Name for the API key

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing API Usage Logging${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Function to check if dependencies are installed
check_dependencies() {
  if ! command -v curl &>/dev/null; then
    echo -e "${RED}Error: curl is not installed${NC}"
    echo "Please install curl to run this test"
    exit 1
  fi

  if ! command -v jq &>/dev/null; then
    echo -e "${RED}Error: jq is not installed${NC}"
    echo "Please install jq to run this test"
    exit 1
  }
}

# Function to test generating a new API key
test_generate_key() {
  echo -e "${YELLOW}Testing: Generate API Key${NC}"
  echo "Email: $TEST_EMAIL, Key Name: $KEY_NAME"
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
{
  "email": "$TEST_EMAIL",
  "key_name": "$KEY_NAME"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/keys")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  
  # Extract and save the API key and key_id for later tests
  API_KEY=$(echo "$response" | jq -r '.key')
  KEY_ID=$(echo "$response" | jq -r '.keyId')
  
  if [ "$API_KEY" != "null" ] && [ "$KEY_ID" != "null" ]; then
    echo -e "${GREEN}API Key: $API_KEY${NC}"
    echo -e "${GREEN}Key ID: $KEY_ID${NC}"
    
    # Save to a temp file for later tests
    echo "$API_KEY" > /tmp/api_key_logging.txt
    echo "$KEY_ID" > /tmp/key_id_logging.txt
    echo "$TEST_EMAIL" > /tmp/email_logging.txt
  else
    echo -e "${RED}Failed to extract API key and key ID from response${NC}"
    exit 1
  fi
  
  echo
}

# Function to test logging API usage
test_log_api_usage() {
  KEY_ID=$(cat /tmp/key_id_logging.txt)
  EMAIL=$(cat /tmp/email_logging.txt)
  
  echo -e "${YELLOW}Testing: Log API Usage${NC}"
  echo "Email: $EMAIL, Key ID: $KEY_ID"
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
{
  "key_id": "$KEY_ID",
  "email": "$EMAIL",
  "endpoint_path": "/api/test/endpoint",
  "method": "GET",
  "status_code": 200,
  "response_time_ms": 132,
  "request_size_bytes": 1024,
  "response_size_bytes": 8192,
  "ip_address": "127.0.0.1",
  "user_agent": "Test Script/1.0"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/usage/log")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  
  # Extract log ID if available
  LOG_ID=$(echo "$response" | jq -r '.log_id')
  
  if [ "$LOG_ID" != "null" ]; then
    echo -e "${GREEN}Successfully logged API usage with ID: $LOG_ID${NC}"
  else
    echo -e "${RED}Failed to log API usage${NC}"
  fi
  
  echo
}

# Function to test getting API usage logs
test_get_api_usage_logs() {
  KEY_ID=$(cat /tmp/key_id_logging.txt)
  EMAIL=$(cat /tmp/email_logging.txt)
  
  echo -e "${YELLOW}Testing: Get API Usage Logs${NC}"
  echo "Email: $EMAIL, Key ID: $KEY_ID"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/usage/logs/$EMAIL/$KEY_ID?limit=10")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  
  # Check if logs were successfully retrieved
  SUCCESS=$(echo "$response" | jq -r '.success')
  LOG_COUNT=$(echo "$response" | jq -r '.count')
  
  if [ "$SUCCESS" = "true" ]; then
    echo -e "${GREEN}Successfully retrieved $LOG_COUNT API usage logs${NC}"
  else
    echo -e "${RED}Failed to retrieve API usage logs${NC}"
  fi
  
  echo
}

# Main execution
print_header
check_dependencies

# Generate a new API key
echo -e "${BLUE}Generating a new API key:${NC}"
test_generate_key

# Log API usage
echo -e "${BLUE}Logging API usage:${NC}"
test_log_api_usage

# Log multiple API usages for better testing
for i in {1..5}; do
  KEY_ID=$(cat /tmp/key_id_logging.txt)
  EMAIL=$(cat /tmp/email_logging.txt)
  
  # Create a varied payload
  REQUEST_PAYLOAD=$(cat <<EOF
{
  "key_id": "$KEY_ID",
  "email": "$EMAIL",
  "endpoint_path": "/api/test/endpoint$i",
  "method": "$([ $i % 2 -eq 0 ] && echo 'GET' || echo 'POST')",
  "status_code": $([ $i % 3 -eq 0 ] && echo '404' || echo '200'),
  "response_time_ms": $((50 + $i * 25)),
  "request_size_bytes": $((512 * $i)),
  "response_size_bytes": $((1024 * $i)),
  "ip_address": "127.0.0.$i",
  "user_agent": "Test Script/$i.0"
}
EOF
  )

  # Skip output for cleaner test
  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/usage/log")
  echo -e "${GREEN}Logged additional API usage ${i}/5${NC}"
done

# Get API usage logs
echo -e "${BLUE}Getting API usage logs:${NC}"
test_get_api_usage_logs

# Test cleanup
echo -e "${BLUE}Cleaning up test data...${NC}"
KEY_ID=$(cat /tmp/key_id_logging.txt)
EMAIL=$(cat /tmp/email_logging.txt)
curl -s -X DELETE "$HOST/api/user/keys/$EMAIL/$KEY_ID" > /dev/null
echo -e "${GREEN}Test completed and resources cleaned up${NC}"

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"

# Clean up
rm -f /tmp/api_key_logging.txt /tmp/key_id_logging.txt /tmp/email_logging.txt
````

## File: test/upload_grpc.sh
````bash
#!/bin/bash
# test/upload_grpc.sh

# Single configurable variable for the input file
INPUT_FILE="${1:-samples/divess.yaml}" # Default to samples/divess.yaml if no argument provided

# Configuration
HOST="0.0.0.0:50055" # gRPC server address
EMAIL="mohamed.bennekrouf@gmail.com"

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing gRPC Upload Service${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Check for grpcurl
check_dependencies() {
  if ! command -v grpcurl &>/dev/null; then
    echo -e "${RED}Error: grpcurl is not installed${NC}"
    echo "Please install grpcurl to run this test"
    exit 1
  fi
}

# Test uploading endpoints
test_upload_endpoints() {
  echo -e "${YELLOW}Testing: Upload API Configuration via gRPC${NC}"
  echo "Email: $EMAIL, File: $INPUT_FILE"
  echo "-----------------"

  # Read file content and encode as base64
  FILE_CONTENT=$(base64 <"$INPUT_FILE" | tr -d '\n')

  REQUEST_PAYLOAD=$(
    cat <<EOF
{
    "email": "$EMAIL",
    "file_content": "$FILE_CONTENT",
    "file_name": "$INPUT_FILE"
}
EOF
  )

  echo "Request payload (truncated):"
  echo "{ \"email\": \"$EMAIL\", \"file_name\": \"$INPUT_FILE\", \"file_content\": \"[BASE64 CONTENT]\" }"
  echo "-----------------"

  response=$(grpcurl -plaintext \
    -d "$REQUEST_PAYLOAD" \
    $HOST \
    endpoint.EndpointService/UploadApiGroups)

  echo "Response:"
  echo "$response"
  echo "-----------------"

  # Check if the upload was successful
  if echo "$response" | grep -q '"success": true'; then
    echo -e "${GREEN}Upload successful!${NC}"
  else
    echo -e "${RED}Upload failed!${NC}"
  fi

  echo
}

# Main execution
print_header
check_dependencies

# Test uploading with the specified file
test_upload_endpoints

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"
````

## File: test/upload_http.sh
````bash
#!/bin/bash
# test/upload_http.sh

# Single configurable variable for the input file
INPUT_FILE="${1:-samples/divess.yaml}" # Default to samples/divess.yaml if no argument provided

# Configuration
HOST="127.0.0.1:9090" # HTTP server address
EMAIL="mohamed.bennekrouf@gmail.com"

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing HTTP Upload Service${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Check for curl and jq
check_dependencies() {
  if ! command -v curl &>/dev/null; then
    echo -e "${RED}Error: curl is not installed${NC}"
    echo "Please install curl to run this test"
    exit 1
  fi

  if ! command -v jq &>/dev/null; then
    echo -e "${RED}Error: jq is not installed${NC}"
    echo "Please install jq to run this test"
    exit 1
  fi
}

# Test uploading endpoints
test_upload_endpoints() {
  echo -e "${YELLOW}Testing: Upload API Configuration via HTTP${NC}"
  echo "Email: $EMAIL, File: $INPUT_FILE"
  echo "-----------------"

  # Read file content and encode as base64
  FILE_CONTENT=$(base64 <"$INPUT_FILE" | tr -d '\n')

  REQUEST_PAYLOAD=$(
    cat <<EOF
{
    "email": "$EMAIL",
    "file_content": "$FILE_CONTENT",
    "file_name": "$INPUT_FILE"
}
EOF
  )

  echo "Request payload (truncated):"
  echo "{ \"email\": \"$EMAIL\", \"file_name\": \"$INPUT_FILE\", \"file_content\": \"[BASE64 CONTENT]\" }"
  echo "-----------------"

  response=$(curl -s -X POST \
    -H "Content-Type: application/json" \
    -d "$REQUEST_PAYLOAD" \
    "$HOST/api/upload")

  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"

  # Check if the upload was successful
  if echo "$response" | jq -e '.success == true' >/dev/null; then
    echo -e "${GREEN}Upload successful!${NC}"
    echo -e "Imported ${GREEN}$(echo "$response" | jq '.imported_count')${NC} endpoints in ${GREEN}$(echo "$response" | jq '.group_count')${NC} groups."
  else
    echo -e "${RED}Upload failed!${NC}"
  fi

  echo
}

# Main execution
print_header
check_dependencies

# Test uploading with the specified file
test_upload_endpoints

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"
````

## File: apidocumentation-markdown.md
````markdown
# API Documentation

## API Groups Endpoints
- `GET /api/groups/{email}` - Get all API groups for a user with preferences applied
- `POST /api/upload` - Upload API groups configuration file (YAML/JSON)
- `POST /api/group` - Add a new API group
- `PUT /api/group` - Update an existing API group
- `DELETE /api/groups/{email}/{group_id}` - Delete an API group

## User Preferences Endpoints
- `GET /api/user/preferences/{email}` - Get user preferences
- `POST /api/user/preferences` - Update user preferences
- `DELETE /api/user/preferences/{email}` - Reset user preferences

## API Key Management Endpoints
- `GET /api/user/keys/{email}` - Get status of all API keys for a user
- `POST /api/user/keys` - Generate a new API key for a user
- `DELETE /api/user/keys/{email}/{key_id}` - Revoke a specific API key
- `DELETE /api/user/keys/{email}` - Revoke all API keys for a user
- `GET /api/key/usage/{email}/{key_id}` - Get usage statistics for a specific API key
- `POST /api/key/validate` - Validate an API key
- `POST /api/key/usage` - Record basic API key usage

## API Usage Logging Endpoints
- `POST /api/user/usage/log` - Log detailed API usage with additional information
- `GET /api/user/usage/logs/{email}/{key_id}` - Get detailed API usage logs for a specific key

## Credit Balance Endpoints
- `GET /api/user/credits/{email}` - Get credit balance for a user
- `POST /api/user/credits` - Update credit balance for a user

## System Endpoints
- `GET /api/health` - Health check endpoint
````

## File: apikey.md
````markdown
# API Key Management

## Overview

API Store now includes a comprehensive API key management system that allows users to generate, manage, and track usage of API keys. These API keys can be used to authenticate API requests without requiring user login sessions.

## Features

- **API Key Generation**: Users can generate secure API keys with custom names
- **API Key Authentication**: All API endpoints can be authenticated using API keys
- **Usage Tracking**: Track API key usage, including count and last used time
- **Key Revocation**: Users can revoke their API keys at any time
- **Security**: API keys are securely hashed using SHA-256 before storage
- **Prefix Display**: Only key prefixes are stored and displayed for reference

## API Endpoints

### Get API Key Status

```
GET /api/user/key/{email}
```

Returns the current status of a user's API key, including metadata like usage count and last used time.

**Example Response:**
```json
{
  "success": true,
  "keyPreference": {
    "hasKey": true,
    "generatedAt": "2023-05-15T14:30:00Z",
    "lastUsed": "2023-05-16T09:12:43Z",
    "usageCount": 127,
    "keyName": "Production Key",
    "keyPrefix": "sk_abc123",
    "balance": 0
  }
}
```

### Generate API Key

```
POST /api/user/key
```

**Request Body:**
```json
{
  "email": "user@example.com",
  "key_name": "Production Key"
}
```

Generates a new API key for the user. If a key already exists, it will be replaced.

**Example Response:**
```json
{
  "success": true,
  "message": "API key generated successfully",
  "key": "sk_live_abcdef123456789...", // Full key, only returned once
  "keyPrefix": "sk_abc123"
}
```

### Revoke API Key

```
DELETE /api/user/key/{email}
```

Revokes a user's existing API key.

**Example Response:**
```json
{
  "success": true,
  "message": "API key revoked successfully"
}
```

### Get API Key Usage

```
GET /api/user/usage/{email}
```

Returns usage analytics for a user's API key.

**Example Response:**
```json
{
  "success": true,
  "usage": {
    "hasKey": true,
    "generatedAt": "2023-05-15T14:30:00Z",
    "lastUsed": "2023-05-16T09:12:43Z",
    "usageCount": 127,
    "keyName": "Production Key",
    "keyPrefix": "sk_abc123",
    "balance": 0
  }
}
```

## Using API Keys

To use an API key for authentication, include it in API requests using the `X-API-Key` HTTP header:

```
X-API-Key: sk_live_yourApiKeyHere
```

All API endpoints that require authentication will accept this header.

## Security Considerations

- API keys are securely hashed before storage
- Only key prefixes are stored in plain text for reference
- Full keys are only returned once upon creation
- Keys follow the format `sk_live_<random string>` for easy identification
- Rate limiting is applied to all API requests

## Implementation Details

The API key management system is implemented with the following components:

1. **Database Schema**: The `user_preferences` table is extended with additional fields for API key storage
2. **API Key Generation**: Uses cryptographically secure random generators
3. **Authentication Middleware**: All API requests are authenticated using the API key middleware
4. **Usage Tracking**: API key usage is automatically tracked on each request
5. **Security**: API keys are hashed using SHA-256 before storage

## Future Enhancements

- Enhanced analytics for API key usage
- Support for multiple API keys per user
- Role-based permissions for API keys
- Usage quotas and rate limiting per key
- Credit-based usage system
````

## File: build.rs
````rust
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
````

## File: endpoints.md
````markdown
Here's a list of all the HTTP endpoints exposed by your backend for client communication:

## API Groups Endpoints
- `GET /api/groups/{email}` - Get all API groups for a user with preferences applied
- `POST /api/upload` - Upload API groups configuration file (YAML/JSON)
- `POST /api/group` - Add a new API group
- `PUT /api/group` - Update an existing API group
- `DELETE /api/groups/{email}/{group_id}` - Delete an API group

## User Preferences Endpoints
- `GET /api/user/preferences/{email}` - Get user preferences
- `POST /api/user/preferences` - Update user preferences
- `DELETE /api/user/preferences/{email}` - Reset user preferences

## API Key Management Endpoints
- `GET /api/user/keys/{email}` - Get status of all API keys for a user
- `POST /api/user/keys` - Generate a new API key for a user
- `DELETE /api/user/keys/{email}/{key_id}` - Revoke a specific API key
- `DELETE /api/user/keys/{email}` - Revoke all API keys for a user
- `GET /api/key/usage/{email}/{key_id}` - Get usage statistics for a specific API key
- `POST /api/key/validate` - Validate an API key

## Credit Balance Endpoints
- `GET /api/user/credits/{email}` - Get credit balance for a user
- `POST /api/user/credits` - Update credit balance for a user

## System Endpoints
- `GET /api/health` - Health check endpoint

Each of these endpoints accepts and returns JSON data and follows RESTful conventions. The multi-key system now allows users to manage multiple API keys independently while maintaining a single credit balance attached to their email.
````

## File: stripe.guide
````
# Backend Payment API Specification

This document outlines the required backend API endpoints for Stripe payment integration and credit management.

## Environment Variables Required

```bash
# Stripe Configuration
STRIPE_PUBLISHABLE_KEY=pk_live_... # or pk_test_... for development
STRIPE_SECRET_KEY=sk_live_...      # or sk_test_... for development
STRIPE_WEBHOOK_SECRET=whsec_...    # For webhook verification

# Frontend Configuration (to be added to env.local)
NEXT_PUBLIC_STRIPE_PUBLISHABLE_KEY=pk_live_... # or pk_test_...
```

## API Endpoints

### 1. Create Payment Intent

**Endpoint:** `POST /api/payments/intent`

**Headers:**
- `Content-Type: application/json`
- `X-Firebase-Auth: <firebase_id_token>`

**Request Body:**
```json
{
  "amount": 2500,           // Amount in cents ($25.00)
  "currency": "usd",        // Currency code
  "email": "user@example.com",
  "metadata": {             // Optional
    "source": "web_app",
    "timestamp": "2024-01-01T00:00:00Z"
  }
}
```

**Response (Success - 200):**
```json
{
  "success": true,
  "client_secret": "pi_1234567890_secret_abcdef",
  "payment_intent_id": "pi_1234567890",
  "message": "Payment intent created successfully"
}
```

**Response (Error - 400/500):**
```json
{
  "success": false,
  "message": "Invalid amount or currency",
  "error_code": "INVALID_AMOUNT"
}
```

**Implementation Notes:**
- Validate amount (minimum $1.00, maximum $10,000.00)
- Create Stripe PaymentIntent with automatic payment methods
- Store payment intent in database with user email
- Return client_secret for frontend confirmation

### 2. Confirm Payment and Add Credits

**Endpoint:** `POST /api/payments/confirm`

**Headers:**
- `Content-Type: application/json`
- `X-Firebase-Auth: <firebase_id_token>`

**Request Body:**
```json
{
  "payment_intent_id": "pi_1234567890",
  "email": "user@example.com",
  "amount": 25.00           // Amount in dollars for verification
}
```

**Response (Success - 200):**
```json
{
  "success": true,
  "message": "Payment confirmed and credits added",
  "new_balance": 75.50,     // Updated user balance
  "transaction_id": "txn_abc123",
  "amount_added": 25.00
}
```

**Response (Error):**
```json
{
  "success": false,
  "message": "Payment not found or already processed",
  "error_code": "PAYMENT_NOT_FOUND"
}
```

**Implementation Notes:**
- Verify payment intent exists and succeeded with Stripe
- Check payment hasn't been processed already (idempotent)
- Add credits to user's account balance in database
- Create transaction record for audit trail
- Send confirmation email (optional)

### 3. Get Payment History

**Endpoint:** `GET /api/payments/history/{email}?limit=50`

**Headers:**
- `X-Firebase-Auth: <firebase_id_token>`

**Query Parameters:**
- `limit` (optional): Number of records to return (default: 50, max: 100)
- `offset` (optional): Pagination offset (default: 0)

**Response (Success - 200):**
```json
{
  "success": true,
  "payments": [
    {
      "id": "pi_1234567890",
      "amount": 2500,         // Amount in cents
      "currency": "usd",
      "status": "succeeded",  // succeeded, pending, failed
      "created_at": "2024-01-01T12:00:00Z",
      "description": "API Credits Purchase"
    }
  ],
  "total_count": 15,
  "message": "Payment history retrieved successfully"
}
```

**Implementation Notes:**
- Return payments in descending chronological order
- Include pagination metadata
- Filter by user email from Firebase auth
- Convert amounts from cents to dollars in frontend

### 4. Cancel Payment Intent (Optional)

**Endpoint:** `POST /api/payments/cancel/{payment_intent_id}`

**Headers:**
- `X-Firebase-Auth: <firebase_id_token>`

**Response (Success - 200):**
```json
{
  "success": true,
  "message": "Payment intent cancelled successfully"
}
```

**Implementation Notes:**
- Cancel payment intent with Stripe if still possible
- Update local database record
- Only allow cancellation by the payment owner

## Database Schema

### payments table
```sql
CREATE TABLE payments (
    id VARCHAR(255) PRIMARY KEY,           -- Stripe payment_intent_id
    user_email VARCHAR(255) NOT NULL,     -- From Firebase auth
    amount_cents INTEGER NOT NULL,        -- Amount in cents
    currency VARCHAR(3) NOT NULL DEFAULT 'usd',
    status VARCHAR(50) NOT NULL,           -- pending, succeeded, failed, cancelled
    stripe_payment_intent_id VARCHAR(255) UNIQUE NOT NULL,
    created_at TIMESTAMP DEFAULT NOW(),
    updated_at TIMESTAMP DEFAULT NOW(),
    metadata JSONB,                        -- Additional data
    INDEX idx_user_email (user_email),
    INDEX idx_created_at (created_at),
    INDEX idx_status (status)
);
```

### user_credits table (or add to existing users table)
```sql
-- If separate table:
CREATE TABLE user_credits (
    user_email VARCHAR(255) PRIMARY KEY,
    balance_cents INTEGER DEFAULT 0,      -- Credits in cents for precision
    created_at TIMESTAMP DEFAULT NOW(),
    updated_at TIMESTAMP DEFAULT NOW()
);

-- Or add to existing users table:
ALTER TABLE users ADD COLUMN credit_balance_cents INTEGER DEFAULT 0;
```

### transactions table (for audit trail)
```sql
CREATE TABLE transactions (
    id SERIAL PRIMARY KEY,
    user_email VARCHAR(255) NOT NULL,
    type VARCHAR(50) NOT NULL,             -- credit_purchase, api_usage, refund
    amount_cents INTEGER NOT NULL,        -- Positive for credits, negative for usage
    balance_after_cents INTEGER NOT NULL, -- Balance after this transaction
    reference_id VARCHAR(255),            -- payment_intent_id or usage_id
    description TEXT,
    created_at TIMESTAMP DEFAULT NOW(),
    INDEX idx_user_email (user_email),
    INDEX idx_created_at (created_at),
    INDEX idx_type (type)
);
```

## Webhook Handling (Recommended)

### Stripe Webhook Endpoint
**Endpoint:** `POST /api/webhooks/stripe`

**Implementation Notes:**
- Verify webhook signature using Stripe webhook secret
- Handle `payment_intent.succeeded` events
- Update payment status in database
- Add credits to user account if not already processed
- Handle failed/cancelled payments

**Events to Handle:**
- `payment_intent.succeeded`
- `payment_intent.payment_failed`
- `payment_intent.canceled`

## Security Considerations

1. **Authentication**: Validate Firebase ID token on all endpoints
2. **Authorization**: Ensure users can only access their own data
3. **Webhook Security**: Verify Stripe webhook signatures
4. **Idempotency**: Prevent duplicate credit additions
5. **Amount Validation**: Validate amounts match between frontend and Stripe
6. **Rate Limiting**: Implement rate limits on payment endpoints
7. **Logging**: Log all payment operations for audit trail

## Error Handling

### Standard Error Codes:
- `INVALID_AMOUNT`: Amount out of allowed range
- `PAYMENT_NOT_FOUND`: Payment intent doesn't exist
- `ALREADY_PROCESSED`: Payment already confirmed
- `INSUFFICIENT_FUNDS`: For future usage deduction
- `STRIPE_ERROR`: Stripe API returned an error
- `AUTHENTICATION_FAILED`: Invalid Firebase token

## Testing

### Test Cards (Stripe Test Mode):
- Success: `4242424242424242`
- Decline: `4000000000000002`
- 3D Secure: `4000002500003155`

### Environment Setup:
1. Use Stripe test keys in development
2. Test webhook endpoints with Stripe CLI
3. Verify Firebase authentication integration
4. Test credit balance updates
5. Verify payment history retrieval

## Integration Steps

1. **Setup Stripe Account**: Create Stripe account and get API keys
2. **Configure Webhooks**: Set up webhook endpoint in Stripe dashboard
3. **Database Setup**: Create required tables
4. **Environment Variables**: Configure all required environment variables
5. **Testing**: Test with Stripe test cards
6. **Production**: Switch to live Stripe keys for production

## Frontend Integration

The frontend payment system expects these exact endpoint URLs and response formats. The components are already implemented and will work with this API specification.

Required environment variable in `.env.local`:
```
NEXT_PUBLIC_STRIPE_PUBLISHABLE_KEY=pk_test_...
```
````

## File: samples/gpecs.yaml
````yaml
endpoints:
  - id: "action_list"
    text: "list actions"
    description: "Get a list of actions for a specific application"
    verb: "GET"
    parameters:
      - name: "idApplication"
        description: "ID of the application"
        required: true

  - id: "application_get"
    text: "get application"
    description: "Retrieve application details by ID"
    verb: "GET"
    parameters:
      - name: "id"
        description: "Application ID"
        required: true

  - id: "bootstrap"
    text: "get bootstrap data"
    description: "Retrieve environment configuration and bootstrap data"
    verb: "GET"
    parameters: []

  - id: "etat_list"
    text: "list states"
    description: "Get a list of states for a specific application"
    verb: "GET"
    parameters:
      - name: "idApplication"
        description: "ID of the application"
        required: true

  - id: "historiqueetat_list"
    text: "list state history"
    description: "Get history of state changes for a procedure"
    verb: "GET"
    parameters:
      - name: "pIdProcedure"
        description: "ID of the procedure"
        required: true

  - id: "historiqueetat_get"
    text: "get state history"
    description: "Get specific state history by ID"
    verb: "GET"
    parameters:
      - name: "pId"
        description: "ID of the state history"
        required: true

  - id: "procedure_create"
    text: "create procedure"
    description: "Create a new procedure"
    verb: "POST"
    parameters:
      - name: "idApplication"
        description: "ID of the application"
        required: true
      - name: "dateObjetMetier"
        description: "Date of the business object"
        required: true

  - id: "procedure_get"
    text: "get procedure"
    description: "Retrieve procedure details by ID"
    verb: "GET"
    parameters:
      - name: "id"
        description: "Procedure ID"
        required: true

  - id: "procedure_actions_possibles"
    text: "get possible actions"
    description: "Get possible actions for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "procedure_informations"
    text: "get procedure information"
    description: "Get information for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true
      - name: "typeObjetMetier"
        description: "Type of business object"
        required: true

  - id: "procedure_effectuer_action"
    text: "perform action on procedure"
    description: "Perform an action on a procedure"
    verb: "POST"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true
      - name: "idAction"
        description: "ID of the action"
        required: true
      - name: "date"
        description: "Date of the action"
        required: true

  - id: "bannette_list"
    text: "list bannettes"
    description: "Get a list of bannettes"
    verb: "GET"
    parameters: []

  - id: "bannette_download"
    text: "download bannette document"
    description: "Download a document from a bannette"
    verb: "GET"
    parameters:
      - name: "id"
        description: "ID of the document"
        required: true
      - name: "nom"
        description: "Name of the document"
        required: true

  - id: "bannette_delete"
    text: "delete bannette"
    description: "Delete a bannette"
    verb: "DELETE"
    parameters:
      - name: "id"
        description: "ID of the bannette"
        required: true

  - id: "generer_bordereau"
    text: "generate bordereau"
    description: "Generate a bordereau document"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true
      - name: "typeObjetMetier"
        description: "Type of business object"
        required: true

  - id: "courrier_list"
    text: "list mails"
    description: "Get a list of mails"
    verb: "GET"
    parameters:
      - name: "typeObjetMetier"
        description: "Type of business object"
        required: true

  - id: "courrier_upload"
    text: "upload mail"
    description: "Upload a new mail"
    verb: "POST"
    parameters:
      - name: "typeObjetMetier"
        description: "Type of business object"
        required: true
      - name: "file"
        description: "File to upload"
        required: true

  - id: "courrier_download"
    text: "download mail"
    description: "Download a mail"
    verb: "GET"
    parameters:
      - name: "id"
        description: "ID of the mail"
        required: true
      - name: "nom"
        description: "Name of the mail"
        required: true

  - id: "courrier_delete"
    text: "delete mail"
    description: "Delete a mail"
    verb: "DELETE"
    parameters:
      - name: "id"
        description: "ID of the mail"
        required: true

  - id: "courrier_generate"
    text: "generate mail"
    description: "Generate a mail document"
    verb: "GET"
    parameters:
      - name: "idCourrier"
        description: "ID of the mail"
        required: true
      - name: "nomCourrier"
        description: "Name of the mail"
        required: true
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true
      - name: "typeObjetMetier"
        description: "Type of business object"
        required: true
      - name: "idProcedureSignalementAssocie"
        description: "ID of the associated reporting procedure"
        required: true

  - id: "courrier_tester"
    text: "test mail"
    description: "Test a mail document"
    verb: "GET"
    parameters:
      - name: "idCourrier"
        description: "ID of the mail"
        required: true
      - name: "nomCourrier"
        description: "Name of the mail"
        required: true
      - name: "typeObjetMetier"
        description: "Type of business object"
        required: true

  - id: "document_antivirus"
    text: "check document with antivirus"
    description: "Check a document with antivirus"
    verb: "POST"
    parameters:
      - name: "file"
        description: "File to check"
        required: true

  - id: "document_upload"
    text: "upload document"
    description: "Upload a new document"
    verb: "POST"
    parameters:
      - name: "file"
        description: "File to upload"
        required: true
      - name: "idEvenement"
        description: "ID of the event"
        required: true

  - id: "document_download"
    text: "download document"
    description: "Download a document"
    verb: "GET"
    parameters:
      - name: "id"
        description: "ID of the document"
        required: true
      - name: "nom"
        description: "Name of the document"
        required: true

  - id: "document_delete"
    text: "delete document"
    description: "Delete a document"
    verb: "DELETE"
    parameters:
      - name: "id"
        description: "ID of the document"
        required: true

  - id: "evenement_get"
    text: "get event"
    description: "Retrieve event details by ID"
    verb: "GET"
    parameters:
      - name: "id"
        description: "Event ID"
        required: true

  - id: "evenement_list"
    text: "list events"
    description: "Get a list of events"
    verb: "GET"
    parameters:
      - name: "typeObjetMetier"
        description: "Type of business object"
        required: true
      - name: "ids"
        description: "List of IDs"
        required: true

  - id: "evenement_create"
    text: "create event"
    description: "Create a new event"
    verb: "POST"
    parameters:
      - name: "evenementDto"
        description: "Event data"
        required: true

  - id: "evenement_update"
    text: "update event"
    description: "Update an event"
    verb: "PUT"
    parameters:
      - name: "id"
        description: "ID of the event"
        required: true
      - name: "dto"
        description: "Event data"
        required: true

  - id: "evenement_checkDroitPourModification"
    text: "check rights for modification"
    description: "Check rights for modifying an event"
    verb: "GET"
    parameters:
      - name: "id"
        description: "ID of the event"
        required: true

  - id: "evenement_delete"
    text: "delete event"
    description: "Delete an event"
    verb: "DELETE"
    parameters:
      - name: "id"
        description: "ID of the event"
        required: true

  - id: "recherche"
    text: "search"
    description: "Search for objects"
    verb: "GET"
    parameters:
      - name: "recherche"
        description: "Search query"
        required: true

  - id: "tableauStatistiques_genererTableauStatistiquesCS"
    text: "generate CS statistics table"
    description: "Generate CS statistics table"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "tableauStatistiques_genererTableauStatistiquesSignalement"
    text: "generate reporting statistics table"
    description: "Generate reporting statistics table"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "tableauStatistiques_genererTableauStatistiquesSignalementsEtDoleances"
    text: "generate reports and complaints statistics table"
    description: "Generate reports and complaints statistics table"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "trace_generate"
    text: "generate person trace"
    description: "Generate person trace"
    verb: "GET"
    parameters:
      - name: "idPersonne"
        description: "ID of the person"
        required: true

  - id: "persons_search"
    text: "search persons"
    description: "Search for persons"
    verb: "GET"
    parameters:
      - name: "type"
        description: "Type of search"
        required: true
      - name: "q"
        description: "Search query"
        required: true

  - id: "persons_save"
    text: "save person"
    description: "Save a person"
    verb: "POST"
    parameters:
      - name: "personneDto"
        description: "Person data"
        required: true

  - id: "persons_updateMembreCS"
    text: "update CS member"
    description: "Update a CS member"
    verb: "PUT"
    parameters:
      - name: "id"
        description: "ID of the person"
        required: true
      - name: "dateDebut"
        description: "Start date"
        required: true
      - name: "dateFin"
        description: "End date"
        required: false

  - id: "persons_get"
    text: "get person"
    description: "Retrieve person details by ID"
    verb: "GET"
    parameters:
      - name: "id"
        description: "Person ID"
        required: true

  - id: "persons_adressesPROGRES"
    text: "get PROGRES addresses"
    description: "Get PROGRES addresses for a person"
    verb: "GET"
    parameters:
      - name: "idPersonne"
        description: "ID of the person"
        required: true

  - id: "persons_specialisationsPROGRES"
    text: "get PROGRES specializations"
    description: "Get PROGRES specializations for a person"
    verb: "GET"
    parameters:
      - name: "idPersonne"
        description: "ID of the person"
        required: true

  - id: "persons_getMembreCS"
    text: "get CS members"
    description: "Get CS members"
    verb: "GET"
    parameters: []

  - id: "persons_getMembreCSActives"
    text: "get active CS members"
    description: "Get active CS members"
    verb: "GET"
    parameters: []

  - id: "persons_rechercherPersonnesByIds"
    text: "search persons by IDs"
    description: "Search for persons by IDs"
    verb: "GET"
    parameters:
      - name: "ids"
        description: "List of person IDs"
        required: true

  - id: "refinf_listePays"
    text: "get countries list"
    description: "Get list of countries"
    verb: "GET"
    parameters: []

  - id: "refinf_localitesByNPA"
    text: "get localities by postal code"
    description: "Get localities by postal code"
    verb: "GET"
    parameters:
      - name: "codePostal"
        description: "Postal code"
        required: true

  - id: "refinf_localitesByLibelle"
    text: "get localities by name"
    description: "Get localities by name"
    verb: "GET"
    parameters:
      - name: "libelle"
        description: "Locality name"
        required: true

  - id: "commission_get"
    text: "get commission"
    description: "Retrieve commission details by ID"
    verb: "GET"
    parameters:
      - name: "id"
        description: "Commission ID"
        required: true

  - id: "commission_planifiees"
    text: "get planned commissions"
    description: "Get planned commissions"
    verb: "GET"
    parameters: []

  - id: "commission_passees"
    text: "get past commissions"
    description: "Get past commissions"
    verb: "GET"
    parameters: []

  - id: "commission_create"
    text: "create commission"
    description: "Create a new commission"
    verb: "POST"
    parameters:
      - name: "commission"
        description: "Commission data"
        required: true

  - id: "commission_reorderCommissionProcedure"
    text: "reorder commission procedure"
    description: "Reorder commission procedure"
    verb: "GET"
    parameters:
      - name: "id"
        description: "ID of the commission"
        required: true
      - name: "position"
        description: "Position to remove"
        required: true

  - id: "commission_update"
    text: "update commission"
    description: "Update a commission"
    verb: "PUT"
    parameters:
      - name: "commission"
        description: "Commission data"
        required: true

  - id: "commission_numeroUnique"
    text: "check unique commission number"
    description: "Check if commission number is unique"
    verb: "GET"
    parameters:
      - name: "id"
        description: "ID of the commission"
        required: true
      - name: "dateCommission"
        description: "Date of the commission"
        required: true

  - id: "commission_getCommissionsForProcedure"
    text: "get commissions for procedure"
    description: "Get commissions for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "commission_getForProcedure"
    text: "get commission for procedure"
    description: "Get commission for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "commission_getMisEnCauseForCommission"
    text: "get accused for commission"
    description: "Get accused persons for a commission"
    verb: "GET"
    parameters:
      - name: "id"
        description: "ID of the commission"
        required: true

  - id: "commission_sauverOrdreProcedures"
    text: "save procedures order"
    description: "Save the order of procedures"
    verb: "POST"
    parameters:
      - name: "liste"
        description: "List of sessions"
        required: true

  - id: "commission_exportProcedureAssociees"
    text: "export associated procedures"
    description: "Export associated procedures"
    verb: "GET"
    parameters:
      - name: "idCommission"
        description: "ID of the commission"
        required: true

  - id: "doleance_recherche"
    text: "search complaints"
    description: "Search for complaints"
    verb: "GET"
    parameters:
      - name: "recherche"
        description: "Search query"
        required: true

  - id: "doleance_create"
    text: "create complaint"
    description: "Create a new complaint"
    verb: "POST"
    parameters:
      - name: "signalement"
        description: "Complaint data"
        required: true

  - id: "doleance_update"
    text: "update complaint"
    description: "Update a complaint"
    verb: "PUT"
    parameters:
      - name: "signalement"
        description: "Complaint data"
        required: true

  - id: "doleance_enCours"
    text: "get ongoing complaints"
    description: "Get ongoing complaints"
    verb: "GET"
    parameters: []

  - id: "doleance_forProcedure"
    text: "get complaint for procedure"
    description: "Get complaint for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "doleance_signalementsMemeSignalant"
    text: "get complaints from same reporter"
    description: "Get complaints from the same reporter"
    verb: "GET"
    parameters:
      - name: "idSignalant"
        description: "ID of the reporter"
        required: true
      - name: "idSignalement"
        description: "ID of the complaint"
        required: false

  - id: "doleance_effectuerAction"
    text: "perform action on complaint"
    description: "Perform an action on a complaint"
    verb: "POST"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true
      - name: "idAction"
        description: "ID of the action"
        required: true
      - name: "date"
        description: "Date of the action"
        required: true

  - id: "doleance_clotureN1"
    text: "close complaint N1"
    description: "Close a complaint at N1 level"
    verb: "PUT"
    parameters:
      - name: "signalement"
        description: "Complaint data"
        required: true
      - name: "date"
        description: "Date of closure"
        required: false

  - id: "doleance_clotureN2"
    text: "close complaint N2"
    description: "Close a complaint at N2 level"
    verb: "PUT"
    parameters:
      - name: "signalement"
        description: "Complaint data"
        required: true
      - name: "date"
        description: "Date of closure"
        required: false

  - id: "doleance_transmettreCommission"
    text: "forward complaint to commission"
    description: "Forward a complaint to a commission"
    verb: "PUT"
    parameters:
      - name: "signalement"
        description: "Complaint data"
        required: true
      - name: "date"
        description: "Date of forwarding"
        required: false

  - id: "doleance_choisirOrientation"
    text: "choose complaint orientation"
    description: "Choose orientation for a complaint"
    verb: "PUT"
    parameters:
      - name: "signalement"
        description: "Complaint data"
        required: true
      - name: "date"
        description: "Date of orientation"
        required: false

  - id: "doleance_orientationOmc"
    text: "orient complaint to OMC"
    description: "Orient a complaint to OMC"
    verb: "POST"
    parameters:
      - name: "signalement"
        description: "Complaint data"
        required: true
      - name: "date"
        description: "Date of orientation"
        required: false

  - id: "doleance_orientationGenerale"
    text: "general orientation of complaint"
    description: "General orientation of a complaint"
    verb: "POST"
    parameters:
      - name: "signalement"
        description: "Complaint data"
        required: true
      - name: "date"
        description: "Date of orientation"
        required: false

  - id: "doleance_reorienter"
    text: "reorient complaint"
    description: "Reorient a complaint"
    verb: "POST"
    parameters:
      - name: "signalement"
        description: "Complaint data"
        required: true
      - name: "date"
        description: "Date of reorientation"
        required: false

  - id: "doleance_openForUser"
    text: "get open complaints for user"
    description: "Get open complaints for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "doleance_closedForUser"
    text: "get closed complaints for user"
    description: "Get closed complaints for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "doleance_getForEnqueteCS"
    text: "get complaints for CS investigation"
    description: "Get complaints for a CS investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteCS"
        description: "ID of the CS investigation"
        required: true

  - id: "doleance_getForEnqueteCSResume"
    text: "get complaint summaries for CS investigation"
    description: "Get complaint summaries for a CS investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteCS"
        description: "ID of the CS investigation"
        required: true

  - id: "doleance_getForEnquetePenale"
    text: "get complaints for criminal investigation"
    description: "Get complaints for a criminal investigation"
    verb: "GET"
    parameters:
      - name: "idEnquetePenale"
        description: "ID of the criminal investigation"
        required: true

  - id: "doleance_getForEnqueteCop"
    text: "get complaints for COP investigation"
    description: "Get complaints for a COP investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteCop"
        description: "ID of the COP investigation"
        required: true

  - id: "doleance_getForEnqueteOMC"
    text: "get complaints for OMC investigation"
    description: "Get complaints for an OMC investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteOMC"
        description: "ID of the OMC investigation"
        required: true

  - id: "doleance_getForEnqueteAOMC"
    text: "get complaints for AOMC investigation"
    description: "Get complaints for an AOMC investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteAOMC"
        description: "ID of the AOMC investigation"
        required: true

  - id: "doleance_getForPleniere"
    text: "get complaints for plenary"
    description: "Get complaints for a plenary session"
    verb: "GET"
    parameters:
      - name: "idPleniere"
        description: "ID of the plenary session"
        required: true

  - id: "doleance_getForCommission"
    text: "get complaints for commission"
    description: "Get complaints for a commission"
    verb: "GET"
    parameters:
      - name: "idCommission"
        description: "ID of the commission"
        required: true

  - id: "doleance_genererFicheInstructionPlainte"
    text: "generate complaint instruction sheet"
    description: "Generate complaint instruction sheet"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "doleance_getAllSignalement"
    text: "get all complaints in range"
    description: "Get all complaints in a date range"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "droitsfins_getAll"
    text: "get all rights"
    description: "Get all fine-grained rights"
    verb: "GET"
    parameters: []

  - id: "echeance_nonTraitees"
    text: "get unprocessed deadlines"
    description: "Get unprocessed deadlines"
    verb: "GET"
    parameters: []

  - id: "echeance_create"
    text: "create deadline"
    description: "Create a new deadline"
    verb: "POST"
    parameters:
      - name: "echeance"
        description: "Deadline data"
        required: true

  - id: "echeance_update"
    text: "update deadline"
    description: "Update a deadline"
    verb: "PUT"
    parameters:
      - name: "echeance"
        description: "Deadline data"
        required: true

  - id: "echeance_delete"
    text: "delete deadline"
    description: "Delete a deadline"
    verb: "DELETE"
    parameters:
      - name: "id"
        description: "ID of the deadline"
        required: true

  - id: "echeance_traiter"
    text: "process deadline"
    description: "Process a deadline"
    verb: "POST"
    parameters:
      - name: "id"
        description: "ID of the deadline"
        required: true

  - id: "echeance_list"
    text: "list deadlines"
    description: "Get a list of deadlines"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "enqueteAOMC_create"
    text: "create AOMC investigation"
    description: "Create a new AOMC investigation"
    verb: "POST"
    parameters:
      - name: "enqueteAOMC"
        description: "AOMC investigation data"
        required: true

  - id: "enqueteAOMC_update"
    text: "update AOMC investigation"
    description: "Update an AOMC investigation"
    verb: "PUT"
    parameters:
      - name: "enqueteAOMC"
        description: "AOMC investigation data"
        required: true

  - id: "enqueteAOMC_forProcedure"
    text: "get AOMC investigation for procedure"
    description: "Get AOMC investigation for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "enqueteAOMC_openForUser"
    text: "get open AOMC investigations for user"
    description: "Get open AOMC investigations for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "enqueteAOMC_closedForUser"
    text: "get closed AOMC investigations for user"
    description: "Get closed AOMC investigations for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "enqueteAOMC_getForSignalement"
    text: "get AOMC investigations for report"
    description: "Get AOMC investigations for a report"
    verb: "GET"
    parameters:
      - name: "idSignalement"
        description: "ID of the report"
        required: true

  - id: "enqueteAOMC_getForDoleance"
    text: "get AOMC investigations for complaint"
    description: "Get AOMC investigations for a complaint"
    verb: "GET"
    parameters:
      - name: "idDoleance"
        description: "ID of the complaint"
        required: true

  - id: "enqueteAOMC_getForMisEnCause"
    text: "get AOMC investigations for accused"
    description: "Get AOMC investigations for an accused person"
    verb: "GET"
    parameters:
      - name: "idMisEnCause"
        description: "ID of the accused person"
        required: true

  - id: "enqueteAOMC_getForMesureCDSAS"
    text: "get AOMC investigations for CDSAS measure"
    description: "Get AOMC investigations for a CDSAS measure"
    verb: "GET"
    parameters:
      - name: "idMesureCDSAS"
        description: "ID of the CDSAS measure"
        required: true

  - id: "enqueteAOMC_recherche"
    text: "search AOMC investigations"
    description: "Search for AOMC investigations"
    verb: "GET"
    parameters:
      - name: "recherche"
        description: "Search query"
        required: true

  - id: "enqueteAOMC_decider"
    text: "decide on AOMC investigation"
    description: "Make a decision on an AOMC investigation"
    verb: "POST"
    parameters:
      - name: "deciderResponseDTO"
        description: "Decision data"
        required: true

  - id: "enqueteAOMC_getForCommission"
    text: "get AOMC investigations for commission"
    description: "Get AOMC investigations for a commission"
    verb: "GET"
    parameters:
      - name: "idCommission"
        description: "ID of the commission"
        required: true

  - id: "conseilsante_recherche"
    text: "search health council investigations"
    description: "Search for health council investigations"
    verb: "GET"
    parameters:
      - name: "recherche"
        description: "Search query"
        required: true

  - id: "conseilsante_enCours"
    text: "get ongoing health council investigations"
    description: "Get ongoing health council investigations"
    verb: "GET"
    parameters: []

  - id: "conseilsante_forProcedure"
    text: "get health council investigation for procedure"
    description: "Get health council investigation for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "conseilsante_openForUser"
    text: "get open health council investigations for user"
    description: "Get open health council investigations for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "conseilsante_closedForUser"
    text: "get closed health council investigations for user"
    description: "Get closed health council investigations for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "conseilsante_getForSignalement"
    text: "get health council investigations for report"
    description: "Get health council investigations for a report"
    verb: "GET"
    parameters:
      - name: "idSignalement"
        description: "ID of the report"
        required: true

  - id: "conseilsante_getForDoleance"
    text: "get health council investigations for complaint"
    description: "Get health council investigations for a complaint"
    verb: "GET"
    parameters:
      - name: "idDoleance"
        description: "ID of the complaint"
        required: true

  - id: "conseilsante_getForMesureCDSAS"
    text: "get health council investigations for CDSAS measure"
    description: "Get health council investigations for a CDSAS measure"
    verb: "GET"
    parameters:
      - name: "idMesureCDSAS"
        description: "ID of the CDSAS measure"
        required: true

  - id: "conseilsante_getForPleniere"
    text: "get health council investigations for plenary"
    description: "Get health council investigations for a plenary session"
    verb: "GET"
    parameters:
      - name: "idPleniere"
        description: "ID of the plenary session"
        required: true

  - id: "conseilsante_getForMisEnCause"
    text: "get health council investigations for accused"
    description: "Get health council investigations for an accused person"
    verb: "GET"
    parameters:
      - name: "idMisEnCause"
        description: "ID of the accused person"
        required: true

  - id: "conseilsante_create"
    text: "create health council investigation"
    description: "Create a new health council investigation"
    verb: "POST"
    parameters:
      - name: "enqueteConseilSante"
        description: "Health council investigation data"
        required: true

  - id: "conseilsante_effectuerAction"
    text: "perform action on health council investigation"
    description: "Perform an action on a health council investigation"
    verb: "POST"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true
      - name: "idAction"
        description: "ID of the action"
        required: true
      - name: "date"
        description: "Date of the action"
        required: true

  - id: "conseilsante_update"
    text: "update health council investigation"
    description: "Update a health council investigation"
    verb: "PUT"
    parameters:
      - name: "enqueteConseilSante"
        description: "Health council investigation data"
        required: true

  - id: "conseilsante_updatePersonne"
    text: "update person in health council investigation"
    description: "Update a person in a health council investigation"
    verb: "POST"
    parameters:
      - name: "idPersonne"
        description: "ID of the person"
        required: true
      - name: "libelle"
        description: "Label"
        required: true

  - id: "conseilsante_rendreDecision"
    text: "make decision on health council investigation"
    description: "Make a decision on a health council investigation"
    verb: "POST"
    parameters:
      - name: "rendreDecisionResponseDTO"
        description: "Decision data"
        required: true

  - id: "conseilsante_getAllEnqueteConseilSante"
    text: "get all health council investigations in range"
    description: "Get all health council investigations in a date range"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "enqueteCop_create"
    text: "create COP investigation"
    description: "Create a new COP investigation"
    verb: "POST"
    parameters:
      - name: "enqueteCOP"
        description: "COP investigation data"
        required: true

  - id: "enqueteCop_update"
    text: "update COP investigation"
    description: "Update a COP investigation"
    verb: "PUT"
    parameters:
      - name: "enqueteCOP"
        description: "COP investigation data"
        required: true

  - id: "enqueteCop_forProcedure"
    text: "get COP investigation for procedure"
    description: "Get COP investigation for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "enqueteCop_numeroUnique"
    text: "check unique COP investigation number"
    description: "Check if COP investigation number is unique"
    verb: "GET"
    parameters:
      - name: "id"
        description: "ID of the COP investigation"
        required: true
      - name: "numero"
        description: "Number of the COP investigation"
        required: true

  - id: "enqueteCop_openForUser"
    text: "get open COP investigations for user"
    description: "Get open COP investigations for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "enqueteCop_closedForUser"
    text: "get closed COP investigations for user"
    description: "Get closed COP investigations for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "enqueteCop_getForSignalement"
    text: "get COP investigations for report"
    description: "Get COP investigations for a report"
    verb: "GET"
    parameters:
      - name: "idSignalement"
        description: "ID of the report"
        required: true

  - id: "enqueteCop_getForDoleance"
    text: "get COP investigations for complaint"
    description: "Get COP investigations for a complaint"
    verb: "GET"
    parameters:
      - name: "idDoleance"
        description: "ID of the complaint"
        required: true

  - id: "enqueteCop_getForSignalementResume"
    text: "get COP investigation summaries for report"
    description: "Get COP investigation summaries for a report"
    verb: "GET"
    parameters:
      - name: "idSignalement"
        description: "ID of the report"
        required: true

  - id: "enqueteCop_recherche"
    text: "search COP investigations"
    description: "Search for COP investigations"
    verb: "GET"
    parameters:
      - name: "recherche"
        description: "Search query"
        required: true

  - id: "enqueteOMC_create"
    text: "create OMC investigation"
    description: "Create a new OMC investigation"
    verb: "POST"
    parameters:
      - name: "enqueteOMC"
        description: "OMC investigation data"
        required: true

  - id: "enqueteOMC_update"
    text: "update OMC investigation"
    description: "Update an OMC investigation"
    verb: "PUT"
    parameters:
      - name: "enqueteOMC"
        description: "OMC investigation data"
        required: true

  - id: "enqueteOMC_forProcedure"
    text: "get OMC investigation for procedure"
    description: "Get OMC investigation for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "enqueteOMC_openForUser"
    text: "get open OMC investigations for user"
    description: "Get open OMC investigations for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "enqueteOMC_closedForUser"
    text: "get closed OMC investigations for user"
    description: "Get closed OMC investigations for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "enqueteOMC_getForSignalement"
    text: "get OMC investigations for report"
    description: "Get OMC investigations for a report"
    verb: "GET"
    parameters:
      - name: "idSignalement"
        description: "ID of the report"
        required: true

  - id: "enqueteOMC_getForDoleance"
    text: "get OMC investigations for complaint"
    description: "Get OMC investigations for a complaint"
    verb: "GET"
    parameters:
      - name: "idDoleance"
        description: "ID of the complaint"
        required: true

  - id: "enqueteOMC_getForMisEnCause"
    text: "get OMC investigations for accused"
    description: "Get OMC investigations for an accused person"
    verb: "GET"
    parameters:
      - name: "idMisEnCause"
        description: "ID of the accused person"
        required: true

  - id: "enqueteOMC_getForMesureMC"
    text: "get OMC investigations for MC measure"
    description: "Get OMC investigations for an MC measure"
    verb: "GET"
    parameters:
      - name: "idMesureMC"
        description: "ID of the MC measure"
        required: true

  - id: "enqueteOMC_effectuerAction"
    text: "perform action on OMC investigation"
    description: "Perform an action on an OMC investigation"
    verb: "POST"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true
      - name: "idAction"
        description: "ID of the action"
        required: true
      - name: "date"
        description: "Date of the action"
        required: true

  - id: "enqueteOMC_conclure"
    text: "conclude OMC investigation"
    description: "Conclude an OMC investigation"
    verb: "POST"
    parameters:
      - name: "conclureResponseDTO"
        description: "Conclusion data"
        required: true

  - id: "enqueteOMC_recherche"
    text: "search OMC investigations"
    description: "Search for OMC investigations"
    verb: "GET"
    parameters:
      - name: "recherche"
        description: "Search query"
        required: true

  - id: "enqueteOMC_getForCommission"
    text: "get OMC investigations for commission"
    description: "Get OMC investigations for a commission"
    verb: "GET"
    parameters:
      - name: "idCommission"
        description: "ID of the commission"
        required: true

  - id: "enquetePenale_recherche"
    text: "search criminal investigations"
    description: "Search for criminal investigations"
    verb: "GET"
    parameters:
      - name: "recherche"
        description: "Search query"
        required: true

  - id: "enquetePenale_create"
    text: "create criminal investigation"
    description: "Create a new criminal investigation"
    verb: "POST"
    parameters:
      - name: "enquetePenale"
        description: "Criminal investigation data"
        required: true

  - id: "enquetePenale_update"
    text: "update criminal investigation"
    description: "Update a criminal investigation"
    verb: "PUT"
    parameters:
      - name: "enquetePenale"
        description: "Criminal investigation data"
        required: true

  - id: "enquetePenale_forProcedure"
    text: "get criminal investigation for procedure"
    description: "Get criminal investigation for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "enquetePenale_numeroUnique"
    text: "check unique criminal investigation number"
    description: "Check if criminal investigation number is unique"
    verb: "GET"
    parameters:
      - name: "id"
        description: "ID of the criminal investigation"
        required: true
      - name: "numero"
        description: "Number of the criminal investigation"
        required: true

  - id: "enquetePenale_openForUser"
    text: "get open criminal investigations for user"
    description: "Get open criminal investigations for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "enquetePenale_closedForUser"
    text: "get closed criminal investigations for user"
    description: "Get closed criminal investigations for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "enquetePenale_getForSignalement"
    text: "get criminal investigations for report"
    description: "Get criminal investigations for a report"
    verb: "GET"
    parameters:
      - name: "idSignalement"
        description: "ID of the report"
        required: true

  - id: "enquetePenale_getForDoleance"
    text: "get criminal investigations for complaint"
    description: "Get criminal investigations for a complaint"
    verb: "GET"
    parameters:
      - name: "idDoleance"
        description: "ID of the complaint"
        required: true

  - id: "enquetePenale_getForSignalementResume"
    text: "get criminal investigation summaries for report"
    description: "Get criminal investigation summaries for a report"
    verb: "GET"
    parameters:
      - name: "idSignalement"
        description: "ID of the report"
        required: true

  - id: "listeElement_getAll"
    text: "get all list elements"
    description: "Get all elements of a list"
    verb: "GET"
    parameters:
      - name: "idListe"
        description: "ID of the list"
        required: true

  - id: "listeElement_getAllProcessed"
    text: "get all processed elements"
    description: "Get all processed elements"
    verb: "GET"
    parameters: []

  - id: "listeElement_getAllRestricted"
    text: "get all restricted list elements"
    description: "Get all restricted elements of a list"
    verb: "GET"
    parameters:
      - name: "idListe"
        description: "ID of the list"
        required: true

  - id: "listeElement_saveAll"
    text: "save all list elements"
    description: "Save all elements of a list"
    verb: "POST"
    parameters:
      - name: "listeElements"
        description: "List elements data"
        required: true

  - id: "listeElement_save"
    text: "save list element"
    description: "Save an element of a list"
    verb: "POST"
    parameters:
      - name: "listeElement"
        description: "List element data"
        required: true

  - id: "liste_getAllExceptCategories"
    text: "get all lists except categories"
    description: "Get all lists except categories"
    verb: "GET"
    parameters: []

  - id: "liste_getAllCategories"
    text: "get all categories"
    description: "Get all categories"
    verb: "GET"
    parameters: []

  - id: "mesureCDSAS_create"
    text: "create CDSAS measure"
    description: "Create a new CDSAS measure"
    verb: "POST"
    parameters:
      - name: "mesureCDSAS"
        description: "CDSAS measure data"
        required: true

  - id: "mesureCDSAS_update"
    text: "update CDSAS measure"
    description: "Update a CDSAS measure"
    verb: "PUT"
    parameters:
      - name: "mesureCDSAS"
        description: "CDSAS measure data"
        required: true

  - id: "mesureCDSAS_forProcedure"
    text: "get CDSAS measure for procedure"
    description: "Get CDSAS measure for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "mesureCDSAS_getCDSASForEnqueteCsCDSAS"
    text: "get CDSAS measures for CS investigation"
    description: "Get CDSAS measures for a CS investigation and accused person"
    verb: "GET"
    parameters:
      - name: "idMisEnCause"
        description: "ID of the accused person"
        required: true
      - name: "idEnqueteCS"
        description: "ID of the CS investigation"
        required: true

  - id: "mesureCDSAS_openForUser"
    text: "get open CDSAS measures for user"
    description: "Get open CDSAS measures for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "mesureCDSAS_closedForUser"
    text: "get closed CDSAS measures for user"
    description: "Get closed CDSAS measures for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "mesureCDSAS_getForEnqueteCS"
    text: "get CDSAS measures for CS investigation"
    description: "Get CDSAS measures for a CS investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteCS"
        description: "ID of the CS investigation"
        required: true

  - id: "mesureCDSAS_getForEnqueteAOMC"
    text: "get CDSAS measures for AOMC investigation"
    description: "Get CDSAS measures for an AOMC investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteAOMC"
        description: "ID of the AOMC investigation"
        required: true

  - id: "mesureCDSAS_getForEnqueteCSResume"
    text: "get CDSAS measure summaries for CS investigation"
    description: "Get CDSAS measure summaries for a CS investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteCS"
        description: "ID of the CS investigation"
        required: true

  - id: "mesureCDSAS_getCDSASForEnqueteAOMCCDSAS"
    text: "get CDSAS measures for AOMC investigation"
    description: "Get CDSAS measures for an AOMC investigation and accused person"
    verb: "GET"
    parameters:
      - name: "idMisEnCause"
        description: "ID of the accused person"
        required: true
      - name: "idEnqueteAOMC"
        description: "ID of the AOMC investigation"
        required: true

  - id: "mesureMedecinCantonal_create"
    text: "create cantonal doctor measure"
    description: "Create a new cantonal doctor measure"
    verb: "POST"
    parameters:
      - name: "mesureMedecinCantonal"
        description: "Cantonal doctor measure data"
        required: true

  - id: "mesureMedecinCantonal_update"
    text: "update cantonal doctor measure"
    description: "Update a cantonal doctor measure"
    verb: "PUT"
    parameters:
      - name: "mesureMedecinCantonal"
        description: "Cantonal doctor measure data"
        required: true

  - id: "mesureMedecinCantonal_forProcedure"
    text: "get cantonal doctor measure for procedure"
    description: "Get cantonal doctor measure for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "mesureMedecinCantonal_getMMCForEnqueteOMCMMC"
    text: "get cantonal doctor measures for OMC investigation"
    description: "Get cantonal doctor measures for an OMC investigation and accused person"
    verb: "GET"
    parameters:
      - name: "idMisEnCause"
        description: "ID of the accused person"
        required: true
      - name: "idEnqueteOMC"
        description: "ID of the OMC investigation"
        required: true

  - id: "mesureMedecinCantonal_openForUser"
    text: "get open cantonal doctor measures for user"
    description: "Get open cantonal doctor measures for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "mesureMedecinCantonal_closedForUser"
    text: "get closed cantonal doctor measures for user"
    description: "Get closed cantonal doctor measures for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "mesureMedecinCantonal_getForEnqueteOMC"
    text: "get cantonal doctor measures for OMC investigation"
    description: "Get cantonal doctor measures for an OMC investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteOMC"
        description: "ID of the OMC investigation"
        required: true

  - id: "mesureMedecinCantonal_getForEnqueteOMCResume"
    text: "get cantonal doctor measure summaries for OMC investigation"
    description: "Get cantonal doctor measure summaries for an OMC investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteOMC"
        description: "ID of the OMC investigation"
        required: true

  - id: "pleniere_get"
    text: "get plenary"
    description: "Retrieve plenary details by ID"
    verb: "GET"
    parameters:
      - name: "id"
        description: "Plenary ID"
        required: true
      - name: "userPrincipalName"
        description: "User principal name"
        required: true

  - id: "pleniere_planifiees"
    text: "get planned plenaries"
    description: "Get planned plenaries"
    verb: "GET"
    parameters: []

  - id: "pleniere_passees"
    text: "get past plenaries"
    description: "Get past plenaries"
    verb: "GET"
    parameters: []

  - id: "pleniere_create"
    text: "create plenary"
    description: "Create a new plenary"
    verb: "POST"
    parameters:
      - name: "pleniere"
        description: "Plenary data"
        required: true

  - id: "pleniere_update"
    text: "update plenary"
    description: "Update a plenary"
    verb: "PUT"
    parameters:
      - name: "pleniere"
        description: "Plenary data"
        required: true

  - id: "pleniere_numeroUnique"
    text: "check unique plenary number"
    description: "Check if plenary number is unique"
    verb: "GET"
    parameters:
      - name: "id"
        description: "ID of the plenary"
        required: true
      - name: "datePleniere"
        description: "Date of the plenary"
        required: true

  - id: "pleniere_getPlenieresForProcedure"
    text: "get plenaries for procedure"
    description: "Get plenaries for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "pleniere_getForProcedure"
    text: "get plenary summaries for procedure"
    description: "Get plenary summaries for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "pleniere_getMisEnCauseForPleniere"
    text: "get accused for plenary"
    description: "Get accused persons for a plenary"
    verb: "GET"
    parameters:
      - name: "id"
        description: "ID of the plenary"
        required: true

  - id: "pleniere_exportProcedureAssociees"
    text: "export associated procedures"
    description: "Export associated procedures"
    verb: "GET"
    parameters:
      - name: "idPleniere"
        description: "ID of the plenary"
        required: true

  - id: "pleniere_exportPleniere"
    text: "export plenary"
    description: "Export plenary"
    verb: "GET"
    parameters:
      - name: "idPleniere"
        description: "ID of the plenary"
        required: true

  - id: "signalement_recherche"
    text: "search reports"
    description: "Search for reports"
    verb: "GET"
    parameters:
      - name: "recherche"
        description: "Search query"
        required: true

  - id: "signalement_create"
    text: "create report"
    description: "Create a new report"
    verb: "POST"
    parameters:
      - name: "signalement"
        description: "Report data"
        required: true

  - id: "signalement_update"
    text: "update report"
    description: "Update a report"
    verb: "PUT"
    parameters:
      - name: "signalement"
        description: "Report data"
        required: true

  - id: "signalement_enCours"
    text: "get ongoing reports"
    description: "Get ongoing reports"
    verb: "GET"
    parameters: []

  - id: "signalement_forProcedure"
    text: "get report for procedure"
    description: "Get report for a procedure"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "signalement_signalementsMemeSignalant"
    text: "get reports from same reporter"
    description: "Get reports from the same reporter"
    verb: "GET"
    parameters:
      - name: "idSignalant"
        description: "ID of the reporter"
        required: true
      - name: "idSignalement"
        description: "ID of the report"
        required: false

  - id: "signalement_effectuerAction"
    text: "perform action on report"
    description: "Perform an action on a report"
    verb: "POST"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true
      - name: "idAction"
        description: "ID of the action"
        required: true
      - name: "date"
        description: "Date of the action"
        required: true

  - id: "signalement_transmettreCommission"
    text: "forward report to commission"
    description: "Forward a report to a commission"
    verb: "PUT"
    parameters:
      - name: "signalement"
        description: "Report data"
        required: true
      - name: "date"
        description: "Date of forwarding"
        required: false

  - id: "signalement_choisirOrientation"
    text: "choose report orientation"
    description: "Choose orientation for a report"
    verb: "PUT"
    parameters:
      - name: "signalement"
        description: "Report data"
        required: true
      - name: "date"
        description: "Date of orientation"
        required: false

  - id: "signalement_openForUser"
    text: "get open reports for user"
    description: "Get open reports for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "signalement_closedForUser"
    text: "get closed reports for user"
    description: "Get closed reports for a user"
    verb: "GET"
    parameters:
      - name: "idUtilisateur"
        description: "ID of the user"
        required: false
      - name: "idMisEnCause"
        description: "ID of the accused"
        required: false
      - name: "idPersonneConcernee"
        description: "ID of the concerned person"
        required: false

  - id: "signalement_getForEnqueteCS"
    text: "get reports for CS investigation"
    description: "Get reports for a CS investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteCS"
        description: "ID of the CS investigation"
        required: true

  - id: "signalement_getForEnqueteCSResume"
    text: "get report summaries for CS investigation"
    description: "Get report summaries for a CS investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteCS"
        description: "ID of the CS investigation"
        required: true

  - id: "signalement_getForEnquetePenale"
    text: "get reports for criminal investigation"
    description: "Get reports for a criminal investigation"
    verb: "GET"
    parameters:
      - name: "idEnquetePenale"
        description: "ID of the criminal investigation"
        required: true

  - id: "signalement_getForEnqueteCop"
    text: "get reports for COP investigation"
    description: "Get reports for a COP investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteCop"
        description: "ID of the COP investigation"
        required: true

  - id: "signalement_getForEnqueteOMC"
    text: "get reports for OMC investigation"
    description: "Get reports for an OMC investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteOMC"
        description: "ID of the OMC investigation"
        required: true

  - id: "signalement_getForEnqueteAOMC"
    text: "get reports for AOMC investigation"
    description: "Get reports for an AOMC investigation"
    verb: "GET"
    parameters:
      - name: "idEnqueteAOMC"
        description: "ID of the AOMC investigation"
        required: true

  - id: "signalement_getForPleniere"
    text: "get reports for plenary"
    description: "Get reports for a plenary session"
    verb: "GET"
    parameters:
      - name: "idPleniere"
        description: "ID of the plenary session"
        required: true

  - id: "signalement_getForCommission"
    text: "get reports for commission"
    description: "Get reports for a commission"
    verb: "GET"
    parameters:
      - name: "idCommission"
        description: "ID of the commission"
        required: true

  - id: "signalement_genererFicheInstructionPlainte"
    text: "generate complaint instruction sheet"
    description: "Generate complaint instruction sheet"
    verb: "GET"
    parameters:
      - name: "idProcedure"
        description: "ID of the procedure"
        required: true

  - id: "signalement_getAllSignalement"
    text: "get all reports in range"
    description: "Get all reports in a date range"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "statistique_getSignalementByEtatFilterByPeriode"
    text: "get reports by state filtered by period"
    description: "Get reports by state filtered by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "statistique_getLienSignalantPatientFilterByPeriode"
    text: "get reporter-patient links filtered by period"
    description: "Get reporter-patient links filtered by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "statistique_getMotifsPlaintesFilterByPeriode"
    text: "get complaint motifs filtered by period"
    description: "Get complaint motifs filtered by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "statistique_getSubMotifsPlaintesFilterByPeriode"
    text: "get complaint sub-motifs filtered by period"
    description: "Get complaint sub-motifs filtered by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "statistique_getSignalementByTypePersonneFilterByPeriode"
    text: "get reports by person type filtered by period"
    description: "Get reports by person type filtered by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "statistique_getCDSASByTypeFilterByPeriode"
    text: "get CDSAS by type filtered by period"
    description: "Get CDSAS by type filtered by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "statistique_getCDSASByProfessionFilterByPeriode"
    text: "get CDSAS by profession filtered by period"
    description: "Get CDSAS by profession filtered by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "statistique_getEnqueteCSByProfessionOuvertureByPeriode"
    text: "get CS investigations by profession opening by period"
    description: "Get CS investigations by profession opening by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "statistique_getEnqueteCSByAgeMECByPeriode"
    text: "get CS investigations by accused age by period"
    description: "Get CS investigations by accused age by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true
      - name: "civilite"
        description: "Civility"
        required: true

  - id: "statistique_getEnqueteCSByProvenanceDiplomeByPeriode"
    text: "get CS investigations by diploma origin by period"
    description: "Get CS investigations by diploma origin by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "statistique_getEnqueteCSByCategorieEvtByPeriode"
    text: "get CS investigations by event category by period"
    description: "Get CS investigations by event category by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true

  - id: "statistique_getEnqueteCSByEtatFilterByPeriode"
    text: "get CS investigations by state filtered by period"
    description: "Get CS investigations by state filtered by period"
    verb: "GET"
    parameters:
      - name: "periodeDu"
        description: "Start date period"
        required: true
      - name: "periodeAu"
        description: "End date period"
        required: true
      - name: "userPrincipalName"
        description: "User principal name"
        required: true

  - id: "trigramme_getAll"
    text: "get all trigrams"
    description: "Get all user trigrams"
    verb: "GET"
    parameters: []

  - id: "trigramme_create"
    text: "create trigram"
    description: "Create a new user trigram"
    verb: "POST"
    parameters:
      - name: "userTrigramme"
        description: "User trigram data"
        required: true

  - id: "trigramme_update"
    text: "update trigram"
    description: "Update a user trigram"
    verb: "PUT"
    parameters:
      - name: "userTrigramme"
        description: "User trigram data"
        required: true

  - id: "trigramme_delete"
    text: "delete trigram"
    description: "Delete a user trigram"
    verb: "DELETE"
    parameters:
      - name: "id"
        description: "ID of the user trigram"
        required: true
````

## File: src/endpoint_store/authorized_domains.rs
````rust
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};

/// Get all authorized domains (system-wide for CORS)
pub async fn get_all_authorized_domains(store: &EndpointStore) -> Result<Vec<String>, StoreError> {
    let client = store.get_conn().await?;

    tracing::debug!("Fetching all authorized domains");

    let rows = client
        .query(
            "SELECT DISTINCT domain FROM domains WHERE verified = true OR email = 'system' ORDER BY domain",
            &[],
        )
        .await
        .to_store_error()?;

    let domains: Vec<String> = rows.iter().map(|row| row.get(0)).collect();

    tracing::debug!(
        domain_count = domains.len(),
        "Retrieved authorized domains from database"
    );

    // Add default system domains if none exist
    if domains.is_empty() {
        tracing::info!("No domains found in database, returning default system domains");
        return Ok(vec![
            "https://studio.cvenom.com".to_string(),
            "https://app.api0.ai".to_string(),
            "http://localhost:3000".to_string(),
            "http://localhost:5173".to_string(),
        ]);
    }

    Ok(domains)
}

/// Initialize default system domains
pub async fn initialize_system_domains(store: &EndpointStore) -> Result<(), StoreError> {
    let client = store.get_conn().await?;

    // Check if system domains already exist
    let count_row = client
        .query_one("SELECT COUNT(*) FROM domains WHERE email = 'system'", &[])
        .await
        .to_store_error()?;

    let count: i64 = count_row.get(0);

    if count > 0 {
        tracing::debug!("System domains already initialized");
        return Ok(());
    }

    tracing::info!("Initializing default system domains");

    let system_domains = vec![
        "https://studio.cvenom.com",
        "https://app.api0.ai",
        "http://localhost:3000",
        "http://localhost:5173",
    ];

    for domain in system_domains {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        client
            .execute(
                "INSERT INTO domains (id, email, domain, verified, added_at) VALUES ($1, 'system', $2, true, $3) ON CONFLICT DO NOTHING",
                &[&id, &domain, &now],
            )
            .await  // <-- Added .await here
            .to_store_error()?;
    }

    tracing::info!("System domains initialized successfully");
    Ok(())
}
````

## File: src/endpoint_store/manage_single_endpoint.rs
````rust
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{Endpoint, EndpointStore, StoreError};

/// Manages (adds or updates) a single endpoint
pub async fn manage_single_endpoint(
    store: &EndpointStore,
    email: &str,
    endpoint: &Endpoint,
) -> Result<String, StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let endpoint_id = &endpoint.id;
    let group_id = &endpoint.group_id;

    tracing::info!(
        email = %email,
        endpoint_id = %endpoint_id,
        group_id = %group_id,
        "Managing single endpoint"
    );

    // Check if user has access to this group
    let user_has_group_row = tx
        .query_opt(
            "SELECT 1 FROM user_groups WHERE email = $1 AND group_id = $2",
            &[&email, group_id],
        )
        .await
        .to_store_error()?;

    if user_has_group_row.is_none() {
        return Err(StoreError::Database(
            "User does not have access to this API group".to_string(),
        ));
    }

    // Check if endpoint exists
    let endpoint_exists_row = tx
        .query_opt("SELECT 1 FROM endpoints WHERE id = $1", &[endpoint_id])
        .await
        .to_store_error()?;

    let operation_type = if endpoint_exists_row.is_some() {
        // Update existing endpoint
        tx.execute(
            "UPDATE endpoints SET text = $1, description = $2, verb = $3, base = $4, path = $5, group_id = $6 WHERE id = $7",
            &[
                &endpoint.text,
                &endpoint.description,
                &endpoint.verb,
                &endpoint.base,
                &endpoint.path,
                group_id,
                endpoint_id,
            ],
        )
        .await
        .to_store_error()?;

        // Ensure user-endpoint association exists
        tx.execute(
            "INSERT INTO user_endpoints (email, endpoint_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            &[&email, endpoint_id],
        )
        .await
        .to_store_error()?;

        "updated"
    } else {
        // Create new endpoint
        tx.execute(
            "INSERT INTO endpoints (id, text, description, verb, base, path, group_id) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                endpoint_id,
                &endpoint.text,
                &endpoint.description,
                &endpoint.verb,
                &endpoint.base,
                &endpoint.path,
                group_id,
            ],
        )
        .await
        .to_store_error()?;

        // Associate endpoint with user
        tx.execute(
            "INSERT INTO user_endpoints (email, endpoint_id) VALUES ($1, $2)",
            &[&email, endpoint_id],
        )
        .await
        .to_store_error()?;

        "created"
    };

    // Clean up existing parameters
    tx.execute(
        "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
        &[endpoint_id],
    )
    .await
    .to_store_error()?;

    tx.execute(
        "DELETE FROM parameters WHERE endpoint_id = $1",
        &[endpoint_id],
    )
    .await
    .to_store_error()?;

    // Add parameters
    for param in &endpoint.parameters {
        let required = param.required.parse::<bool>().unwrap_or(false);

        tx.execute(
            "INSERT INTO parameters (endpoint_id, name, description, required) VALUES ($1, $2, $3, $4)",
            &[endpoint_id, &param.name, &param.description, &required],
        )
        .await
        .to_store_error()?;

        // Add parameter alternatives
        for alt in &param.alternatives {
            tx.execute(
                "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) VALUES ($1, $2, $3)",
                &[endpoint_id, &param.name, alt],
            )
            .await
            .to_store_error()?;
        }
    }

    tx.commit().await.to_store_error()?;

    tracing::info!(
        email = %email,
        endpoint_id = %endpoint.id,
        operation = %operation_type,
        "Successfully managed endpoint"
    );

    Ok(operation_type.to_string())
}
````

## File: src/delete_api_group.rs
````rust
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for deleting an API group
pub async fn delete_api_group(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>,
) -> impl Responder {
    let (email, group_id) = path_params.into_inner();

    tracing::info!(
        email = %email,
        group_id = %group_id,
        "Received HTTP delete API group request"
    );

    match store.delete_user_api_group(&email, &group_id).await {
        Ok(deleted) => {
            if deleted {
                tracing::info!(
                    email = %email,
                    group_id = %group_id,
                    "Successfully deleted API group"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "API group and its endpoints successfully deleted"
                }))
            } else {
                tracing::warn!(
                    email = %email,
                    group_id = %group_id,
                    "API group not found or not deletable"
                );
                HttpResponse::NotFound().json(serde_json::json!({
                    "success": false,
                    "message": "API group not found or is a default group that cannot be deleted"
                }))
            }
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                group_id = %group_id,
                "Failed to delete API group"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to delete API group: {}", e)
            }))
        }
    }
}
````

## File: src/formatter.rs
````rust
use reqwest::multipart::{Form, Part};
use std::error::Error;
use std::io::Write;
use tempfile::NamedTempFile;
use tracing::{error, info};

#[derive(Clone)]
pub struct YamlFormatter {
    formatter_url: String,
}

impl YamlFormatter {
    pub fn new(formatter_url: &str) -> Self {
        Self {
            formatter_url: formatter_url.to_string(),
        }
    }

    /// Format a YAML file using the formatter service
    ///
    /// # Arguments
    /// * `content` - The file content as bytes
    /// * `filename` - The original filename (for logging purposes)
    ///
    /// # Returns
    /// * `Result<Vec<u8>, Box<dyn Error>>` - The formatted content as bytes
    pub async fn format_yaml(
        &self,
        content: &[u8],
        filename: &str,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        info!(
            filename = %filename,
            "Formatting YAML file through formatter service"
        );

        // Create a temporary file with the content
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(content)?;
        let _temp_path = temp_file.path().to_string_lossy().to_string();

        // Create a multipart form with the file
        // Create a multipart form with the content directly
        let part = Part::bytes(content.to_vec()).file_name(filename.to_string());

        let form = Form::new().part("file", part);
        // Send the request to the formatter service
        let client = reqwest::Client::new();
        let response = client
            .post(&self.formatter_url)
            .multipart(form)
            .send()
            .await?;

        // Check if request was successful
        if !response.status().is_success() {
            let error_message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error from formatter service".to_string());
            error!(error = %error_message, "Formatter service returned an error");
            return Err(format!("Failed to format YAML: {}", error_message).into());
        }

        // Get the formatted content
        let formatted_content = response.bytes().await?.to_vec();

        info!("Successfully formatted YAML file");
        Ok(formatted_content)
    }
}
````

## File: src/get_api_key_usage.rs
````rust
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;
// use actix_web::{web, HttpResponse, Responder};

pub async fn get_api_key_usage(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP get API key usage request");

    match store.get_api_key_usage(&email).await {
        Ok(usage) => {
            tracing::info!(
                email = %email,
                "Successfully retrieved API key usage"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "usage": usage,
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to retrieve API key usage"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error: {}", e),
            }))
        }
    }
}
````

## File: src/get_api_usage_logs.rs
````rust
// src/get_api_usage_logs.rs
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::EndpointStore;

/// Handler for getting detailed API usage logs with token information
pub async fn get_api_usage_logs(
    store: web::Data<Arc<EndpointStore>>,
    path_params: web::Path<(String, String)>, // (email, key_id)
    query: web::Query<std::collections::HashMap<String, String>>,
) -> impl Responder {
    let (email, key_id) = path_params.into_inner();

    // Extract limit from query parameters if provided
    let limit = query.get("limit").and_then(|l| l.parse::<i64>().ok());

    tracing::info!(
        email = %email,
        key_id = %key_id,
        limit = limit,
        "Received HTTP get API usage logs request"
    );

    match store.get_api_usage_logs(&key_id, limit).await {
        Ok(logs) => {
            tracing::info!(
                email = %email,
                key_id = %key_id,
                log_count = logs.len(),
                "Successfully retrieved API usage logs with token data"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "logs": logs,
                "count": logs.len(),
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                key_id = %key_id,
                "Failed to retrieve API usage logs"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to retrieve API usage logs: {}", e),
            }))
        }
    }
}
````

## File: test/test_api_key.sh
````bash
#!/bin/bash

# Test script for API key management

# Configuration
HOST="127.0.0.1:9090"         # HTTP server address
TEST_EMAIL="test@example.com" # Test email
KEY_NAME="Test API Key"       # Name for the API key

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing API Key Management${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Function to test getting API key status
test_get_key_status() {
  echo -e "${YELLOW}Testing: Get API Key Status${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/key/$TEST_EMAIL")

  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test generating a new API key
test_generate_key() {
  echo -e "${YELLOW}Testing: Generate API Key${NC}"
  echo "Email: $TEST_EMAIL, Key Name: $KEY_NAME"
  echo "-----------------"

  REQUEST_PAYLOAD=$(
    cat <<EOF
{
  "email": "$TEST_EMAIL",
  "key_name": "$KEY_NAME"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/key")

  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"

  # Extract and save the API key for later tests
  API_KEY=$(echo "$response" | jq -r '.key')
  if [ "$API_KEY" != "null" ]; then
    echo -e "${GREEN}API Key: $API_KEY${NC}"
    # Save to a temp file for later tests
    echo "$API_KEY" >/tmp/api_key.txt
  else
    echo -e "${RED}Failed to extract API key from response${NC}"
  fi

  echo
}

# Function to test an endpoint with API key
test_endpoint_with_key() {
  API_KEY=$(cat /tmp/api_key.txt)

  echo -e "${YELLOW}Testing: Access API with Key${NC}"
  echo "API Key: ${API_KEY:0:10}..."
  echo "-----------------"

  response=$(curl -s -X GET -H "X-API-Key: $API_KEY" "$HOST/api/groups/$TEST_EMAIL")

  echo "Response (truncated):"
  echo "$response" | jq '.success, .message, (.api_groups | length) as $len | "Number of groups: \($len)"'
  echo "-----------------"
  echo
}

# Function to test getting API key usage
test_get_key_usage() {
  echo -e "${YELLOW}Testing: Get API Key Usage${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/usage/$TEST_EMAIL")

  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test revoking an API key
test_revoke_key() {
  echo -e "${YELLOW}Testing: Revoke API Key${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X DELETE "$HOST/api/user/key/$TEST_EMAIL")

  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Main execution
print_header

# Initial state
echo -e "${BLUE}Initial API key status:${NC}"
test_get_key_status

# Generate a new API key
echo -e "${BLUE}Generating a new API key:${NC}"
test_generate_key

# Verify key status after generation
echo -e "${BLUE}API key status after generation:${NC}"
test_get_key_status

# Test an endpoint with the API key
echo -e "${BLUE}Testing an endpoint with the API key:${NC}"
test_endpoint_with_key

# Check key usage
echo -e "${BLUE}Checking API key usage:${NC}"
test_get_key_usage

# Revoke the API key
# echo -e "${BLUE}Revoking the API key:${NC}"
# test_revoke_key

# Final state
echo -e "${BLUE}Final API key status:${NC}"
test_get_key_status

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"

# Clean up
rm -f /tmp/api_key.txt
````

## File: test/test_user_preferences.sh
````bash
#!/bin/bash
# test/test_user_preferences.sh

# Configuration
HOST="127.0.0.1:9090"  # HTTP server address
TEST_EMAIL="test@example.com"  # Test email

# Color codes for output
GREEN='\033[0;32m'
RED='\033[0;31m'
BLUE='\033[0;34m'
YELLOW='\033[0;33m'
NC='\033[0m' # No Color

# Function to display header
print_header() {
  echo -e "${BLUE}=======================================================${NC}"
  echo -e "${BLUE}  Testing User Preferences API${NC}"
  echo -e "${BLUE}=======================================================${NC}"
  echo
}

# Function to check if curl is installed
check_dependencies() {
  if ! command -v curl &>/dev/null; then
    echo -e "${RED}Error: curl is not installed${NC}"
    echo "Please install curl to run this test"
    exit 1
  fi

  if ! command -v jq &>/dev/null; then
    echo -e "${RED}Error: jq is not installed${NC}"
    echo "Please install jq to run this test"
    exit 1
  }
}

# Function to test getting user preferences
test_get_preferences() {
  echo -e "${YELLOW}Testing: Get User Preferences${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/user/preferences/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test updating user preferences
test_update_preferences() {
  local action=$1
  local endpoint_id=$2
  
  echo -e "${YELLOW}Testing: Update User Preferences ($action)${NC}"
  echo "Email: $TEST_EMAIL, Endpoint ID: $endpoint_id"
  echo "-----------------"

  REQUEST_PAYLOAD=$(cat <<EOF
{
  "email": "$TEST_EMAIL",
  "action": "$action",
  "endpoint_id": "$endpoint_id"
}
EOF
  )

  echo "Request payload:"
  echo "$REQUEST_PAYLOAD" | jq .
  echo "-----------------"

  response=$(curl -s -X POST -H "Content-Type: application/json" -d "$REQUEST_PAYLOAD" "$HOST/api/user/preferences")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test resetting user preferences
test_reset_preferences() {
  echo -e "${YELLOW}Testing: Reset User Preferences${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X DELETE "$HOST/api/user/preferences/$TEST_EMAIL")
  
  echo "Response:"
  echo "$response" | jq .
  echo "-----------------"
  echo
}

# Function to test getting API groups with preferences applied
test_get_api_groups() {
  echo -e "${YELLOW}Testing: Get API Groups with Preferences Applied${NC}"
  echo "Email: $TEST_EMAIL"
  echo "-----------------"

  response=$(curl -s -X GET "$HOST/api/groups/$TEST_EMAIL")
  
  echo "Response (truncated):"
  echo "$response" | jq '.success, .message, (.api_groups | length) as $len | "Number of groups: \($len)"'
  echo "-----------------"
  echo
}

# Main execution
print_header
check_dependencies

# Initial state
echo -e "${BLUE}Initial state:${NC}"
test_get_preferences

# Get default endpoints
response=$(curl -s -X GET "$HOST/api/groups/$TEST_EMAIL")
default_endpoint_id=$(echo "$response" | jq -r '.api_groups[0].endpoints[0].id')

if [ -z "$default_endpoint_id" ] || [ "$default_endpoint_id" == "null" ]; then
  echo -e "${RED}No endpoints found for testing!${NC}"
  exit 1
fi

echo -e "${GREEN}Found endpoint ID for testing: $default_endpoint_id${NC}"
echo

# Hide a default endpoint
echo -e "${BLUE}Hiding a default endpoint:${NC}"
test_update_preferences "hide_default" "$default_endpoint_id"

# Get user preferences to verify
echo -e "${BLUE}Verifying preferences after hiding:${NC}"
test_get_preferences

# Test get API groups to see if hiding worked
echo -e "${BLUE}Verifying API groups after hiding:${NC}"
test_get_api_groups

# Show the default endpoint again
echo -e "${BLUE}Showing the default endpoint again:${NC}"
test_update_preferences "show_default" "$default_endpoint_id"

# Get user preferences to verify
echo -e "${BLUE}Verifying preferences after showing:${NC}"
test_get_preferences

# Reset preferences
echo -e "${BLUE}Resetting all preferences:${NC}"
test_reset_preferences

# Final state
echo -e "${BLUE}Final state:${NC}"
test_get_preferences

echo -e "${BLUE}=======================================================${NC}"
echo -e "${BLUE}  Test Completed${NC}"
echo -e "${BLUE}=======================================================${NC}"
````

## File: apinext.md
````markdown
# API Key Implementation Summary

## Overview of Changes

We've successfully implemented a complete API key management system for the api0 Store backend. Here's a summary of the changes made:

1. **Database Updates**:
   - Extended the `user_preferences` table with API key fields
   - Created migration scripts for safe schema updates

2. **Core API Key Management**:
   - Implemented secure API key generation with the `sk_live_` prefix
   - Added API key hashing using SHA-256 for secure storage
   - Created API key validation and usage tracking

3. **HTTP API Endpoints**:
   - Added endpoints for API key management:
     - GET `/api/user/key/{email}` - Get key status
     - POST `/api/user/key` - Generate new key
     - DELETE `/api/user/key/{email}` - Revoke key
     - GET `/api/user/usage/{email}` - Get usage analytics

4. **Authentication**:
   - Implemented an API key authentication middleware
   - Added support for the `X-API-Key` header
   - Created user identification and tracking

5. **Testing**:
   - Added test scripts for API key functionality

## Next Steps

To complete the implementation and enhance the API key system, consider these next steps:

1. **Enhanced Security**:
   - Implement rate limiting per API key
   - Add IP-based access restrictions
   - Create more comprehensive logging for security events

2. **Usage Analytics**:
   - Enhance usage tracking with endpoint-specific analytics
   - Add response time and error rate tracking
   - Create a dashboard for usage visualization

3. **Credit System Integration**:
   - Implement the credit balance system
   - Add usage quotas and limits
   - Create a billing/credit replenishment system

4. **Operational Improvements**:
   - Add key expiration functionality
   - Implement key rotation policies
   - Add support for multiple API keys per user

5. **Integration Tests**:
   - Create comprehensive integration tests
   - Add load testing for API key performance

## Frontend Integration Notes

The backend implementation is fully compatible with the frontend requirements. The frontend components should:

1. Fetch API key status with `GET /api/user/key/{email}`
2. Generate new keys with `POST /api/user/key`
3. Revoke keys with `DELETE /api/user/key/{email}`
4. Display usage analytics from `GET /api/user/usage/{email}`

All API responses match the requested formats, and authentication is handled through the `X-API-Key` header as specified.

## Deployment Considerations

When deploying this update:

1. **Database Migration**: Ensure the migration script runs to add the new fields
2. **Configuration**: Update any configuration needed for API key settings
3. **Testing**: Run the provided test scripts to verify functionality
4. **Documentation**: Update API documentation to include the new endpoints
5. **Monitoring**: Add monitoring for API key usage and authentication events

This implementation provides a solid foundation for the API key management system, meeting all the specified requirements while integrating smoothly with the existing codebase.
````

## File: src/endpoint_store/get_default_api_groups.rs
````rust
use crate::endpoint_store::{EndpointStore, StoreError, ApiGroup, ApiGroupWithEndpoints};
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::get_endpoints_by_group_id;

/// Gets the default API groups from the database
pub(crate) async fn get_default_api_groups(
    store: &EndpointStore,
) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
    tracing::info!("Fetching default API groups from database");
    
    // First get all default groups in a single transaction scope
    let groups: Vec<ApiGroup> = {
        let mut conn = store.get_conn().await?;
        let tx = conn.transaction().to_store_error()?;

        // Check if there are any default groups
        let default_count: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM api_groups WHERE is_default = true",
                [],
                |row| row.get(0),
            )
            .to_store_error()?;

        tracing::info!(
            count = default_count,
            "Found default API groups in database"
        );

        if default_count == 0 {
            tracing::warn!("No default API groups found in database");
            // Commit empty transaction before returning
            tx.commit().to_store_error()?;
            return Ok(Vec::new());
        }

        // Get all default groups - scope the statement properly
        let groups = {
            let mut stmt = tx
                .prepare("SELECT id, name, description, base FROM api_groups WHERE is_default = true")
                .map_err(|e| {
                    tracing::error!(error = %e, "Failed to prepare statement for fetching default groups");
                    StoreError::Database(e.to_string())
                })?;

            let groups_iter = stmt.query_map([], |row| {
                Ok(ApiGroup {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    description: row.get(2)?,
                    base: row.get(3)?,
                })
            })
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to query default groups");
                StoreError::Database(e.to_string())
            })?;

            groups_iter.collect::<Result<Vec<_>, _>>()
                .map_err(|e| {
                    tracing::error!(error = %e, "Failed to process group result");
                    StoreError::Database(e.to_string())
                })?
        }; // stmt is dropped here

        // Now we can commit the transaction
        tx.commit().to_store_error()?;
        
        groups
    }; // Transaction and connection are dropped here

    // Now fetch endpoints for each group with separate connections
    let mut result = Vec::new();
    for group in groups {
        tracing::debug!(
            group_id = %group.id,
            group_name = %group.name,
            "Processing default group"
        );
        
        match get_endpoints_by_group_id(store, &group.id).await {
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
````

## File: src/endpoint_store/user_preferences.rs
````rust
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError, UserPreferences};

/// Get user preferences by email
pub async fn get_user_preferences(
    store: &EndpointStore,
    email: &str,
) -> Result<UserPreferences, StoreError> {
    let client = store.get_conn().await?;

    let row = client
        .query_opt(
            "SELECT hidden_defaults FROM user_preferences WHERE email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    match row {
        Some(r) => {
            let hidden_defaults_str: String = r.get(0);
            let hidden_defaults = if hidden_defaults_str.is_empty() {
                Vec::new()
            } else {
                hidden_defaults_str.split(',').map(String::from).collect()
            };

            Ok(UserPreferences {
                email: email.to_string(),
                hidden_defaults,
            })
        }
        None => Ok(UserPreferences {
            email: email.to_string(),
            hidden_defaults: Vec::new(),
        }),
    }
}

/// Update user preferences
pub async fn update_user_preferences(
    store: &EndpointStore,
    email: &str,
    action: &str,
    endpoint_id: &str,
) -> Result<bool, StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let row = tx
        .query_opt(
            "SELECT hidden_defaults FROM user_preferences WHERE email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    let exists = row.is_some();
    let hidden_defaults = if let Some(r) = row {
        let hidden_defaults_str: String = r.get(0);
        if hidden_defaults_str.is_empty() {
            Vec::new()
        } else {
            hidden_defaults_str
                .split(',')
                .map(String::from)
                .collect::<Vec<String>>()
        }
    } else {
        Vec::new()
    };

    let mut updated_hidden_defaults = hidden_defaults.clone();

    match action {
        "hide_default" => {
            if !updated_hidden_defaults.contains(&endpoint_id.to_string()) {
                updated_hidden_defaults.push(endpoint_id.to_string());
            }
        }
        "show_default" => {
            updated_hidden_defaults.retain(|id| id != endpoint_id);
        }
        _ => {
            return Err(StoreError::Database(format!("Invalid action: {}", action)));
        }
    }

    let updated_hidden_defaults_str = updated_hidden_defaults.join(",");

    if exists {
        tx.execute(
            "UPDATE user_preferences SET hidden_defaults = $1 WHERE email = $2",
            &[&updated_hidden_defaults_str, &email],
        )
        .await
        .to_store_error()?;
    } else {
        tx.execute(
            "INSERT INTO user_preferences (email, hidden_defaults) VALUES ($1, $2)",
            &[&email, &updated_hidden_defaults_str],
        )
        .await
        .to_store_error()?;
    }

    tx.commit().await.to_store_error()?;
    Ok(true)
}

/// Reset user preferences
pub async fn reset_user_preferences(
    store: &EndpointStore,
    email: &str,
) -> Result<bool, StoreError> {
    let client = store.get_conn().await?;

    client
        .execute("DELETE FROM user_preferences WHERE email = $1", &[&email])
        .await
        .to_store_error()?;

    Ok(true)
}
````

## File: src/add_api_group.rs
````rust
use crate::{
    endpoint_store::{generate_id_from_text, EndpointStore},
    models::AddApiGroupRequest,
};

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

pub async fn add_api_group(
    store: web::Data<Arc<EndpointStore>>,
    add_data: web::Json<AddApiGroupRequest>,
) -> impl Responder {
    let email = &add_data.email;
    let mut api_group = add_data.api_group.clone();

    tracing::info!(
        email = %email,
        group_name = %api_group.group.name,
        "Received HTTP add API group request"
    );

    // Validate group data
    if api_group.group.name.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "API group name cannot be empty"
        }));
    }

    if api_group.group.base.trim().is_empty() {
        api_group.group.base = "https://api.example.com".to_string();
        // return HttpResponse::BadRequest().json(serde_json::json!({
        //     "success": false,
        //     "message": "Base URL cannot be empty"
        // }));
    }

    // Generate group ID if not provided
    if api_group.group.id.trim().is_empty() {
        api_group.group.id = generate_id_from_text(&api_group.group.name);
    }

    // Process endpoints with inheritance and auto-generation
    for endpoint in &mut api_group.endpoints {
        // Generate endpoint ID if not provided
        if endpoint.id.trim().is_empty() {
            endpoint.id = generate_id_from_text(&endpoint.text);
        }

        // Set group_id from parent group
        endpoint.group_id = api_group.group.id.clone();

        // Inherit base URL from group if endpoint base is empty
        if endpoint.base.trim().is_empty() {
            endpoint.base = api_group.group.base.clone();
        }
    }

    // Add the API group (don't replace existing ones)
    match store.add_user_api_group(email, &api_group).await {
        Ok(endpoint_count) => {
            tracing::info!(
                email = %email,
                group_id = %api_group.group.id,
                endpoint_count = endpoint_count,
                "Successfully added API group"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": "API group successfully added",
                "group_id": api_group.group.id,
                "endpoint_count": endpoint_count
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                group_id = %api_group.group.id,
                "Failed to add API group"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to add API group: {}", e)
            }))
        }
    }
}
````

## File: src/log_api_usage.rs
````rust
// src/log_api_usage.rs
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

use crate::endpoint_store::{EndpointStore, LogApiUsageRequest, LogApiUsageResponse};

/// Handler for logging detailed API usage with token information
pub async fn log_api_usage(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<LogApiUsageRequest>,
) -> impl Responder {
    let log_request = request.into_inner();

    tracing::info!(
        key_id = %log_request.key_id,
        email = %log_request.email,
        endpoint = %log_request.endpoint_path,
        method = %log_request.method,
        has_token_usage = log_request.usage.is_some(),
        total_tokens = log_request.usage.as_ref().map(|u| u.total_tokens),
        model = log_request.usage.as_ref().map(|u| u.model.as_str()).unwrap_or("none"),
        "Received HTTP log API usage request with token data"
    );

    match store.log_api_usage(&log_request).await {
        Ok(log_id) => {
            tracing::info!(
                key_id = %log_request.key_id,
                log_id = %log_id,
                total_tokens = log_request.usage.as_ref().map(|u| u.total_tokens),
                "Successfully logged API usage with token data"
            );
            HttpResponse::Ok().json(LogApiUsageResponse {
                success: true,
                message: "API usage logged successfully".to_string(),
                log_id: Some(log_id),
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                key_id = %log_request.key_id,
                "Failed to log API usage"
            );
            HttpResponse::InternalServerError().json(LogApiUsageResponse {
                success: false,
                message: format!("Failed to log API usage: {}", e),
                log_id: None,
            })
        }
    }
}
````

## File: src/manage_endpoint.rs
````rust
use crate::endpoint_store::{generate_id_from_text, Endpoint, EndpointStore};
use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

#[derive(Debug, Clone, Deserialize)]
pub struct ManageEndpointRequest {
    pub email: String,
    pub group_id: String,
    pub endpoint: Endpoint,
}

// Handler for adding or updating a single endpoint
pub async fn manage_endpoint(
    store: web::Data<Arc<EndpointStore>>,
    request: web::Json<ManageEndpointRequest>,
) -> impl Responder {
    let email = &request.email;
    let mut endpoint = request.endpoint.clone();
    let group_id = &request.group_id;

    tracing::info!(
        email = %email,
        group_id = %group_id,
        endpoint_text = %endpoint.text,
        "Received HTTP manage endpoint request"
    );

    // Validate endpoint data
    if endpoint.text.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "Endpoint text cannot be empty"
        }));
    }

    // Generate ID if not provided
    if endpoint.id.trim().is_empty() {
        endpoint.id = generate_id_from_text(&endpoint.text);
    }

    // Set group_id
    endpoint.group_id = group_id.clone();

    // If endpoint base is empty, inherit from group
    if endpoint.base.trim().is_empty() {
        match store.get_group_base_url(group_id).await {
            Ok(group_base) => {
                if group_base.trim().is_empty() {
                    endpoint.base = "https://api.example.com".to_string();
                } else {
                    endpoint.base = group_base;
                }
            }
            Err(_) => {
                endpoint.base = "https://api.example.com".to_string();
            }
        }
    }

    match store.manage_single_endpoint(email, &endpoint).await {
        Ok(operation_type) => {
            tracing::info!(
                email = %email,
                endpoint_id = %endpoint.id,
                operation = %operation_type,
                "Successfully managed endpoint"
            );
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "message": format!("Endpoint successfully {}", operation_type),
                "endpoint_id": endpoint.id,
                "operation": operation_type
            }))
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                endpoint_id = %endpoint.id,
                "Failed to manage endpoint"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to manage endpoint: {}", e)
            }))
        }
    }
}
````

## File: src/update_api_group.rs
````rust
use crate::{
    endpoint_store::{generate_id_from_text, EndpointStore},
    models::UpdateApiGroupRequest,
};

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

// Handler for updating an API group
pub async fn update_api_group(
    store: web::Data<Arc<EndpointStore>>,
    update_data: web::Json<UpdateApiGroupRequest>,
) -> impl Responder {
    let email = &update_data.email;
    let group_id = &update_data.group_id;
    let mut api_group = update_data.api_group.clone();

    tracing::info!(
        email = %email,
        group_id = %group_id,
        "Received HTTP update API group request"
    );

    // Validate group data
    if api_group.group.name.trim().is_empty() {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "success": false,
            "message": "API group name cannot be empty"
        }));
    }

    if api_group.group.base.trim().is_empty() {
        api_group.group.base = "https://api.example.com".to_string();
    }

    // Ensure group ID is consistent
    api_group.group.id = group_id.clone();

    // Set group_id on all endpoints
    for endpoint in &mut api_group.endpoints {
        // Generate endpoint ID if not provided
        if endpoint.id.trim().is_empty() {
            endpoint.id = generate_id_from_text(&endpoint.text);
        }
        endpoint.group_id = group_id.clone();
    }

    // Update API group by first deleting and then adding
    match store.delete_user_api_group(email, group_id).await {
        Ok(_) => match store.add_user_api_group(email, &api_group).await {
            Ok(endpoint_count) => {
                tracing::info!(
                    email = %email,
                    group_id = %group_id,
                    endpoint_count = endpoint_count,
                    "Successfully updated API group"
                );
                HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "message": "API group successfully updated",
                    "group_id": group_id,
                    "endpoint_count": endpoint_count
                }))
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    group_id = %group_id,
                    "Failed to add updated API group"
                );
                HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": format!("Failed to update API group: {}", e)
                }))
            }
        },
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                group_id = %group_id,
                "Failed to delete API group before update"
            );
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Failed to update API group: {}", e)
            }))
        }
    }
}
````

## File: test/query.sh
````bash
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
    endpoint.EndpointService/GetEndpoints 2>&1)

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
````

## File: .gitignore
````
/target
db
test/node_modules
````

## File: API.md
````markdown
# API Endpoints Documentation

## Overview

This document provides a comprehensive list of all endpoints exposed by the api0 Store API service. The API contains multiple endpoint categories for managing API groups, user preferences, API keys, and credit balances.

## Base URLs

- **HTTP Server**: `http://127.0.0.1:9090`
- **gRPC Server**: `127.0.0.1:50055`

## Authentication

Most endpoints require authentication using an API key. Include the API key in the `X-API-Key` header:

```
X-API-Key: sk_live_yourApiKeyHere
```

## API Groups Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/groups/{email}` | GET | Get all API groups for a user with preferences applied |
| `/api/upload` | POST | Upload API groups configuration file (YAML/JSON) |
| `/api/group` | POST | Add a new API group |
| `/api/group` | PUT | Update an existing API group |
| `/api/groups/{email}/{group_id}` | DELETE | Delete an API group |

## User Preferences Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/user/preferences/{email}` | GET | Get user preferences |
| `/api/user/preferences` | POST | Update user preferences |
| `/api/user/preferences/{email}` | DELETE | Reset user preferences |

## API Key Management Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/user/keys/{email}` | GET | Get status of all API keys for a user |
| `/api/user/keys` | POST | Generate a new API key for a user |
| `/api/user/keys/{email}/{key_id}` | DELETE | Revoke a specific API key |
| `/api/user/keys/{email}` | DELETE | Revoke all API keys for a user |
| `/api/key/usage/{email}/{key_id}` | GET | Get usage statistics for a specific API key |
| `/api/key/validate` | POST | Validate an API key |
| `/api/key/usage` | POST | Record API key usage |

## Credit Balance Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/user/credits/{email}` | GET | Get credit balance for a user |
| `/api/user/credits` | POST | Update credit balance for a user |

## System Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/health` | GET | Health check endpoint |

## External Services

The backend connects to a YAML formatter service:
- **Formatter Service**: `http://localhost:6001/format-yaml`

## gRPC Services

The backend also exposes the following gRPC services:

| Service | Method | Description |
|---------|--------|-------------|
| `endpoint.EndpointService` | `GetApiGroups` | Stream API groups for a user |
| `endpoint.EndpointService` | `UploadApiGroups` | Upload API groups configuration |
| `endpoint.EndpointService` | `GetUserPreferences` | Get user preferences |
| `endpoint.EndpointService` | `UpdateUserPreferences` | Update user preferences |
| `endpoint.EndpointService` | `ResetUserPreferences` | Reset user preferences |

## Detailed Endpoint Specifications

### API Groups Endpoints

#### GET `/api/groups/{email}`
- **Purpose**: Retrieves all API groups for a user with preferences applied
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: JSON containing API groups and their endpoints

#### POST `/api/upload`
- **Purpose**: Upload an API groups configuration file
- **Request Body**: 
  - `email`: User's email address 
  - `file_content`: Base64-encoded file content
  - `file_name`: Name of the file
- **Returns**: Success status and count of imported groups/endpoints

#### POST `/api/group`
- **Purpose**: Add a new API group
- **Request Body**: 
  - `email`: User's email address
  - `api_group`: Group object with endpoints
- **Returns**: Success status and group ID

#### PUT `/api/group`
- **Purpose**: Update an existing API group
- **Request Body**: 
  - `email`: User's email address
  - `group_id`: ID of the group to update
  - `api_group`: Updated group object with endpoints
- **Returns**: Success status and group ID

#### DELETE `/api/groups/{email}/{group_id}`
- **Purpose**: Delete an API group
- **URL Parameters**: 
  - `email`: User's email address
  - `group_id`: ID of the group to delete
- **Returns**: Success status

### User Preferences Endpoints

#### GET `/api/user/preferences/{email}`
- **Purpose**: Get user preferences
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: User preferences including hidden default endpoints

#### POST `/api/user/preferences`
- **Purpose**: Update user preferences
- **Request Body**: 
  - `email`: User's email address
  - `action`: Action to perform ("hide_default" or "show_default")
  - `endpoint_id`: ID of the endpoint to hide/show
- **Returns**: Success status

#### DELETE `/api/user/preferences/{email}`
- **Purpose**: Reset user preferences
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: Success status

### API Key Management Endpoints

#### GET `/api/user/keys/{email}`
- **Purpose**: Get status of all API keys for a user
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: List of API keys with usage statistics

#### POST `/api/user/keys`
- **Purpose**: Generate a new API key for a user
- **Request Body**: 
  - `email`: User's email address
  - `key_name`: Name for the API key
- **Returns**: The generated API key (shown only once)

#### DELETE `/api/user/keys/{email}/{key_id}`
- **Purpose**: Revoke a specific API key
- **URL Parameters**: 
  - `email`: User's email address
  - `key_id`: ID of the key to revoke
- **Returns**: Success status

#### DELETE `/api/user/keys/{email}`
- **Purpose**: Revoke all API keys for a user
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: Success status and count of revoked keys

#### GET `/api/key/usage/{email}/{key_id}`
- **Purpose**: Get usage statistics for a specific API key
- **URL Parameters**: 
  - `email`: User's email address
  - `key_id`: ID of the key
- **Returns**: Key usage statistics

#### POST `/api/key/validate`
- **Purpose**: Validate an API key
- **Request Body**: 
  - `api_key`: API key to validate
- **Returns**: Validation status and key owner email

#### POST `/api/key/usage`
- **Purpose**: Record API key usage
- **Request Body**: 
  - `key_id`: ID of the key to record usage for
- **Returns**: Success status

### Credit Balance Endpoints

#### GET `/api/user/credits/{email}`
- **Purpose**: Get credit balance for a user
- **URL Parameters**: 
  - `email`: User's email address
- **Returns**: Credit balance

#### POST `/api/user/credits`
- **Purpose**: Update credit balance for a user
- **Request Body**: 
  - `email`: User's email address
  - `amount`: Amount to add to balance (can be negative)
- **Returns**: New balance

### System Endpoints

#### GET `/api/health`
- **Purpose**: Health check endpoint
- **Returns**: Service status information
````

## File: config-samples/config.yaml
````yaml
# Sensei Store Configuration

# Service settings
service:
  name: store
  version: 1.0.0
  port: 50055
  host: 0.0.0.0
  workers: 4

database:
  path: ./data/sensei.db
  cache_size_mb: 100
  memory_limit_mb: 500

# Metrics and monitoring
metrics:
  enabled: true
  path: /metrics
  collect_interval_ms: 10000

# Security settings
security:
  tls:
    enabled: true
    cert_path: /etc/sensei/certs/store.crt
    key_path: /etc/sensei/certs/store.key
  cors:
    allowed_origins:
      - https://app.apisensei.ai
      - https://apisensei.ai
    allowed_methods:
      - GET
      - POST
      - PUT
      - DELETE
    allowed_headers:
      - Authorization
      - Content-Type
    max_age_secs: 86400

# Tracing configuration
tracing:
  level: info
  file_path: /var/log/sensei/store.log
  rotation:
    size_mb: 100
    age_days: 7
    files_kept: 10
````

## File: src/db_pool.rs
````rust
use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use std::error::Error;
use tokio_postgres::NoTls;

pub type PgPool = Pool;
pub type PgConnection = deadpool_postgres::Object;

pub fn create_pg_pool(database_url: &str) -> Result<PgPool, Box<dyn Error + Send + Sync>> {
    let mut cfg = Config::new();

    // Parse the PostgreSQL URL manually
    cfg.url = Some(database_url.to_string());

    cfg.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    });

    let pool = cfg
        .create_pool(Some(Runtime::Tokio1), NoTls)
        .map_err(|e| format!("Failed to create pool: {}", e))?;

    Ok(pool)
}
````

## File: src/models.rs
````rust
use crate::endpoint_store::ApiGroupWithEndpoints;
use serde::{Deserialize, Serialize};

use crate::endpoint_store::Endpoint;
// Request and Response models for API key validation
#[derive(Debug, Deserialize)]
pub struct ValidateKeyRequest {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManageEndpointRequest {
    pub email: String,
    pub group_id: String,
    pub endpoint: Endpoint,
}

#[derive(Debug, Serialize)]
pub struct ValidateKeyResponse {
    pub valid: bool,
    pub email: Option<String>,
    pub key_id: Option<String>,
    pub message: String,
}

// Response model for API key usage
#[derive(Debug, Serialize)]
pub struct RecordUsageResponse {
    pub success: bool,
    pub message: String,
}

// Request and Response models
#[derive(Debug, Clone, Deserialize)]
pub struct UploadRequest {
    pub email: String,
    pub file_name: String,
    pub file_content: String, // Base64 encoded
}

#[derive(Debug, Serialize)]
pub struct UploadResponse {
    pub success: bool,
    pub message: String,
    pub imported_count: i32,
    pub group_count: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AddApiGroupRequest {
    pub email: String,
    pub api_group: ApiGroupWithEndpoints,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateApiGroupRequest {
    pub email: String,
    pub group_id: String,
    pub api_group: ApiGroupWithEndpoints,
}

// Handler for recording API key usage
#[derive(Debug, Deserialize)]
pub struct RecordUsageRequest {
    pub key_id: String,
}
````

## File: src/upload_api_config.rs
````rust
use crate::endpoint_store::{generate_id_from_text, ApiGroupWithEndpoints, ApiStorage};
use crate::models::UploadRequest;
use crate::{endpoint_store::EndpointStore, formatter::YamlFormatter, models::UploadResponse};
use actix_web::{web, HttpResponse, Responder};
use base64::{engine::general_purpose, Engine};
use std::sync::Arc;

/// Detect if content is base64 encoded or plain text
fn is_base64_content(content: &str) -> bool {
    // Check if content looks like base64
    if content.is_empty() {
        return false;
    }

    // Base64 content should not contain typical YAML/JSON keywords at the start
    let trimmed = content.trim();
    if trimmed.starts_with("api_groups:")
        || trimmed.starts_with("{")
        || trimmed.starts_with("endpoints:")
    {
        return false;
    }

    // Check if all characters are valid base64 characters
    let cleaned: String = content.chars().filter(|c| !c.is_whitespace()).collect();

    // Base64 should have reasonable length and valid characters
    if cleaned.len() < 10 {
        return false;
    }

    cleaned
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
}

/// Clean and normalize base64 content
fn clean_base64_content(content: &str) -> String {
    content
        .chars()
        .filter(|c| !c.is_whitespace())
        .filter(|&c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        .collect()
}

/// Decode content - handles both base64 and plain text
fn decode_content(content: &str) -> Result<Vec<u8>, String> {
    if is_base64_content(content) {
        // Content appears to be base64 - try to decode it
        let cleaned_content = clean_base64_content(content);

        // Try multiple decoding strategies
        if let Ok(bytes) = general_purpose::STANDARD.decode(&cleaned_content) {
            return Ok(bytes);
        }

        if let Ok(bytes) = general_purpose::URL_SAFE.decode(&cleaned_content) {
            return Ok(bytes);
        }

        if let Ok(bytes) = general_purpose::URL_SAFE_NO_PAD.decode(&cleaned_content) {
            return Ok(bytes);
        }

        // Try adding padding
        let mut padded_content = cleaned_content.clone();
        while padded_content.len() % 4 != 0 {
            padded_content.push('=');
        }

        if let Ok(bytes) = general_purpose::STANDARD.decode(&padded_content) {
            return Ok(bytes);
        }

        Err("Failed to decode base64 content with all strategies".to_string())
    } else {
        // Content appears to be plain text - return as UTF-8 bytes
        Ok(content.as_bytes().to_vec())
    }
}

// Handler for uploading API configuration
pub async fn upload_api_config(
    store: web::Data<Arc<EndpointStore>>,
    formatter: web::Data<Arc<YamlFormatter>>,
    upload_data: web::Json<UploadRequest>,
) -> impl Responder {
    let is_base64 = is_base64_content(&upload_data.file_content);

    tracing::info!(
        email = %upload_data.email,
        filename = %upload_data.file_name,
        original_content_length = upload_data.file_content.len(),
        detected_format = if is_base64 { "base64" } else { "plain_text" },
        "Received HTTP upload request via Actix"
    );

    // Decode content (base64 or plain text)
    let file_bytes = match decode_content(&upload_data.file_content) {
        Ok(bytes) => {
            tracing::info!(
                decoded_size = bytes.len(),
                format_detected = if is_base64 { "base64" } else { "plain_text" },
                "Successfully processed file content"
            );
            bytes
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                content_sample = %upload_data.file_content.chars().take(100).collect::<String>(),
                "Failed to process file content"
            );
            return HttpResponse::BadRequest().json(UploadResponse {
                success: false,
                message: format!("Invalid file content: {}", e),
                imported_count: 0,
                group_count: 0,
            });
        }
    };

    // Convert to UTF-8 string
    let file_content = match String::from_utf8(file_bytes.clone()) {
        Ok(content) => content,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "File content is not valid UTF-8, attempting lossy conversion"
            );

            let lossy_content = String::from_utf8_lossy(&file_bytes);
            if lossy_content.trim().is_empty() {
                return HttpResponse::BadRequest().json(UploadResponse {
                    success: false,
                    message: "File content is empty or not valid text".to_string(),
                    imported_count: 0,
                    group_count: 0,
                });
            }

            tracing::info!("Using lossy UTF-8 conversion");
            lossy_content.to_string()
        }
    };

    // Format the content if it's YAML and formatter is available
    let processed_content =
        if upload_data.file_name.ends_with(".yaml") || upload_data.file_name.ends_with(".yml") {
            match formatter
                .format_yaml(file_content.as_bytes(), &upload_data.file_name)
                .await
            {
                Ok(formatted) => match String::from_utf8(formatted) {
                    Ok(content) => {
                        tracing::info!("Successfully formatted YAML content");
                        content
                    }
                    Err(_) => {
                        tracing::warn!("Formatted content is not valid UTF-8, using original");
                        file_content
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "Failed to format YAML, proceeding with original content"
                    );
                    file_content
                }
            }
        } else if upload_data.file_name.ends_with(".json") {
            // Pretty print JSON if possible
            match serde_json::from_str::<serde_json::Value>(&file_content) {
                Ok(json_value) => serde_json::to_string_pretty(&json_value).unwrap_or(file_content),
                Err(_) => file_content,
            }
        } else {
            file_content
        };

    // Parse the content based on file extension
    let api_storage =
        if upload_data.file_name.ends_with(".yaml") || upload_data.file_name.ends_with(".yml") {
            match serde_yaml::from_str::<ApiStorage>(&processed_content) {
                Ok(storage) => storage,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        content_preview = %processed_content.chars().take(200).collect::<String>(),
                        "Failed to parse YAML content"
                    );
                    return HttpResponse::BadRequest().json(UploadResponse {
                        success: false,
                        message: format!("Invalid YAML format: {}", e),
                        imported_count: 0,
                        group_count: 0,
                    });
                }
            }
        } else if upload_data.file_name.ends_with(".json") {
            match serde_json::from_str::<ApiStorage>(&processed_content) {
                Ok(storage) => storage,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        content_preview = %processed_content.chars().take(200).collect::<String>(),
                        "Failed to parse JSON content"
                    );
                    return HttpResponse::BadRequest().json(UploadResponse {
                        success: false,
                        message: format!("Invalid JSON format: {}", e),
                        imported_count: 0,
                        group_count: 0,
                    });
                }
            }
        } else {
            return HttpResponse::BadRequest().json(UploadResponse {
            success: false,
            message:
                "Unsupported file format. Please upload YAML (.yaml/.yml) or JSON (.json) files."
                    .to_string(),
            imported_count: 0,
            group_count: 0,
        });
        };

    // Validate and process API groups
    let group_count = api_storage.api_groups.len();
    if group_count == 0 {
        return HttpResponse::BadRequest().json(UploadResponse {
            success: false,
            message: "No API groups found in the file".to_string(),
            imported_count: 0,
            group_count: 0,
        });
    }

    // After parsing, before processing groups
    // for group in &api_storage.api_groups {
    //     if group.group.base.trim().is_empty() {
    //         return HttpResponse::BadRequest().json(UploadResponse {
    //             success: false,
    //             message: format!("API group '{}' must have a base URL", group.group.name),
    //             imported_count: 0,
    //             group_count: 0,
    //         });
    //     }
    //
    //     for endpoint in &group.endpoints {
    //         if endpoint.base.trim().is_empty() {
    //             return HttpResponse::BadRequest().json(UploadResponse {
    //                 success: false,
    //                 message: format!("Endpoint '{}' must have a base URL", endpoint.text),
    //                 imported_count: 0,
    //                 group_count: 0,
    //             });
    //         }
    //     }
    // }

    // Process groups and endpoints
    let mut processed_groups = Vec::new();
    for mut group in api_storage.api_groups {
        // Generate ID for group if missing
        if group.group.id.trim().is_empty() {
            group.group.id = generate_id_from_text(&group.group.name);
        }

        // Provide default base URL if empty
        if group.group.base.trim().is_empty() {
            group.group.base = "https://api.example.com".to_string();
        }

        // Process endpoints
        let mut processed_endpoints = Vec::new();
        for mut endpoint in group.endpoints {
            // Generate ID for endpoint if missing
            if endpoint.id.trim().is_empty() {
                endpoint.id = generate_id_from_text(&endpoint.text);
            }

            // Inherit from group or provide default if empty
            if endpoint.base.trim().is_empty() {
                endpoint.base = group.group.base.clone();
            }

            endpoint.group_id = group.group.id.clone();
            processed_endpoints.push(endpoint);
        }

        processed_groups.push(ApiGroupWithEndpoints {
            group: group.group,
            endpoints: processed_endpoints,
        });
    }

    // Save to database
    match store
        .replace_user_api_groups(&upload_data.email, processed_groups)
        .await
    {
        Ok(endpoint_count) => {
            tracing::info!(
                email = %upload_data.email,
                endpoint_count = endpoint_count,
                group_count = group_count,
                "Successfully imported API groups and endpoints"
            );
            HttpResponse::Ok().json(UploadResponse {
                success: true,
                message: "API groups and endpoints successfully imported".to_string(),
                imported_count: endpoint_count as i32,
                group_count: group_count as i32,
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %upload_data.email,
                "Failed to import API groups"
            );
            HttpResponse::InternalServerError().json(UploadResponse {
                success: false,
                message: format!("Failed to import API groups: {}", e),
                imported_count: 0,
                group_count: 0,
            })
        }
    }
}
````

## File: src/endpoint_store/db_helpers.rs
````rust
use crate::endpoint_store::StoreError;

pub trait ResultExt<T> {
    fn to_store_error(self) -> Result<T, StoreError>;
}

impl<T> ResultExt<T> for Result<T, tokio_postgres::Error> {
    fn to_store_error(self) -> Result<T, StoreError> {
        self.map_err(|e| StoreError::Database(e.to_string()))
    }
}

impl<T> ResultExt<T> for Result<T, deadpool_postgres::PoolError> {
    fn to_store_error(self) -> Result<T, StoreError> {
        self.map_err(|e| StoreError::Pool(format!("Database pool error: {:?}", e)))
    }
}
````

## File: src/endpoint_store/errors.rs
````rust
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(String),
    #[error("Connection pool error: {0}")]
    Pool(String),
}

impl From<tokio_postgres::Error> for StoreError {
    fn from(err: tokio_postgres::Error) -> Self {
        StoreError::Database(err.to_string())
    }
}

impl From<deadpool_postgres::PoolError> for StoreError {
    fn from(err: deadpool_postgres::PoolError) -> Self {
        StoreError::Pool(format!("Failed to get connection from pool: {:?}", err))
    }
}
````

## File: src/endpoint_store/get_create_user_api_groups.rs
````rust
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::generate_id_from_text;
use crate::endpoint_store::{ApiGroup, ApiGroupWithEndpoints, Endpoint, EndpointStore, StoreError};

/// Gets or creates API groups for a user
pub async fn get_or_create_user_api_groups(
    store: &EndpointStore,
    email: &str,
) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
    // Check if user already has API groups
    let existing_groups = store.get_api_groups_by_email(email).await?;
    if !existing_groups.is_empty() {
        tracing::info!(
            email = %email,
            group_count = existing_groups.len(),
            "User already has API groups"
        );
        return Ok(existing_groups);
    }

    // Create a default group for new users if they don't have any
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    tracing::info!(email = %email, "User has no API groups, creating a default one");

    // Create a basic default group
    let default_group_id = generate_id_from_text("Default API");
    let default_group = ApiGroup {
        id: default_group_id.clone(),
        name: "Default API".to_string(),
        description: "Your default API group".to_string(),
        base: "https://api.example.com".to_string(),
    };

    // Create a sample endpoint for the default group
    let sample_endpoint = Endpoint {
        id: generate_id_from_text("sample-endpoint"),
        text: "Sample endpoint".to_string(),
        description: "A sample endpoint to get you started".to_string(),
        verb: "GET".to_string(),
        base: "https://api.example.com".to_string(),
        path: "/sample".to_string(),
        group_id: default_group_id.clone(),
        parameters: vec![],
    };

    // Insert the default group
    tx.execute(
        "INSERT INTO api_groups (id, name, description, base) VALUES ($1, $2, $3, $4)",
        &[
            &default_group.id,
            &default_group.name,
            &default_group.description,
            &default_group.base,
        ],
    )
    .await
    .to_store_error()?;

    // Insert the sample endpoint
    tx.execute(
        "INSERT INTO endpoints (id, text, description, verb, base, path, group_id) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &sample_endpoint.id,
            &sample_endpoint.text,
            &sample_endpoint.description,
            &sample_endpoint.verb,
            &sample_endpoint.base,
            &sample_endpoint.path,
            &sample_endpoint.group_id,
        ],
    )
    .await
    .to_store_error()?;

    // Associate group with user
    tx.execute(
        "INSERT INTO user_groups (email, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        &[&email, &default_group_id],
    )
    .await
    .to_store_error()?;

    // Associate endpoint with user
    tx.execute(
        "INSERT INTO user_endpoints (email, endpoint_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        &[&email, &sample_endpoint.id],
    )
    .await
    .to_store_error()?;

    tx.commit().await.to_store_error()?;

    // Create the result
    let default_api_group = ApiGroupWithEndpoints {
        group: default_group,
        endpoints: vec![sample_endpoint],
    };

    tracing::info!(
        email = %email,
        "Created default API group for new user"
    );

    Ok(vec![default_api_group])
}
````

## File: src/endpoint_store/replace_user_api_groups.rs
````rust
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{
    generate_id_from_text, ApiGroupWithEndpoints, EndpointStore, StoreError,
};

/// Replaces all API groups and endpoints for a user
pub async fn replace_user_api_groups(
    store: &EndpointStore,
    email: &str,
    api_groups: Vec<ApiGroupWithEndpoints>,
) -> Result<usize, StoreError> {
    tracing::info!(email = %email, "Starting complete API group replacement");

    // Clean up existing user data
    match store.force_clean_user_data(email).await {
        Ok(_) => {
            tracing::info!(email = %email, "Successfully cleaned up user data");
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to clean up user data, will try fallback approach"
            );

            match store.fallback_clean_user_data(email).await {
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
    let mut imported_count = 0;
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    for group_with_endpoints in &api_groups {
        let group = &group_with_endpoints.group;

        // Generate ID if not provided
        let group_id = if group.id.is_empty() {
            generate_id_from_text(&group.name)
        } else {
            group.id.clone()
        };

        // Check if group exists
        let group_exists_row = tx
            .query_opt("SELECT 1 FROM api_groups WHERE id = $1", &[&group_id])
            .await
            .to_store_error()?;

        if group_exists_row.is_none() {
            tracing::debug!(group_id = %group_id, "Creating new API group");
            tx.execute(
                "INSERT INTO api_groups (id, name, description, base) VALUES ($1, $2, $3, $4)",
                &[&group_id, &group.name, &group.description, &group.base],
            )
            .await
            .to_store_error()?;
        } else {
            tracing::debug!(group_id = %group_id, "Updating existing API group");
            tx.execute(
                "UPDATE api_groups SET name = $1, description = $2, base = $3 WHERE id = $4",
                &[&group.name, &group.description, &group.base, &group_id],
            )
            .await
            .to_store_error()?;
        }

        // Link group to user
        tx.execute(
            "INSERT INTO user_groups (email, group_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            &[&email, &group_id],
        )
        .await
        .to_store_error()?;

        // Process endpoints for this group
        for endpoint in &group_with_endpoints.endpoints {
            let endpoint_id = if endpoint.id.is_empty() {
                generate_id_from_text(&endpoint.text)
            } else {
                endpoint.id.clone()
            };

            let endpoint_exists_row = tx
                .query_opt("SELECT 1 FROM endpoints WHERE id = $1", &[&endpoint_id])
                .await
                .to_store_error()?;

            if endpoint_exists_row.is_none() {
                tracing::debug!(endpoint_id = %endpoint_id, "Creating new endpoint");
                tx.execute(
                    "INSERT INTO endpoints (id, text, description, verb, base, path, group_id) 
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                    &[
                        &endpoint_id,
                        &endpoint.text,
                        &endpoint.description,
                        &endpoint.verb,
                        &endpoint.base,
                        &endpoint.path,
                        &group_id,
                    ],
                )
                .await
                .to_store_error()?;
            } else {
                tracing::debug!(endpoint_id = %endpoint_id, "Updating existing endpoint");
                tx.execute(
                    "UPDATE endpoints SET text = $1, description = $2, verb = $3, base = $4, path = $5, group_id = $6 WHERE id = $7",
                    &[
                        &endpoint.text,
                        &endpoint.description,
                        &endpoint.verb,
                        &endpoint.base,
                        &endpoint.path,
                        &group_id,
                        &endpoint_id,
                    ],
                )
                .await
                .to_store_error()?;
            }

            // Link endpoint to user
            tx.execute(
                "INSERT INTO user_endpoints (email, endpoint_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                &[&email, &endpoint_id],
            )
            .await
            .to_store_error()?;

            // Clean up existing parameters
            tx.execute(
                "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
                &[&endpoint_id],
            )
            .await
            .to_store_error()?;

            tx.execute(
                "DELETE FROM parameters WHERE endpoint_id = $1",
                &[&endpoint_id],
            )
            .await
            .to_store_error()?;

            // Add new parameters
            for param in &endpoint.parameters {
                let required = param.required.parse::<bool>().unwrap_or(false);

                tx.execute(
                    "INSERT INTO parameters (endpoint_id, name, description, required) 
                        VALUES ($1, $2, $3, $4)",
                    &[&endpoint_id, &param.name, &param.description, &required],
                )
                .await
                .to_store_error()?;

                for alt in &param.alternatives {
                    tx.execute(
                        "INSERT INTO parameter_alternatives (endpoint_id, parameter_name, alternative) 
                            VALUES ($1, $2, $3)",
                        &[&endpoint_id, &param.name, alt],
                    )
                    .await
                    .to_store_error()?;
                }
            }

            imported_count += 1;
        }
    }

    tracing::info!(
        email = %email,
        group_count = api_groups.len(),
        endpoint_count = imported_count,
        "Successfully imported API groups and endpoints"
    );

    tx.commit().await.to_store_error()?;
    Ok(imported_count)
}
````

## File: src/config.rs
````rust
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub grpc: GrpcServerConfig,
    pub http: HttpServerConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GrpcServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HttpServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    formatter_host: Option<String>,
    formatter_port: Option<u16>,
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let config: Config = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    pub fn grpc_address(&self) -> String {
        format!("{}:{}", self.server.grpc.host, self.server.grpc.port)
    }

    pub fn http_host(&self) -> &str {
        &self.server.http.host
    }

    pub fn http_port(&self) -> u16 {
        self.server.http.port
    }

    pub fn formatter_url(&self) -> String {
        let host = self.formatter_host.as_deref().unwrap_or("localhost");
        let port = self.formatter_port.unwrap_or(6001);
        format!("http://{}:{}/format-yaml", host, port)
    }
}

// Default implementation for testing or when config file is missing
impl Default for Config {
    fn default() -> Self {
        Self {
            // whoami: "api-store".to_string(),
            // output: "grpc".to_string(),
            // level: "debug".to_string(),
            server: ServerConfig {
                grpc: GrpcServerConfig {
                    host: "0.0.0.0".to_string(),
                    port: 50055,
                },
                http: HttpServerConfig {
                    host: "127.0.0.1".to_string(),
                    port: 5007,
                },
            },
            formatter_port: Some(6001),
            formatter_host: Some("localhost".to_string()),
        }
    }
}
````

## File: src/get_api_groups.rs
````rust
use crate::endpoint_store::EndpointStore;

use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

#[derive(serde::Serialize)]
pub struct EnhancedApiGroupsResponse {
    pub success: bool,
    pub api_groups: Vec<crate::endpoint_store::ApiGroupWithEndpoints>,
    pub message: String,
    // Remove API key fields since we won't auto-generate keys
    pub is_new_user: bool,
    pub credit_balance: i64,
}

pub async fn get_api_groups(
    store: web::Data<Arc<EndpointStore>>,
    email: web::Path<String>,
) -> impl Responder {
    let email = email.into_inner();
    tracing::info!(email = %email, "Received HTTP get API groups request");

    // Check if this is a new user by looking for existing API keys
    let (is_new_user, current_balance) = match store.get_api_keys_status(&email).await {
        Ok(status) => {
            tracing::info!(
                email = %email,
                has_keys = status.has_keys,
                active_key_count = status.active_key_count,
                current_balance = status.balance,
                "API key status check result"
            );
            (!status.has_keys, status.balance)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                email = %email,
                "Failed to check API key status, assuming new user"
            );
            (true, 0) // Assume new user if we can't check
        }
    };

    tracing::info!(
        email = %email,
        is_new_user = is_new_user,
        current_balance = current_balance,
        "Computed user status"
    );

    let mut response = EnhancedApiGroupsResponse {
        success: true,
        api_groups: vec![],
        message: "API groups successfully retrieved".to_string(),
        is_new_user,
        credit_balance: current_balance,
    };

    // Add default credit for new users (without creating an API key)
    if is_new_user && current_balance == 0 {
        tracing::info!(email = %email, "New user detected, adding $5 default credit");

        match store.update_credit_balance(&email, 500).await {
            Ok(new_balance) => {
                tracing::info!(
                    email = %email,
                    new_balance = new_balance,
                    "Added $5 default credit for new user"
                );
                response.credit_balance = new_balance;
                response.message = "Welcome! $5 credit has been added to your account. Create an API key to start using the service.".to_string();
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    "Failed to add default credit for new user"
                );
                response.message =
                    "Welcome! Please create an API key to start using the service.".to_string();
            }
        }
    }

    // Get API groups with preferences applied
    match store.get_api_groups_with_preferences(&email).await {
        Ok(api_groups) => {
            tracing::info!(
                email = %email,
                group_count = api_groups.len(),
                is_new_user = is_new_user,
                credit_balance = response.credit_balance,
                "Successfully retrieved API groups"
            );

            response.api_groups = api_groups;
            HttpResponse::Ok().json(response)
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %email,
                "Failed to retrieve API groups"
            );

            response.success = false;
            response.message = format!("Error: {}", e);
            HttpResponse::InternalServerError().json(response)
        }
    }
}
````

## File: README.md
````markdown
# API Store

A Rust service for managing API endpoints with parameter definitions, alternatives, and HTTP verbs.

## Overview

API Store is a gRPC service written in Rust that allows you to manage and query API endpoint definitions. It supports storing default endpoints and user-specific endpoint configurations.

## Features

- Store API endpoint definitions with parameters
- Support for HTTP verbs (GET, POST, PUT, DELETE, etc.)
- Parameter definitions with optional descriptions and alternatives
- User-specific endpoint configurations
- Default endpoints management with user preferences
- YAML and JSON import/export

## Data Structure

Each endpoint consists of:

- `id`: Unique identifier
- `text`: Display text or short description
- `description`: Optional longer description (defaults to empty string)
- `verb`: HTTP verb (GET, POST, PUT, DELETE, etc. - defaults to "GET")
- `parameters`: List of parameters (can be empty)

Each parameter consists of:

- `name`: Parameter name
- `description`: Optional parameter description (defaults to empty string)
- `required`: Whether the parameter is required (defaults to false)
- `alternatives`: List of alternative names for the parameter (defaults to empty list)

User preferences consist of:

- `email`: User's email address
- `hidden_defaults`: List of default endpoint IDs that the user has chosen to hide

## Configuration

Endpoints can be defined in YAML or JSON. Here's an example:

```yaml
endpoints:
  - id: "get_users"
    text: "Get users"
    # Description is optional
    verb: "GET"  # Optional, defaults to GET
    parameters:
      - name: "page"
        # No required field - defaults to false
        # No description - defaults to empty string
      - name: "limit"
        description: "Number of users per page"
      - name: "sort_by"
        description: "Field to sort by"
        alternatives:
          - "order_by"
          - "sort"

  - id: "create_user"
    text: "Create a new user"
    description: "Creates a new user in the system"
    verb: "POST"
    parameters:
      - name: "username"
        description: "User's username"
        required: true
        alternatives:
          - "user_name"
          - "login"
```

## Default Endpoints Management

API Store includes support for default endpoints that cannot be modified or deleted by users. Users can, however, choose to hide specific default endpoints from their view.

### User Preferences API

The following API endpoints are available for managing user preferences:

#### Get User Preferences

```
GET /api/user/preferences/:email
```

Returns user preferences including hidden default endpoints.

Response:
```json
{
  "success": true,
  "preferences": {
    "email": "user@example.com",
    "hidden_defaults": ["endpoint_id_1", "endpoint_id_2"]
  }
}
```

#### Update User Preferences

```
POST /api/user/preferences
```

Request body:
```json
{
  "email": "user@example.com",
  "action": "hide_default",  // or "show_default"
  "endpoint_id": "endpoint_id_1"
}
```

Response:
```json
{
  "success": true,
  "message": "User preferences successfully updated"
}
```

#### Reset User Preferences

```
DELETE /api/user/preferences/:email
```

Resets all user preferences to default.

Response:
```json
{
  "success": true,
  "message": "User preferences successfully reset"
}
```

## Usage

### Running the Server

```bash
cargo run
```

The server will start on port 50055 (gRPC) and 9090 (HTTP) by default.

### Testing

Use the provided test scripts in the `test` directory:

```bash
cd test
./query.sh                   # Fetch endpoints for a user
./upload.sh                  # Upload a new endpoints file
./test_user_preferences.sh   # Test user preferences functionality
```

## API

The service exposes both gRPC and HTTP endpoints:

### gRPC Endpoints:

1. `GetApiGroups`: Fetch endpoints for a user
2. `UploadApiGroups`: Upload a new endpoints configuration file
3. `GetUserPreferences`: Get user preferences
4. `UpdateUserPreferences`: Update user preferences
5. `ResetUserPreferences`: Reset user preferences

### HTTP Endpoints:

1. `GET /api/groups/:email`: Get API groups for a user
2. `POST /api/upload`: Upload a new endpoints configuration file
3. `POST /api/group`: Add a new API group
4. `PUT /api/group`: Update an API group
5. `DELETE /api/groups/:email/:group_id`: Delete an API group
6. `GET /api/user/preferences/:email`: Get user preferences
7. `POST /api/user/preferences`: Update user preferences
8. `DELETE /api/user/preferences/:email`: Reset user preferences


Convenient curl to add credit for test 

curl -X POST \
  -H "Content-Type: application/json" \
  -d '{"email":"mohamed.bennekrouf@gmail.com","amount":500}' \
  http://127.0.0.1:9090/api/user/credits

## License

MIT
````

## File: src/endpoint_store/add_user_api_group.rs
````rust
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{ApiGroupWithEndpoints, EndpointStore, StoreError};

/// Adds a single API group for a user
pub async fn add_user_api_group(
    store: &EndpointStore,
    email: &str,
    api_group: &ApiGroupWithEndpoints,
) -> Result<usize, StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let group = &api_group.group;
    let group_id = &group.id;

    tracing::info!(
        email = %email,
        group_id = %group_id,
        "Adding API group"
    );

    // Check if group exists
    let group_exists_row = tx
        .query_opt("SELECT 1 FROM api_groups WHERE id = $1", &[group_id])
        .await
        .to_store_error()?;

    if group_exists_row.is_none() {
        tx.execute(
            "INSERT INTO api_groups (id, name, description, base) VALUES ($1, $2, $3, $4)",
            &[group_id, &group.name, &group.description, &group.base],
        )
        .await
        .to_store_error()?;
    } else {
        tx.execute(
            "UPDATE api_groups SET name = $1, description = $2, base = $3 WHERE id = $4",
            &[&group.name, &group.description, &group.base, group_id],
        )
        .await
        .to_store_error()?;
    }

    // Associate group with user
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
                "INSERT INTO endpoints (id, text, description, verb, base, path, group_id) 
                VALUES ($1, $2, $3, $4, $5, $6, $7)",
                &[
                    &endpoint.id,
                    &endpoint.text,
                    &endpoint.description,
                    &endpoint.verb,
                    &endpoint.base,
                    &endpoint.path,
                    group_id,
                ],
            )
            .await
            .to_store_error()?;
        } else {
            tx.execute(
                "UPDATE endpoints SET text = $1, description = $2, verb = $3, base = $4, path = $5, group_id = $6 WHERE id = $7",
                &[
                    &endpoint.text,
                    &endpoint.description,
                    &endpoint.verb,
                    &endpoint.base,
                    &endpoint.path,
                    group_id,
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

    tracing::info!(
        email = %email,
        group_id = %group_id,
        endpoint_count = endpoint_count,
        "API group successfully added"
    );

    tx.commit().await.to_store_error()?;
    Ok(endpoint_count)
}
````

## File: src/endpoint_store/cleanup.rs
````rust
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::EndpointStore;
use crate::endpoint_store::StoreError;

/// Cleans up user data in a more conservative way (fallback)
pub async fn fallback_clean_user_data(
    store: &EndpointStore,
    email: &str,
) -> Result<(), StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    // Get endpoint IDs
    let rows = tx
        .query(
            "SELECT e.id 
            FROM endpoints e
            JOIN user_endpoints ue ON e.id = ue.endpoint_id
            WHERE ue.email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    let endpoint_ids: Vec<String> = rows.iter().map(|row| row.get(0)).collect();

    // Remove user-endpoint associations
    for id in &endpoint_ids {
        tx.execute(
            "DELETE FROM user_endpoints WHERE email = $1 AND endpoint_id = $2",
            &[&email, id],
        )
        .await
        .to_store_error()?;
    }

    // Check and clean up unused endpoints
    for id in &endpoint_ids {
        let still_used_row = tx
            .query_opt("SELECT 1 FROM user_endpoints WHERE endpoint_id = $1", &[id])
            .await
            .to_store_error()?;

        if still_used_row.is_none() {
            tx.execute(
                "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
                &[id],
            )
            .await
            .to_store_error()?;

            tx.execute("DELETE FROM parameters WHERE endpoint_id = $1", &[id])
                .await
                .to_store_error()?;

            tx.execute("DELETE FROM endpoints WHERE id = $1", &[id])
                .await
                .to_store_error()?;
        }
    }

    tx.commit().await.to_store_error()?;
    Ok(())
}

/// Forces a clean of user data
pub async fn force_clean_user_data(store: &EndpointStore, email: &str) -> Result<(), StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    // Get user's custom endpoints
    let rows = tx
        .query(
            "SELECT e.id 
            FROM endpoints e
            JOIN user_endpoints ue ON e.id = ue.endpoint_id
            WHERE ue.email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    let endpoint_ids: Vec<String> = rows.iter().map(|row| row.get(0)).collect();

    // Delete parameter alternatives
    for id in &endpoint_ids {
        tx.execute(
            "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
            &[id],
        )
        .await
        .to_store_error()?;
    }

    // Delete parameters
    for id in &endpoint_ids {
        tx.execute("DELETE FROM parameters WHERE endpoint_id = $1", &[id])
            .await
            .to_store_error()?;
    }

    // Delete user endpoint associations
    tx.execute("DELETE FROM user_endpoints WHERE email = $1", &[&email])
        .await
        .to_store_error()?;

    // Delete endpoints that are no longer referenced
    for id in &endpoint_ids {
        let still_referenced = tx
            .query_opt("SELECT 1 FROM user_endpoints WHERE endpoint_id = $1", &[id])
            .await
            .to_store_error()?;

        if still_referenced.is_none() {
            tx.execute("DELETE FROM endpoints WHERE id = $1", &[id])
                .await
                .to_store_error()?;
        }
    }

    tx.commit().await.to_store_error()?;
    Ok(())
}
````

## File: src/endpoint_store/delete_user_api_group.rs
````rust
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::{EndpointStore, StoreError};

/// Deletes an API group and all its endpoints for a user
pub async fn delete_user_api_group(
    store: &EndpointStore,
    email: &str,
    group_id: &str,
) -> Result<bool, StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    tracing::info!(
        email = %email,
        group_id = %group_id,
        "Deleting API group"
    );

    // Get all endpoint IDs for this group
    let rows = tx
        .query(
            "SELECT e.id 
            FROM endpoints e
            INNER JOIN user_endpoints ue ON e.id = ue.endpoint_id
            WHERE ue.email = $1 AND e.group_id = $2",
            &[&email, &group_id],
        )
        .await
        .to_store_error()?;

    let endpoint_ids: Vec<String> = rows.iter().map(|row| row.get(0)).collect();

    // Remove user-group association
    tx.execute(
        "DELETE FROM user_groups WHERE email = $1 AND group_id = $2",
        &[&email, &group_id],
    )
    .await
    .to_store_error()?;

    // Remove user-endpoint associations
    for endpoint_id in &endpoint_ids {
        tx.execute(
            "DELETE FROM user_endpoints WHERE email = $1 AND endpoint_id = $2",
            &[&email, endpoint_id],
        )
        .await
        .to_store_error()?;
    }

    // Check if the group is still associated with any user
    let group_still_used_row = tx
        .query_opt(
            "SELECT 1 FROM user_groups WHERE group_id = $1",
            &[&group_id],
        )
        .await
        .to_store_error()?;

    // If no user is using this group anymore, delete it and its endpoints
    if group_still_used_row.is_none() {
        for endpoint_id in &endpoint_ids {
            let endpoint_still_used_row = tx
                .query_opt(
                    "SELECT 1 FROM user_endpoints WHERE endpoint_id = $1",
                    &[endpoint_id],
                )
                .await
                .to_store_error()?;

            if endpoint_still_used_row.is_none() {
                // Delete parameter alternatives
                tx.execute(
                    "DELETE FROM parameter_alternatives WHERE endpoint_id = $1",
                    &[endpoint_id],
                )
                .await
                .to_store_error()?;

                // Delete parameters
                tx.execute(
                    "DELETE FROM parameters WHERE endpoint_id = $1",
                    &[endpoint_id],
                )
                .await
                .to_store_error()?;

                // Delete endpoint
                tx.execute("DELETE FROM endpoints WHERE id = $1", &[endpoint_id])
                    .await
                    .to_store_error()?;
            }
        }

        // Delete the group itself
        tx.execute("DELETE FROM api_groups WHERE id = $1", &[&group_id])
            .await
            .to_store_error()?;
    }

    tracing::info!(
        email = %email,
        group_id = %group_id,
        endpoint_count = endpoint_ids.len(),
        "API group successfully deleted"
    );

    tx.commit().await.to_store_error()?;
    Ok(true)
}
````

## File: endpoints.yaml
````yaml
api_groups:
  - name: "Email Service"
    description: "APIs for sending and managing email communications"
    base: "https://api.example.com"
    endpoints:
      - text: "Send email"
        description: "Send an email with possible attachments"
        verb: "POST"
        base: "https://api.example.com"  # Add this
        path: "/messaging/emails"
        parameters:
          - name: "to"
            description: "Recipient's email address"
            required: "true"
            alternatives:
              - "recipient_email"
              - "email_to"
          - name: "subject"
            description: "Email subject"
            required: "true"
            alternatives:
              - "email_title"
              - "mail_subject"
          - name: "body"
            description: "Email content"
            required: "true"
            alternatives:
              - "content"
              - "message"
              
      - text: "Get email status"
        description: "Check the delivery status of an email"
        verb: "GET"
        base: "https://api.example.com"  # Add this
        path: "/messaging/emails/{id}/status"
        parameters:
          - name: "id"
            description: "Email identifier"
            required: "true"
            alternatives:
              - "email_id"
              - "message_id"
  
  - name: "User Management"
    description: "APIs for managing user accounts"
    base: "https://api.example.com"
    endpoints:
      - text: "Create user"
        description: "Register a new user account"
        verb: "POST"
        base: "https://api.example.com"  # Add this
        path: "/users"
        parameters:
          - name: "username"
            description: "User's login name"
            required: "true"
            alternatives:
              - "login"
              - "user_name"
          - name: "email"
            description: "User's email address"
            required: "true"
            alternatives: []
          - name: "password"
            description: "User's password"
            required: "true"
            alternatives: []
      
      - text: "Delete user"
        description: "Remove a user account"
        verb: "DELETE"
        base: "https://api.example.com"  # Add this
        path: "/users/{id}"
        parameters:
          - name: "id"
            description: "User identifier"
            required: "true"
            alternatives:
              - "user_id"
````

## File: src/grpc_server.rs
````rust
use crate::endpoint::endpoint_service_server::EndpointService;

use crate::endpoint::{
    ApiGroup as ProtoApiGroup, Endpoint as ProtoEndpoint, GetApiGroupsRequest,
    GetApiGroupsResponse, GetUserPreferencesRequest, GetUserPreferencesResponse,
    Parameter as ProtoParameter, ResetUserPreferencesRequest, ResetUserPreferencesResponse,
    UpdateUserPreferencesRequest, UpdateUserPreferencesResponse, UploadApiGroupsRequest,
    UploadApiGroupsResponse, UserPreferences as ProtoUserPreferences,
};
use crate::formatter::YamlFormatter;

use crate::endpoint_store::{
    generate_id_from_text, ApiGroup, ApiGroupWithEndpoints, ApiStorage, EndpointStore,
};
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

#[derive(Clone)]
pub struct EndpointServiceImpl {
    store: Arc<EndpointStore>,
    formatter: Arc<YamlFormatter>,
}

impl EndpointServiceImpl {
    pub fn new(store: Arc<EndpointStore>, formatter_url: &str) -> Self {
        Self {
            store,
            formatter: Arc::new(YamlFormatter::new(formatter_url)),
        }
    }
}

#[tonic::async_trait]
impl EndpointService for EndpointServiceImpl {
    type GetApiGroupsStream =
        Pin<Box<dyn Stream<Item = Result<GetApiGroupsResponse, Status>> + Send + 'static>>;

    async fn get_api_groups(
        &self,
        request: Request<GetApiGroupsRequest>,
    ) -> Result<Response<Self::GetApiGroupsStream>, Status> {
        let email = request.into_inner().email;
        tracing::info!(email = %email, "Received get_api_groups request");

        // Clone necessary data for the stream
        let store = self.store.clone();

        // Create the stream
        let stream = async_stream::try_stream! {
            // Get API groups and endpoints
            let api_groups = match store.get_or_create_user_api_groups(&email).await {
                Ok(groups) => groups,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to get API groups from store");
                    // Yield an empty response instead of returning an error
                    yield GetApiGroupsResponse { api_groups: vec![] };
                    return;
                }
            };

            const BATCH_SIZE: usize = 5; // Process 5 groups at a time
            let mut current_batch = Vec::with_capacity(BATCH_SIZE);

            tracing::info!("Starting API group transformation and streaming");

            for api_group_with_endpoints in api_groups {
                let group = api_group_with_endpoints.group;
                let endpoints = api_group_with_endpoints.endpoints;

                tracing::debug!(
                    group_id = %group.id,
                    group_name = %group.name,
                    endpoint_count = endpoints.len(),
                    "Transforming API group"
                );

                // Transform endpoints to proto format
                let proto_endpoints: Vec<ProtoEndpoint> = endpoints
                .into_iter()
                .map(|e| ProtoEndpoint {
                    id: e.id,
                    text: e.text,
                    description: e.description,
                    verb: e.verb,
                    base: e.base,
                    path: e.path,
                    group_id: e.group_id,
                    parameters: e.parameters
                        .into_iter()
                        .map(|p| ProtoParameter {
                            name: p.name,
                            description: p.description,
                            required: p.required,
                            alternatives: p.alternatives,
                        })
                        .collect(),
                })
                .collect();

                // Create the proto API group
                let proto_group = ProtoApiGroup {
                    id: group.id,
                    name: group.name,
                    description: group.description,
                    base: group.base,
                    endpoints: proto_endpoints,
                };

                current_batch.push(proto_group);

                // When batch is full, yield it
                if current_batch.len() >= BATCH_SIZE {
                    tracing::info!(
                        batch_size = current_batch.len(),
                        "Sending batch of API groups"
                    );

                    yield GetApiGroupsResponse {
                        api_groups: std::mem::take(&mut current_batch),
                    };
                }
            }

            // Send any remaining API groups
            if !current_batch.is_empty() {
                tracing::info!(
                    batch_size = current_batch.len(),
                    "Sending final batch of API groups"
                );

                yield GetApiGroupsResponse {
                    api_groups: current_batch,
                };
            }

            tracing::info!("Finished streaming all API groups");
        };

        Ok(Response::new(Box::pin(stream)))
    }

    async fn upload_api_groups(
        &self,
        request: Request<UploadApiGroupsRequest>,
    ) -> Result<Response<UploadApiGroupsResponse>, Status> {
        let req = request.into_inner();
        let email = req.email;
        let file_content = req.file_content.clone();
        let file_name = req.file_name.clone();

        tracing::info!(
            email = %email,
            filename = %req.file_name,
            "Processing API group upload request"
        );

        // Format YAML content if needed
        let file_content = if file_name.ends_with(".yaml") || file_name.ends_with(".yml") {
            match self.formatter.format_yaml(&file_content, &file_name).await {
                Ok(formatted) => formatted,
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        email = %email,
                        "Failed to format YAML, proceeding with original content"
                    );
                    file_content
                }
            }
        } else {
            file_content
        };

        // Convert to string
        let file_content = match String::from_utf8(file_content) {
            Ok(content) => content,
            Err(e) => {
                tracing::error!(error = %e, "Invalid file content: not UTF-8");
                return Err(Status::invalid_argument(format!(
                    "Invalid file content: {}",
                    e
                )));
            }
        };

        // Detect and parse content based on file extension
        let mut api_storage = if req.file_name.ends_with(".yaml") || req.file_name.ends_with(".yml")
        {
            // Parse YAML content
            match serde_yaml::from_str::<ApiStorage>(&file_content) {
                Ok(storage) => storage,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        email = %email,
                        "Failed to parse YAML content"
                    );
                    return Err(Status::invalid_argument(format!(
                        "Invalid YAML format: {}",
                        e
                    )));
                }
            }
        } else if req.file_name.ends_with(".json") {
            // Parse JSON content
            match serde_json::from_str::<ApiStorage>(&file_content) {
                Ok(storage) => storage,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        email = %email,
                        "Failed to parse JSON content"
                    );
                    return Err(Status::invalid_argument(format!(
                        "Invalid JSON format: {}",
                        e
                    )));
                }
            }
        } else {
            tracing::error!(
                email = %email,
                filename = %req.file_name,
                "Unsupported file format"
            );
            return Err(Status::invalid_argument(
                "Unsupported file format. Please upload a YAML (.yaml/.yml) or JSON (.json) file.",
            ));
        };

        // Validate API groups
        if api_storage.api_groups.is_empty() {
            tracing::warn!(
                email = %email,
                "No API groups found in uploaded file"
            );
            return Err(Status::invalid_argument(
                "No API groups found in uploaded file",
            ));
        }

        // Process and enhance each group and endpoint
        let mut processed_groups = Vec::new();

        for group in &mut api_storage.api_groups {
            // Generate ID for group if not provided
            let group_id = if group.group.id.is_empty() {
                generate_id_from_text(&group.group.name)
            } else {
                group.group.id.clone()
            };

            // Process endpoints
            let mut processed_endpoints = Vec::new();
            for endpoint in &mut group.endpoints {
                // Generate ID for endpoint if not provided
                if endpoint.id.is_empty() {
                    endpoint.id = generate_id_from_text(&endpoint.text);
                }

                // Set group_id reference
                endpoint.group_id = group_id.clone();

                processed_endpoints.push(endpoint.clone());
            }

            // Create processed group
            let processed_group = ApiGroupWithEndpoints {
                group: ApiGroup {
                    id: group_id,
                    name: group.group.name.clone(),
                    description: group.group.description.clone(),
                    base: group.group.base.clone(),
                },
                endpoints: processed_endpoints,
            };

            processed_groups.push(processed_group);
        }

        let group_count = api_storage.api_groups.len();

        // Replace user API groups
        match self
            .store
            .replace_user_api_groups(&email, processed_groups)
            .await
        {
            Ok(endpoint_count) => {
                //let group_count = api_storage.api_groups.len();

                tracing::info!(
                    email = %email,
                    group_count = group_count,
                    endpoint_count = endpoint_count,
                    "Successfully imported API groups and endpoints"
                );

                Ok(Response::new(UploadApiGroupsResponse {
                    success: true,
                    message: "API groups successfully imported".to_string(),
                    imported_count: endpoint_count as i32,
                    group_count: group_count as i32,
                }))
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    "Failed to import API groups"
                );

                Err(Status::internal(format!(
                    "Failed to import API groups: {}",
                    e
                )))
            }
        }
    }

    // Add these methods to impl EndpointService for EndpointServiceImpl in src/grpc_server.rs
    async fn get_user_preferences(
        &self,
        request: Request<GetUserPreferencesRequest>,
    ) -> Result<Response<GetUserPreferencesResponse>, Status> {
        let email = request.into_inner().email;
        tracing::info!(email = %email, "Received get_user_preferences gRPC request");

        match self.store.get_user_preferences(&email).await {
            Ok(prefs) => {
                tracing::info!(
                    email = %email,
                    hidden_count = prefs.hidden_defaults.len(),
                    "Successfully retrieved user preferences"
                );

                // Convert to proto format
                let proto_prefs = ProtoUserPreferences {
                    email: prefs.email,
                    hidden_defaults: prefs.hidden_defaults,
                };

                Ok(Response::new(GetUserPreferencesResponse {
                    success: true,
                    message: "User preferences successfully retrieved".to_string(),
                    preferences: Some(proto_prefs),
                }))
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    "Failed to retrieve user preferences"
                );

                Ok(Response::new(GetUserPreferencesResponse {
                    success: false,
                    message: format!("Failed to retrieve user preferences: {}", e),
                    preferences: None,
                }))
            }
        }
    }

    async fn update_user_preferences(
        &self,
        request: Request<UpdateUserPreferencesRequest>,
    ) -> Result<Response<UpdateUserPreferencesResponse>, Status> {
        let req = request.into_inner();
        let email = req.email;
        let action = req.action;
        let endpoint_id = req.endpoint_id;

        tracing::info!(
            email = %email,
            action = %action,
            endpoint_id = %endpoint_id,
            "Received update_user_preferences gRPC request"
        );

        match self
            .store
            .update_user_preferences(&email, &action, &endpoint_id)
            .await
        {
            Ok(_) => {
                tracing::info!(
                    email = %email,
                    action = %action,
                    endpoint_id = %endpoint_id,
                    "Successfully updated user preferences"
                );

                Ok(Response::new(UpdateUserPreferencesResponse {
                    success: true,
                    message: "User preferences successfully updated".to_string(),
                }))
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    "Failed to update user preferences"
                );

                Ok(Response::new(UpdateUserPreferencesResponse {
                    success: false,
                    message: format!("Failed to update user preferences: {}", e),
                }))
            }
        }
    }

    async fn reset_user_preferences(
        &self,
        request: Request<ResetUserPreferencesRequest>,
    ) -> Result<Response<ResetUserPreferencesResponse>, Status> {
        let email = request.into_inner().email;

        tracing::info!(
            email = %email,
            "Received reset_user_preferences gRPC request"
        );

        match self.store.reset_user_preferences(&email).await {
            Ok(_) => {
                tracing::info!(
                    email = %email,
                    "Successfully reset user preferences"
                );

                Ok(Response::new(ResetUserPreferencesResponse {
                    success: true,
                    message: "User preferences successfully reset".to_string(),
                }))
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    email = %email,
                    "Failed to reset user preferences"
                );

                Ok(Response::new(ResetUserPreferencesResponse {
                    success: false,
                    message: format!("Failed to reset user preferences: {}", e),
                }))
            }
        }
    }
}
````

## File: src/endpoint_store/api_key_management.rs
````rust
use crate::endpoint_store::db_helpers::ResultExt;
use crate::endpoint_store::models::{ApiKeyInfo, KeyPreference};
use crate::endpoint_store::{EndpointStore, StoreError};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use chrono::Utc;
use rand::{rng, Rng};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Generate a secure API key with the prefix "sk_live_"
pub fn generate_secure_key() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0));

    let timestamp = duration.as_nanos().to_string();
    let mut rng = rng();
    let random_number: u64 = rng.random();
    let combined = format!("{}{}", timestamp, random_number);

    let mut hasher = Sha256::new();
    hasher.update(combined.as_bytes());
    let hash = hasher.finalize();

    let base64_hash = URL_SAFE_NO_PAD.encode(hash);
    let key = &base64_hash[0..32];

    format!("sk_live_{}", key)
}

/// Hash the API key for secure storage
pub fn hash_api_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let result = hasher.finalize();
    URL_SAFE_NO_PAD.encode(result)
}

/// Extract the key prefix for display purposes
pub fn extract_key_prefix(key: &str) -> String {
    let parts: Vec<&str> = key.split('_').collect();
    if parts.len() >= 3 {
        format!("sk_{}", &parts[2][..6])
    } else {
        format!("sk_{}", &key[7..13])
    }
}

/// Get API keys status for a user
pub async fn get_api_keys_status(
    store: &EndpointStore,
    email: &str,
) -> Result<KeyPreference, StoreError> {
    let client = store.get_conn().await?;

    tracing::debug!(email = %email, "Checking API keys status");

    let user_exists_row = client
        .query_opt("SELECT 1 FROM user_preferences WHERE email = $1", &[&email])
        .await
        .to_store_error()?;

    let user_exists = user_exists_row.is_some();

    tracing::debug!(email = %email, user_exists = user_exists, "User exists in preferences");

    if !user_exists {
        tracing::info!(email = %email, "Creating new user in preferences table");
        client
            .execute(
                "INSERT INTO user_preferences (email, hidden_defaults, credit_balance) VALUES ($1, '', 0)",
                &[&email],
            )
            .await
            .to_store_error()?;
    }

    let balance_row = client
        .query_one(
            "SELECT credit_balance FROM user_preferences WHERE email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;
    let balance: i64 = balance_row.get(0);

    tracing::debug!(email = %email, balance = balance, "Retrieved credit balance");

    let key_count_row = client
        .query_one(
            "SELECT COUNT(*) FROM api_keys WHERE email = $1 AND is_active = true",
            &[&email],
        )
        .await
        .to_store_error()?;
    let key_count: i64 = key_count_row.get(0);

    tracing::debug!(email = %email, active_key_count = key_count, "Found active API keys");

    if key_count == 0 {
        tracing::info!(email = %email, "No active API keys found - user is considered new");
        return Ok(KeyPreference {
            has_keys: false,
            active_key_count: 0,
            keys: vec![],
            balance,
        });
    }

    tracing::info!(email = %email, key_count = key_count, "User has active API keys");

    let rows = client
        .query(
            "SELECT id, key_prefix, key_name, generated_at, last_used
            FROM api_keys 
            WHERE email = $1 AND is_active = true 
            ORDER BY generated_at DESC",
            &[&email],
        )
        .await
        .to_store_error()?;

    let mut keys = Vec::new();
    for row in rows {
        keys.push(ApiKeyInfo {
            id: row.get(0),
            key_prefix: row.get(1),
            key_name: row.get(2),
            generated_at: row.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
            last_used: row
                .get::<_, Option<chrono::DateTime<chrono::Utc>>>(4)
                .map(|dt| dt.to_rfc3339()),
        });
    }

    tracing::debug!(email = %email, keys_found = keys.len(), "Retrieved API key details");

    Ok(KeyPreference {
        has_keys: true,
        active_key_count: key_count as usize,
        keys,
        balance,
    })
}

/// Revoke a specific API key
pub async fn revoke_api_key(
    store: &EndpointStore,
    email: &str,
    key_id: &str,
) -> Result<bool, StoreError> {
    let client = store.get_conn().await?;

    let key_exists_row = client
        .query_opt(
            "SELECT 1 FROM api_keys WHERE id = $1 AND email = $2 AND is_active = true",
            &[&key_id, &email],
        )
        .await
        .to_store_error()?;

    if key_exists_row.is_none() {
        return Ok(false);
    }

    client
        .execute(
            "UPDATE api_keys SET is_active = false WHERE id = $1 AND email = $2",
            &[&key_id, &email],
        )
        .await
        .to_store_error()?;

    Ok(true)
}

/// Revoke all API keys for a user
pub async fn revoke_all_api_keys(store: &EndpointStore, email: &str) -> Result<usize, StoreError> {
    let client = store.get_conn().await?;

    let affected = client
        .execute(
            "UPDATE api_keys SET is_active = false WHERE email = $1 AND is_active = true",
            &[&email],
        )
        .await
        .to_store_error()?;

    Ok(affected as usize)
}

/// Record API key usage
pub async fn record_api_key_usage(store: &EndpointStore, key_id: &str) -> Result<(), StoreError> {
    let client = store.get_conn().await?;
    let now = Utc::now();

    client
        .execute(
            "UPDATE api_keys SET 
             last_used = $1, 
             usage_count = usage_count + 1 
             WHERE id = $2 AND is_active = true",
            &[&now, &key_id],
        )
        .await
        .to_store_error()?;

    Ok(())
}

/// Validate an API key and return the key_id and email if valid
pub async fn validate_api_key(
    store: &EndpointStore,
    key: &str,
) -> Result<Option<(String, String)>, StoreError> {
    let client = store.get_conn().await?;
    let key_hash = hash_api_key(key);

    let row = client
        .query_opt(
            "SELECT id, email FROM api_keys WHERE key_hash = $1 AND is_active = true",
            &[&key_hash],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| (r.get(0), r.get(1))))
}

/// Get usage statistics for a specific API key
pub async fn get_api_key_usage(
    store: &EndpointStore,
    key_id: &str,
) -> Result<Option<ApiKeyInfo>, StoreError> {
    let client = store.get_conn().await?;

    let row = client
        .query_opt(
            "SELECT id, key_prefix, key_name, generated_at, last_used
             FROM api_keys 
             WHERE id = $1 AND is_active = true",
            &[&key_id],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| ApiKeyInfo {
        id: r.get(0),
        key_prefix: r.get(1),
        key_name: r.get(2),
        generated_at: r.get::<_, chrono::DateTime<chrono::Utc>>(3).to_rfc3339(),
        last_used: r
            .get::<_, Option<chrono::DateTime<chrono::Utc>>>(4)
            .map(|dt| dt.to_rfc3339()),
    }))
}

/// Update credit balance for a user
pub async fn update_credit_balance(
    store: &EndpointStore,
    email: &str,
    amount: i64,
) -> Result<i64, StoreError> {
    let client = store.get_conn().await?;

    let user_exists_row = client
        .query_opt("SELECT 1 FROM user_preferences WHERE email = $1", &[&email])
        .await
        .to_store_error()?;

    if user_exists_row.is_none() {
        client
            .execute(
                "INSERT INTO user_preferences (email, hidden_defaults, credit_balance) VALUES ($1, '', $2)",
                &[&email, &amount],
            )
            .await
            .to_store_error()?;

        return Ok(amount);
    }

    client
        .execute(
            "UPDATE user_preferences SET credit_balance = credit_balance + $1 WHERE email = $2",
            &[&amount, &email],
        )
        .await
        .to_store_error()?;

    let balance_row = client
        .query_one(
            "SELECT credit_balance FROM user_preferences WHERE email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    Ok(balance_row.get(0))
}

/// Get credit balance for a user
pub async fn get_credit_balance(store: &EndpointStore, email: &str) -> Result<i64, StoreError> {
    let client = store.get_conn().await?;

    let row = client
        .query_opt(
            "SELECT credit_balance FROM user_preferences WHERE email = $1",
            &[&email],
        )
        .await
        .to_store_error()?;

    Ok(row.map(|r| r.get(0)).unwrap_or(0))
}

/// Generate a new API key for a user
pub async fn generate_api_key(
    store: &EndpointStore,
    email: &str,
    key_name: &str,
) -> Result<(String, String, String), StoreError> {
    let mut client = store.get_conn().await?;
    let tx = client.transaction().await.to_store_error()?;

    let new_key = generate_secure_key();
    let key_hash = hash_api_key(&new_key);
    let key_prefix = extract_key_prefix(&new_key);
    let now = Utc::now();
    let key_id = Uuid::new_v4().to_string();

    let user_exists_row = tx
        .query_opt("SELECT 1 FROM user_preferences WHERE email = $1", &[&email])
        .await
        .to_store_error()?;

    if user_exists_row.is_none() {
        tx.execute(
            "INSERT INTO user_preferences (email, hidden_defaults, credit_balance) VALUES ($1, '', 0)",
            &[&email],
        )
        .await
        .to_store_error()?;
    }

    tx.execute(
        "INSERT INTO api_keys (
            id, email, key_hash, key_prefix, key_name, 
            generated_at, usage_count, is_active
        ) VALUES ($1, $2, $3, $4, $5, $6, 0, true)",
        &[&key_id, &email, &key_hash, &key_prefix, &key_name, &now],
    )
    .await
    .to_store_error()?;

    tx.commit().await.to_store_error()?;

    Ok((new_key, key_prefix, key_id))
}
````

## File: src/endpoint_store/get_api_groups_by_email.rs
````rust
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
    tracing::info!(email = %email, "Starting to fetch API groups and endpoints");
    let client = store.get_conn().await?;

    tracing::info!(email = %email, "Fetching custom groups and endpoints");
    let result = fetch_custom_groups_with_endpoints(&client, email).await?;

    tracing::info!(
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
    tracing::debug!(email = %email, "Fetching custom groups and endpoints");

    let groups_query = r#"
        SELECT g.id, g.name, g.description, g.base
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
        };

        let endpoints = fetch_custom_endpoints(client, email, &group.id).await?;

        tracing::debug!(
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
            e.id, e.text, e.description, e.verb, e.base, e.path, 
            p.name, p.description, p.required, 
            string_agg(pa.alternative, ',') as alternatives
        FROM endpoints e
        INNER JOIN user_endpoints ue ON e.id = ue.endpoint_id
        LEFT JOIN parameters p ON e.id = p.endpoint_id
        LEFT JOIN parameter_alternatives pa ON e.id = pa.endpoint_id AND p.name = pa.parameter_name
        WHERE ue.email = $1 AND e.group_id = $2
        GROUP BY 
            e.id, e.text, e.description, e.verb, e.base, e.path, 
            p.name, p.description, p.required
    "#;

    tracing::debug!(
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
        let param_name: Option<String> = row.get(6);
        let param_desc: Option<String> = row.get(7);
        let required: Option<bool> = row.get(8);
        let alternatives_str: Option<String> = row.get(9);

        let endpoint = endpoints_map.entry(id.clone()).or_insert_with(|| {
            tracing::debug!(
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
                parameters: Vec::new(),
                group_id: group_id.to_string(),
            }
        });

        if let (Some(name), Some(desc), Some(req)) = (param_name, param_desc, required) {
            let alternatives = alternatives_str
                .map(|s| s.split(',').map(String::from).collect::<Vec<_>>())
                .unwrap_or_default();

            tracing::trace!(
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

    tracing::debug!(
        group_id = %group_id,
        endpoint_count = result.len(),
        "Successfully retrieved custom endpoints for group"
    );

    Ok(result)
}
````

## File: endpoint_service.proto
````protobuf
syntax = "proto3";
package endpoint;

service EndpointService {
    rpc GetApiGroups (GetApiGroupsRequest) returns (stream GetApiGroupsResponse);
    rpc UploadApiGroups (UploadApiGroupsRequest) returns (UploadApiGroupsResponse);

    // New methods for user preferences
    rpc GetUserPreferences (GetUserPreferencesRequest) returns (GetUserPreferencesResponse);
    rpc UpdateUserPreferences (UpdateUserPreferencesRequest) returns (UpdateUserPreferencesResponse);
    rpc ResetUserPreferences (ResetUserPreferencesRequest) returns (ResetUserPreferencesResponse);
}

message GetApiGroupsRequest {
    string email = 1;
}

message Parameter {
    string name = 1;
    string description = 2;
    string required = 3;
    repeated string alternatives = 4;
}

message Endpoint {
    string id = 1;
    string text = 2;
    string description = 3;
    string verb = 4;
    string base = 5;
    string path = 6;
    repeated Parameter parameters = 7;
    string group_id = 8;
}

message ApiGroup {
    string id = 1;
    string name = 2;
    string description = 3;
    string base = 4;
    repeated Endpoint endpoints = 5;
}

message GetApiGroupsResponse {
    repeated ApiGroup api_groups = 1;
}

message UploadApiGroupsRequest {
    string email = 1;
    bytes file_content = 2;
    string file_name = 3;
}

message UploadApiGroupsResponse {
    bool success = 1;
    string message = 2;
    int32 imported_count = 3;
    int32 group_count = 4;
}

message UserPreferences {
    string email = 1;
    repeated string hidden_defaults = 2;
}

message GetUserPreferencesRequest {
    string email = 1;
}

message GetUserPreferencesResponse {
    bool success = 1;
    string message = 2;
    UserPreferences preferences = 3;
}

message UpdateUserPreferencesRequest {
    string email = 1;
    string action = 2; // "hide_default" or "show_default"
    string endpoint_id = 3;
}

message UpdateUserPreferencesResponse {
    bool success = 1;
    string message = 2;
}

message ResetUserPreferencesRequest {
    string email = 1;
}

message ResetUserPreferencesResponse {
    bool success = 1;
    string message = 2;
}
````

## File: config.yaml
````yaml
whoami: "api-store"
output: "grpc" # Options: console, file, grpc
level: "info"
# grpc:
#   address: "0.0.0.0"
#   port: 50057
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
server:
  grpc:
    host: "0.0.0.0"
    port: 50057
  http:
    host: "127.0.0.1"
    port: 5007
formatter_host: "127.0.0.1"
formatter_port: 6001
````

## File: src/endpoint_store/models.rs
````rust
use crate::endpoint_store::utils::generate_uuid;
use serde::{Deserialize, Serialize};

// Helper function to provide default verb value
fn default_verb() -> String {
    "GET".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UserPreferences {
    pub email: String,
    pub hidden_defaults: Vec<String>, // List of hidden default endpoint IDs
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdatePreferenceRequest {
    pub email: String,
    pub action: String, // "hide_default" or "show_default"
    pub endpoint_id: String,
}

use serde::Deserializer;

// Helper function for flexible boolean parsing
fn deserialize_flexible_bool<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum FlexibleBool {
        Bool(bool),
        String(String),
    }

    match FlexibleBool::deserialize(deserializer)? {
        FlexibleBool::Bool(b) => Ok(b.to_string()),
        FlexibleBool::String(s) => {
            // Validate string is a valid boolean representation
            match s.to_lowercase().as_str() {
                "true" | "false" => Ok(s.to_lowercase()),
                _ => Err(Error::custom(
                    "Invalid boolean string, must be 'true' or 'false'",
                )),
            }
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Parameter {
    pub name: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(
        default = "default_false_string",
        deserialize_with = "deserialize_flexible_bool"
    )]
    pub required: String,
    #[serde(default)]
    pub alternatives: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Endpoint {
    #[serde(default = "String::new")] // Allow empty, will be auto-generated
    pub id: String,
    pub text: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default)]
    pub parameters: Vec<Parameter>,
    #[serde(default = "default_verb")]
    #[serde(alias = "method")]
    pub verb: String,
    #[serde(default = "String::new")] // Allow empty, will inherit from group
    pub base: String,
    #[serde(default = "String::new")]
    pub path: String,
    #[serde(default = "String::new")] // Allow empty, will be set by parent group
    pub group_id: String,
}

fn default_false_string() -> String {
    "false".to_string()
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiGroup {
    #[serde(default = "generate_uuid")]
    pub id: String,
    pub name: String,
    #[serde(default = "String::new")]
    pub description: String,
    #[serde(default = "String::new")]
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

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiKeyInfo {
    pub id: String,
    pub key_prefix: String,
    pub key_name: String,
    pub generated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct KeyPreference {
    pub has_keys: bool,
    pub active_key_count: usize,
    pub keys: Vec<ApiKeyInfo>,
    pub balance: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GenerateKeyRequest {
    pub email: String,
    pub key_name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct UpdateCreditRequest {
    pub email: String,
    pub amount: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenUsage {
    pub estimated: bool,
    pub input_tokens: i64,
    pub model: String,
    pub output_tokens: i64,
    pub total_tokens: i64,
}

// Update existing LogApiUsageRequest
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogApiUsageRequest {
    pub key_id: String,
    pub email: String,
    pub endpoint_path: String, // Always "/api/analyze" for api0
    pub method: String,        // Always "POST" for api0
    pub status_code: Option<i32>,
    pub response_time_ms: Option<i64>,
    pub request_size_bytes: Option<i64>,
    pub response_size_bytes: Option<i64>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub usage: Option<TokenUsage>,
    // Add metadata for matched endpoint info (optional)
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LogApiUsageResponse {
    pub success: bool,
    pub message: String,
    pub log_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ApiUsageLog {
    pub id: String,
    pub key_id: String,
    pub email: String,
    pub endpoint_path: String,
    pub method: String,
    pub timestamp: String,
    pub response_status: Option<i32>,
    pub response_time_ms: Option<i64>,
    pub request_size: Option<i64>,
    pub response_size: Option<i64>,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    // Add token usage fields
    pub usage_estimated: Option<bool>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub total_tokens: Option<i64>,
    pub model_used: Option<String>,
    pub metadata: Option<serde_json::Value>,
}
````

## File: src/endpoint_store/mod.rs
````rust
mod add_user_api_group;
mod api_key_management;
mod authorized_domains;
mod cleanup;
pub mod db_helpers;
mod delete_user_api_group;
mod errors;
mod get_api_groups_by_email;
mod get_create_user_api_groups;
mod manage_single_endpoint;
use crate::endpoint_store::db_helpers::ResultExt;
mod delete_user_endpoint;
pub mod models;
mod replace_user_api_groups;
mod user_preferences;
mod utils;

pub use errors::*;
pub use models::*;
pub use utils::*;

use crate::db_pool::{create_pg_pool, PgConnection, PgPool};

#[derive(Clone)]
pub struct EndpointStore {
    pool: PgPool,
}

impl EndpointStore {
    pub async fn get_all_authorized_domains(&self) -> Result<Vec<String>, StoreError> {
        authorized_domains::get_all_authorized_domains(self).await
    }

    pub async fn initialize_system_domains(&self) -> Result<(), StoreError> {
        authorized_domains::initialize_system_domains(self).await
    }

    pub async fn get_group_base_url(&self, group_id: &str) -> Result<String, StoreError> {
        let client = self.get_conn().await?;

        let row = client
            .query_one("SELECT base FROM api_groups WHERE id = $1", &[&group_id])
            .await
            .map_err(|_| StoreError::Database("Group not found".to_string()))?;

        Ok(row.get(0))
    }

    pub async fn new(database_url: &str) -> Result<Self, StoreError> {
        tracing::info!("Initializing EndpointStore with PostgreSQL");

        let pool = create_pg_pool(database_url)
            .map_err(|e| StoreError::Pool(format!("Failed to create connection pool: {:?}", e)))?;

        let store = Self { pool };

        let client = store.get_conn().await?;

        client
            .batch_execute(include_str!("../../sql/schema.sql"))
            .await
            .map_err(|e| StoreError::Database(format!("Schema execution failed: {}", e)))?;

        store.initialize_system_domains().await?;
        Ok(store)
    }

    pub async fn get_conn(&self) -> Result<PgConnection, StoreError> {
        self.pool.get().await.to_store_error()
    }

    pub async fn get_user_preferences(&self, email: &str) -> Result<UserPreferences, StoreError> {
        user_preferences::get_user_preferences(self, email).await
    }

    pub async fn update_user_preferences(
        &self,
        email: &str,
        action: &str,
        endpoint_id: &str,
    ) -> Result<bool, StoreError> {
        user_preferences::update_user_preferences(self, email, action, endpoint_id).await
    }

    pub async fn reset_user_preferences(&self, email: &str) -> Result<bool, StoreError> {
        user_preferences::reset_user_preferences(self, email).await
    }

    pub async fn get_api_groups_with_preferences(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        let api_groups = self.get_api_groups_by_email(email).await?;

        let filtered_groups = api_groups
            .into_iter()
            .map(|group| ApiGroupWithEndpoints {
                group: group.group,
                endpoints: group.endpoints,
            })
            .filter(|group| !group.endpoints.is_empty())
            .collect();

        Ok(filtered_groups)
    }

    pub async fn get_or_create_user_api_groups(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        get_create_user_api_groups::get_or_create_user_api_groups(self, email).await
    }

    pub async fn get_api_groups_by_email(
        &self,
        email: &str,
    ) -> Result<Vec<ApiGroupWithEndpoints>, StoreError> {
        get_api_groups_by_email::get_api_groups_by_email(self, email).await
    }

    pub async fn replace_user_api_groups(
        &self,
        email: &str,
        api_groups: Vec<ApiGroupWithEndpoints>,
    ) -> Result<usize, StoreError> {
        replace_user_api_groups::replace_user_api_groups(self, email, api_groups).await
    }

    pub async fn add_user_api_group(
        &self,
        email: &str,
        api_group: &ApiGroupWithEndpoints,
    ) -> Result<usize, StoreError> {
        add_user_api_group::add_user_api_group(self, email, api_group).await
    }

    pub async fn delete_user_api_group(
        &self,
        email: &str,
        group_id: &str,
    ) -> Result<bool, StoreError> {
        delete_user_api_group::delete_user_api_group(self, email, group_id).await
    }

    pub async fn delete_user_endpoint(
        &self,
        email: &str,
        endpoint_id: &str,
    ) -> Result<bool, StoreError> {
        delete_user_endpoint::delete_user_endpoint(self, email, endpoint_id).await
    }

    pub(crate) async fn force_clean_user_data(&self, email: &str) -> Result<(), StoreError> {
        cleanup::force_clean_user_data(self, email).await
    }

    pub(crate) async fn fallback_clean_user_data(&self, email: &str) -> Result<(), StoreError> {
        cleanup::fallback_clean_user_data(self, email).await
    }

    pub async fn get_api_keys_status(&self, email: &str) -> Result<KeyPreference, StoreError> {
        api_key_management::get_api_keys_status(self, email).await
    }

    pub async fn generate_api_key(
        &self,
        email: &str,
        key_name: &str,
    ) -> Result<(String, String, String), StoreError> {
        api_key_management::generate_api_key(self, email, key_name).await
    }

    pub async fn revoke_api_key(&self, email: &str, key_id: &str) -> Result<bool, StoreError> {
        api_key_management::revoke_api_key(self, email, key_id).await
    }

    pub async fn revoke_all_api_keys(&self, email: &str) -> Result<usize, StoreError> {
        api_key_management::revoke_all_api_keys(self, email).await
    }

    pub async fn validate_api_key(
        &self,
        key: &str,
    ) -> Result<Option<(String, String)>, StoreError> {
        api_key_management::validate_api_key(self, key).await
    }

    pub async fn record_api_key_usage(&self, key_id: &str) -> Result<(), StoreError> {
        api_key_management::record_api_key_usage(self, key_id).await
    }

    pub async fn get_api_key_usage(&self, key_id: &str) -> Result<Option<ApiKeyInfo>, StoreError> {
        api_key_management::get_api_key_usage(self, key_id).await
    }

    pub async fn log_api_usage(&self, request: &LogApiUsageRequest) -> Result<String, StoreError> {
        let client = self.get_conn().await?;
        let log_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        // Convert metadata to JSON string for storage
        let metadata_json = request
            .metadata
            .as_ref()
            .and_then(|m| serde_json::to_string(m).ok());

        client
        .execute(
            "INSERT INTO api_usage_logs (
            id, key_id, email, endpoint_path, method, timestamp,
            response_status, response_time_ms, request_size, response_size,
            ip_address, user_agent, usage_estimated, input_tokens,
            output_tokens, total_tokens, model_used, metadata
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18::jsonb)",
            &[
                &log_id,
                &request.key_id,
                &request.email,
                &request.endpoint_path,
                &request.method,
                &now,
                &request.status_code,
                &request.response_time_ms,
                &request.request_size_bytes,
                &request.response_size_bytes,
                &request.ip_address,
                &request.user_agent,
                &request.usage.as_ref().map(|u| u.estimated),
                &request.usage.as_ref().map(|u| u.input_tokens),
                &request.usage.as_ref().map(|u| u.output_tokens),
                &request.usage.as_ref().map(|u| u.total_tokens),
                &request.usage.as_ref().map(|u| u.model.clone()),
                &metadata_json, // Now a String, which can be cast to jsonb
            ],
        )
        .await
        .to_store_error()?;

        Ok(log_id)
    }

    pub async fn get_api_usage_logs(
        &self,
        key_id: &str,
        limit: Option<i64>,
    ) -> Result<Vec<ApiUsageLog>, StoreError> {
        let client = self.get_conn().await?;
        let limit = limit.unwrap_or(50).min(100);

        let rows = client
            .query(
                "SELECT id, key_id, email, endpoint_path, method, timestamp,
            response_status, response_time_ms, request_size, response_size,
            ip_address, user_agent, usage_estimated, input_tokens,
            output_tokens, total_tokens, model_used, metadata
            FROM api_usage_logs 
            WHERE key_id = $1 
            ORDER BY timestamp DESC 
            LIMIT $2",
                &[&key_id, &limit],
            )
            .await
            .to_store_error()?;

        let mut logs = Vec::new();
        for row in rows {
            // Get metadata as string first, then parse to JSON
            let metadata_str: Option<String> = row.get(17);
            let metadata = metadata_str.and_then(|s| serde_json::from_str(&s).ok());

            logs.push(ApiUsageLog {
                id: row.get(0),
                key_id: row.get(1),
                email: row.get(2),
                endpoint_path: row.get(3),
                method: row.get(4),
                timestamp: row.get::<_, chrono::DateTime<chrono::Utc>>(5).to_rfc3339(),
                response_status: row.get(6),
                response_time_ms: row.get(7),
                request_size: row.get(8),
                response_size: row.get(9),
                ip_address: row.get(10),
                user_agent: row.get(11),
                usage_estimated: row.get(12),
                input_tokens: row.get(13),
                output_tokens: row.get(14),
                total_tokens: row.get(15),
                model_used: row.get(16),
                metadata,
            });
        }

        Ok(logs)
    }

    pub async fn update_credit_balance(&self, email: &str, amount: i64) -> Result<i64, StoreError> {
        api_key_management::update_credit_balance(self, email, amount).await
    }

    pub async fn get_credit_balance(&self, email: &str) -> Result<i64, StoreError> {
        api_key_management::get_credit_balance(self, email).await
    }

    pub async fn manage_single_endpoint(
        &self,
        email: &str,
        endpoint: &Endpoint,
    ) -> Result<String, StoreError> {
        manage_single_endpoint::manage_single_endpoint(self, email, endpoint).await
    }
}
````

## File: src/http_server.rs
````rust
use crate::add_api_group::add_api_group;
use crate::delete_api_group::delete_api_group;
use crate::endpoint_store::EndpointStore;
use crate::formatter::YamlFormatter;
use crate::generate_api_key::generate_api_key;
use crate::get_api_groups::get_api_groups;
use crate::get_api_key_usage::get_api_key_usage;
use crate::get_api_keys_status::get_api_keys_status;
use crate::get_api_usage_logs::get_api_usage_logs;
use crate::get_authorized_domains::get_authorized_domains;
use crate::get_credit_balance_handler::get_credit_balance_handler;
use crate::get_user_preferences::get_user_preferences;
use crate::health_check::health_check;
use crate::log_api_usage::log_api_usage;
use crate::manage_endpoint::manage_endpoint;
use crate::reset_user_preferences::reset_user_preferences;
use crate::revoke_all_api_keys_handler::revoke_all_api_keys_handler;
use crate::revoke_api_key_handler::revoke_api_key_handler;
use crate::update_api_group::update_api_group;
use crate::update_credit_balance_handler::update_credit_balance_handler;
use crate::update_user_preferences::update_user_preferences;
use crate::upload_api_config::upload_api_config;
use crate::validate_api_key::validate_api_key;

use crate::delete_endpoint::delete_endpoint;
use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpServer};
use std::sync::Arc;
// use actix_web::{web, HttpResponse, Responder};
use std::net::SocketAddr;
use tokio::task;

// Server startup function
pub async fn start_http_server(
    store: Arc<EndpointStore>,
    formatter: Arc<YamlFormatter>,
    host: &str,
    port: u16,
) -> std::io::Result<()> {
    let addr = format!("{}:{}", host, port);
    let addr = addr.parse::<SocketAddr>().unwrap();
    let store_clone = store.clone();
    let formatter_clone = formatter.clone();

    // Run Actix Web in a blocking task to avoid Send issues
    let _ = task::spawn_blocking(move || {
        let sys = actix_web::rt::System::new();
        sys.block_on(async move {
            tracing::info!("Starting HTTP server at {}", addr);

            HttpServer::new(move || {
                // Configure CORS
                let cors = Cors::default()
                    .allow_any_origin()
                    .allow_any_method()
                    .allow_any_header()
                    .max_age(3600);

                App::new()
                    .wrap(Logger::default())
                    .wrap(cors)
                    // .wrap(ApiKeyAuth::new(store_clone.clone()))
                    .app_data(web::Data::new(store_clone.clone()))
                    .app_data(web::Data::new(formatter_clone.clone())) // Add formatter to app data
                    .service(
                        web::scope("/api")
                            // API groups endpoints
                            .route("/upload", web::post().to(upload_api_config))
                            .route("/groups/{email}", web::get().to(get_api_groups))
                            .route("/group", web::post().to(add_api_group))
                            .route("/group", web::put().to(update_api_group))
                            .route("/endpoint", web::post().to(manage_endpoint))
                            .route("/domains/authorized", web::get().to(get_authorized_domains))
                            .route("/user/usage/log", web::post().to(log_api_usage))
                            .route(
                                "/user/usage/logs/{email}/{key_id}",
                                web::get().to(get_api_usage_logs),
                            )
                            .route(
                                "/groups/{email}/{group_id}",
                                web::delete().to(delete_api_group),
                            )
                            // User preferences endpoints
                            .route(
                                "/user/preferences/{email}",
                                web::get().to(get_user_preferences),
                            )
                            .route("/user/preferences", web::post().to(update_user_preferences))
                            .route(
                                "/user/preferences/{email}",
                                web::delete().to(reset_user_preferences),
                            )
                            // Updated API key endpoints
                            .route("/user/keys/{email}", web::get().to(get_api_keys_status))
                            .route("/user/keys", web::post().to(generate_api_key))
                            .route(
                                "/user/keys/{email}/{key_id}",
                                web::delete().to(revoke_api_key_handler),
                            )
                            .route(
                                "/user/keys/{email}",
                                web::delete().to(revoke_all_api_keys_handler),
                            )
                            // Credit balance endpoints
                            .route(
                                "/user/credits/{email}",
                                web::get().to(get_credit_balance_handler),
                            )
                            .route(
                                "/user/credits",
                                web::post().to(update_credit_balance_handler),
                            )
                            // Key validation and usage
                            .route("/key/validate", web::post().to(validate_api_key))
                            .route(
                                "/endpoints/{email}/{endpoint_id}",
                                web::delete().to(delete_endpoint),
                            )
                            .route(
                                "/key/usage/{email}/{key_id}",
                                web::get().to(get_api_key_usage),
                            )
                            .route("/health", web::get().to(health_check)),
                    )
                // Credit balance endpoints
            })
            .bind(addr)?
            .workers(1)
            .run()
            .await
        })
    })
    .await
    .expect("Actix system panicked");

    Ok(())
}
````

## File: sql/schema.sql
````sql
-- PostgreSQL Schema

-- Keep user_preferences table for preferences and credit
CREATE TABLE IF NOT EXISTS user_preferences (
    email VARCHAR NOT NULL,
    hidden_defaults TEXT NOT NULL DEFAULT '',
    credit_balance BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (email)
);

-- API keys table
CREATE TABLE IF NOT EXISTS api_keys (
    id VARCHAR NOT NULL,
    email VARCHAR NOT NULL,
    key_hash VARCHAR NOT NULL,
    key_prefix VARCHAR NOT NULL,
    key_name VARCHAR NOT NULL,
    generated_at TIMESTAMP WITH TIME ZONE NOT NULL,
    last_used TIMESTAMP WITH TIME ZONE,
    usage_count BIGINT NOT NULL DEFAULT 0,
    is_active BOOLEAN NOT NULL DEFAULT true,
    PRIMARY KEY (id),
    FOREIGN KEY (email) REFERENCES user_preferences(email)
);

-- API Groups table
CREATE TABLE IF NOT EXISTS api_groups (
    id VARCHAR PRIMARY KEY,
    name VARCHAR NOT NULL,
    description VARCHAR NOT NULL DEFAULT '',
    base VARCHAR NOT NULL DEFAULT ''
);

-- Endpoints table with group reference
CREATE TABLE IF NOT EXISTS endpoints (
    id VARCHAR PRIMARY KEY,
    text VARCHAR NOT NULL,
    description VARCHAR NOT NULL DEFAULT '',
    verb VARCHAR NOT NULL DEFAULT 'GET',
    base VARCHAR NOT NULL DEFAULT '',
    path VARCHAR NOT NULL DEFAULT '',
    group_id VARCHAR,
    FOREIGN KEY (group_id) REFERENCES api_groups(id)
);

-- User associations for groups
CREATE TABLE IF NOT EXISTS user_groups (
    email VARCHAR NOT NULL,
    group_id VARCHAR NOT NULL,
    FOREIGN KEY (group_id) REFERENCES api_groups(id),
    PRIMARY KEY (email, group_id)
);

-- User endpoint associations
CREATE TABLE IF NOT EXISTS user_endpoints (
    email VARCHAR NOT NULL,
    endpoint_id VARCHAR NOT NULL,
    FOREIGN KEY (endpoint_id) REFERENCES endpoints(id),
    PRIMARY KEY (email, endpoint_id)
);

-- Parameters table
CREATE TABLE IF NOT EXISTS parameters (
    endpoint_id VARCHAR,
    name VARCHAR NOT NULL,
    description VARCHAR NOT NULL DEFAULT '',
    required BOOLEAN NOT NULL DEFAULT false,
    FOREIGN KEY (endpoint_id) REFERENCES endpoints(id)
);

-- Parameter alternatives
CREATE TABLE IF NOT EXISTS parameter_alternatives (
    endpoint_id VARCHAR,
    parameter_name VARCHAR,
    alternative VARCHAR NOT NULL,
    FOREIGN KEY (endpoint_id) REFERENCES endpoints(id)
);

-- Domains table
CREATE TABLE IF NOT EXISTS domains (
    id VARCHAR NOT NULL,
    email VARCHAR NOT NULL,
    domain VARCHAR NOT NULL,
    verified BOOLEAN NOT NULL DEFAULT false,
    added_at TIMESTAMP WITH TIME ZONE NOT NULL,
    last_used TIMESTAMP WITH TIME ZONE,
    verification_token VARCHAR,
    PRIMARY KEY (id),
    UNIQUE(email, domain)
);

-- API usage logs table for detailed tracking
CREATE TABLE IF NOT EXISTS api_usage_logs (
    id VARCHAR NOT NULL,
    key_id VARCHAR NOT NULL,
    email VARCHAR NOT NULL,
    endpoint_path VARCHAR NOT NULL,
    method VARCHAR NOT NULL,
    timestamp TIMESTAMP WITH TIME ZONE NOT NULL,
    response_status INTEGER,
    response_time_ms BIGINT,
    request_size BIGINT,
    response_size BIGINT,
    ip_address VARCHAR,
    user_agent VARCHAR,
    usage_estimated BOOLEAN,
    input_tokens BIGINT,
    output_tokens BIGINT,
    total_tokens BIGINT,
    model_used VARCHAR,
    metadata JSONB,
    PRIMARY KEY (id),
    FOREIGN KEY (key_id) REFERENCES api_keys(id)
);

-- Create indexes for better performance
CREATE INDEX IF NOT EXISTS idx_api_keys_email ON api_keys(email);
CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash);
CREATE INDEX IF NOT EXISTS idx_domains_email ON domains(email);
CREATE INDEX IF NOT EXISTS idx_domains_verified ON domains(verified);
CREATE INDEX IF NOT EXISTS idx_usage_logs_timestamp ON api_usage_logs(timestamp);
CREATE INDEX IF NOT EXISTS idx_usage_logs_key_id ON api_usage_logs(key_id);
CREATE INDEX IF NOT EXISTS idx_usage_logs_email ON api_usage_logs(email);
````

## File: Cargo.toml
````toml
[package]
name = "store"
version = "0.1.0"
edition = "2021"

[dependencies]
chrono = { version = "0.4.39", features = ["serde"] }
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
async-stream = "0.3.6"
uuid = { version = "1.13.1", features = ["v4"] }
serde_json = "1.0.139"
actix-web = "4.9.0"
actix-cors = "0.7.0"
base64 = "0.22.1"
slug = "0.1.6"
async-trait = "0.1.88"
rand = { version = "0.9.0" }
sha2 = "0.10.8"
reqwest = { version = "0.12.15", features = ["json", "multipart"] }
tempfile = "3.19.1"
h2 = "0.4.10"
tokio-postgres = { version = "0.7.14", features = ["with-chrono-0_4"] }
deadpool-postgres = { version = "0.14.1", features = ["serde"] }
postgres-types = { version = "0.2.10", features = ["derive", "with-chrono-0_4"] }
dotenvy = "0.15.7"

[build-dependencies]
tonic-build = "0.12.3"

[[bin]]
name = "store"
path = "src/main.rs"
````
