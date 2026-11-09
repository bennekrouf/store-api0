// src/admin/legal_details.rs
//
// The company a workspace contracts through, and its dated acceptance of api0's
// documents. Owners and admins fill it in; what they declare is not verified.
//
//   GET /api/user/legal-details?email=…
//   PUT /api/user/legal-details
//
// Accepting a new version adds a row and never changes an old one. To publish a
// new version, or a new document, change `DOCUMENTS`: the form then asks again.

use crate::app_log;
use crate::endpoint_store::tenant_management::get_default_tenant;
use crate::endpoint_store::EndpointStore;
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::tenant_billing::{can_edit, EDITOR_ROLES};
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use tokio_postgres::Row;

#[derive(Debug)]
struct Document {
    id: &'static str,
    title: &'static str,
    version: &'static str,
    url: &'static str,
}

const DOCUMENTS: &[Document] = &[Document {
    id: "terms",
    title: "Terms of Service",
    version: "2025-10-27",
    url: "https://api0.ai/terms",
}];

const MAX_NAME: usize = 200;
const MAX_ADDRESS: usize = 500;
const MAX_NUMBER: usize = 50;
const MAX_EMAIL: usize = 254;
const VAT_LENGTH: std::ops::RangeInclusive<usize> = 4..=18;

const COLUMNS: &str = "legal_name, registered_address, country, company_number, vat_id, billing_email, \
                       privacy_contact_name, privacy_contact_email, security_contact_email";

#[derive(Debug, Serialize, PartialEq)]
struct Details {
    legal_name: String,
    registered_address: String,
    country: String,
    company_number: String,
    vat_id: String,
    billing_email: String,
    privacy_contact_name: String,
    privacy_contact_email: String,
    security_contact_email: String,
}

#[derive(Deserialize)]
pub struct EmailQuery {
    pub email: String,
}

#[derive(Deserialize)]
pub struct SaveBody {
    pub email: String,
    pub tenant_id: String,
    /// Another workspace's company, when this one belongs to the same company.
    #[serde(default)]
    pub entity_id: Option<String>,
    #[serde(default)]
    pub legal_name: String,
    #[serde(default)]
    pub registered_address: String,
    #[serde(default)]
    pub country: String,
    #[serde(default)]
    pub company_number: String,
    #[serde(default)]
    pub vat_id: String,
    #[serde(default)]
    pub billing_email: String,
    #[serde(default)]
    pub privacy_contact_name: String,
    #[serde(default)]
    pub privacy_contact_email: String,
    #[serde(default)]
    pub security_contact_email: String,
    #[serde(default)]
    pub acts_as_business: bool,
    #[serde(default)]
    pub authority_confirmed: bool,
    #[serde(default)]
    pub accepted: Vec<String>,
}

enum SaveError {
    Invalid(String),
    NotYourCompany,
    Failed(String),
}

impl From<tokio_postgres::Error> for SaveError {
    fn from(e: tokio_postgres::Error) -> Self {
        SaveError::Failed(e.to_string())
    }
}

fn text(value: &str, max: usize, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.chars().count() > max {
        return Err(format!("{label} is too long"));
    }
    if value.chars().any(|c| c.is_control() && c != '\n' && c != '\r') {
        return Err(format!("{label} contains invalid characters"));
    }
    Ok(value.to_string())
}

fn required(value: &str, max: usize, label: &str) -> Result<String, String> {
    let value = text(value, max, label)?;
    if value.is_empty() {
        return Err(format!("{label} is required"));
    }
    Ok(value)
}

fn email(value: &str, label: &str) -> Result<String, String> {
    let value = required(value, MAX_EMAIL, label)?;
    let valid = !value.chars().any(char::is_whitespace)
        && value.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && !domain.contains('@')
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        });
    if !valid {
        return Err(format!("{label} is not a valid email address"));
    }
    Ok(value)
}

fn country(value: &str) -> Result<String, String> {
    let code = value.trim().to_ascii_uppercase();
    if code.len() != 2 || !code.chars().all(|c| c.is_ascii_uppercase()) {
        return Err("Country is required".into());
    }
    Ok(code)
}

/// Stored without separators, so CHE-123.456.789 MWST and CHE123456789MWST are
/// one number. This checks the shape only.
fn vat_id(value: &str) -> Result<String, String> {
    let vat: String = value
        .chars()
        .filter(|c| !matches!(c, ' ' | '.' | '-'))
        .collect::<String>()
        .to_ascii_uppercase();
    if vat.is_empty() {
        return Ok(vat);
    }
    let shaped = VAT_LENGTH.contains(&vat.len())
        && vat.chars().all(|c| c.is_ascii_alphanumeric())
        && vat.chars().take(2).all(|c| c.is_ascii_alphabetic());
    if !shaped {
        return Err("VAT number should start with its country code, for example DE123456789".into());
    }
    Ok(vat)
}

fn clean(body: &SaveBody) -> Result<Details, String> {
    if !body.acts_as_business {
        return Err("Confirm that this company uses api0 for its business".into());
    }
    Ok(Details {
        legal_name: required(&body.legal_name, MAX_NAME, "Legal name")?,
        registered_address: required(&body.registered_address, MAX_ADDRESS, "Registered address")?,
        country: country(&body.country)?,
        company_number: text(&body.company_number, MAX_NUMBER, "Company number")?,
        vat_id: vat_id(&body.vat_id)?,
        billing_email: email(&body.billing_email, "Billing email")?,
        privacy_contact_name: text(&body.privacy_contact_name, MAX_NAME, "Data protection contact")?,
        privacy_contact_email: email(&body.privacy_contact_email, "Data protection email")?,
        security_contact_email: email(&body.security_contact_email, "Security contact email")?,
    })
}

/// The documents this save has to record: those the company has not yet accepted
/// at their current version, all of which must be ticked.
fn newly_accepted<'a>(
    documents: &'a [Document],
    already: &[(String, String)],
    ticked: &[String],
    authority: bool,
) -> Result<Vec<&'a Document>, String> {
    let due: Vec<&Document> = documents
        .iter()
        .filter(|d| !already.iter().any(|(id, version)| id == d.id && version == d.version))
        .collect();
    if let Some(missing) = due.iter().find(|d| !ticked.iter().any(|t| t == d.id)) {
        return Err(format!("Accept the {} to continue", missing.title));
    }
    if !due.is_empty() && !authority {
        return Err("Confirm that you can accept these documents for the company".into());
    }
    Ok(due)
}

fn details_of(row: &Row) -> Details {
    Details {
        legal_name: row.get("legal_name"),
        registered_address: row.get("registered_address"),
        country: row.get("country"),
        company_number: row.get("company_number"),
        vat_id: row.get("vat_id"),
        billing_email: row.get("billing_email"),
        privacy_contact_name: row.get("privacy_contact_name"),
        privacy_contact_email: row.get("privacy_contact_email"),
        security_contact_email: row.get("security_contact_email"),
    }
}

struct Accepted {
    document: String,
    version: String,
    at: DateTime<Utc>,
    by: String,
}

fn documents_json(accepted: &[Accepted]) -> Vec<serde_json::Value> {
    DOCUMENTS
        .iter()
        .map(|d| {
            let found = accepted.iter().find(|a| a.document == d.id && a.version == d.version);
            json!({
                "id": d.id,
                "title": d.title,
                "version": d.version,
                "url": d.url,
                "accepted": found.map(|a| json!({ "at": a.at, "by": a.by })),
            })
        })
        .collect()
}

fn is_complete(details: Option<&Details>, accepted: &[Accepted]) -> bool {
    details.is_some()
        && DOCUMENTS.iter().all(|d| accepted.iter().any(|a| a.document == d.id && a.version == d.version))
}

async fn read(store: &EndpointStore, tenant_id: &str, tenant_name: &str, email: &str, editable: bool) -> Result<serde_json::Value, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let err = |e: tokio_postgres::Error| e.to_string();

    let entity_id: Option<String> = client
        .query_one("SELECT legal_entity_id FROM tenants WHERE id = $1", &[&tenant_id])
        .await
        .map_err(err)?
        .get(0);

    let mut details = None;
    let mut accepted = Vec::new();
    if let Some(id) = &entity_id {
        let row = client
            .query_one(&format!("SELECT {COLUMNS} FROM legal_entities WHERE id = $1"), &[id])
            .await
            .map_err(err)?;
        details = Some(details_of(&row));
        accepted = client
            .query(
                "SELECT document, version, accepted_at, accepted_by FROM legal_acceptances WHERE legal_entity_id = $1",
                &[id],
            )
            .await
            .map_err(err)?
            .iter()
            .map(|r| Accepted { document: r.get(0), version: r.get(1), at: r.get(2), by: r.get(3) })
            .collect();
    }
    let complete = is_complete(details.as_ref(), &accepted);

    let mut others = Vec::new();
    if editable {
        let roles: Vec<&str> = EDITOR_ROLES.to_vec();
        let rows = client
            .query(
                &format!(
                    "SELECT DISTINCT ON (e.id) e.id, t.name AS tenant_name, {COLUMNS}
                       FROM legal_entities e
                       JOIN tenants t ON t.legal_entity_id = e.id
                       JOIN tenant_users u ON u.tenant_id = t.id
                      WHERE LOWER(u.email) = LOWER($1) AND u.role = ANY($2)
                        AND t.id <> $3 AND e.id IS DISTINCT FROM $4
                      ORDER BY e.id, t.name"
                ),
                &[&email, &roles, &tenant_id, &entity_id],
            )
            .await
            .map_err(err)?;
        others = rows
            .iter()
            .map(|r| {
                json!({
                    "id": r.get::<_, String>("id"),
                    "tenant_name": r.get::<_, String>("tenant_name"),
                    "details": details_of(r),
                })
            })
            .collect();
    }

    Ok(json!({
        "success": true,
        "tenant_id": tenant_id,
        "tenant_name": tenant_name,
        "can_edit": editable,
        "complete": complete,
        // Contact details stay with the people who manage them.
        "details": if editable { json!(details) } else { json!(null) },
        "documents": documents_json(&accepted),
        "other_companies": others,
    }))
}

async fn save(
    store: &EndpointStore,
    tenant_id: &str,
    tenant_name: &str,
    body: &SaveBody,
    details: &Details,
) -> Result<(), SaveError> {
    let mut client = store.get_admin_conn().await.map_err(|e| SaveError::Failed(e.to_string()))?;
    let tx = client.transaction().await?;

    // The lock stops two first saves from creating two companies.
    let linked: Option<String> = tx
        .query_one("SELECT legal_entity_id FROM tenants WHERE id = $1 FOR UPDATE", &[&tenant_id])
        .await?
        .get(0);

    let reuse = body.entity_id.as_deref().filter(|id| !id.is_empty() && Some(*id) != linked.as_deref());
    if let Some(id) = reuse {
        let roles: Vec<&str> = EDITOR_ROLES.to_vec();
        let allowed = tx
            .query_opt(
                "SELECT 1 FROM tenants t
                   JOIN tenant_users u ON u.tenant_id = t.id
                  WHERE t.legal_entity_id = $1 AND LOWER(u.email) = LOWER($2) AND u.role = ANY($3)
                  LIMIT 1",
                &[&id, &body.email, &roles],
            )
            .await?;
        if allowed.is_none() {
            return Err(SaveError::NotYourCompany);
        }
    }
    let existing = reuse.map(str::to_string).or(linked);

    let already: Vec<(String, String)> = match &existing {
        None => vec![],
        Some(id) => tx
            .query("SELECT document, version FROM legal_acceptances WHERE legal_entity_id = $1", &[id])
            .await?
            .iter()
            .map(|r| (r.get(0), r.get(1)))
            .collect(),
    };
    let to_record = newly_accepted(DOCUMENTS, &already, &body.accepted, body.authority_confirmed)
        .map_err(SaveError::Invalid)?;

    let entity_id = match existing {
        Some(id) => {
            tx.execute(
                "UPDATE legal_entities SET legal_name = $2, registered_address = $3, country = $4,
                        company_number = $5, vat_id = $6, billing_email = $7, privacy_contact_name = $8,
                        privacy_contact_email = $9, security_contact_email = $10, acts_as_business = TRUE,
                        updated_by = $11, updated_at = NOW()
                  WHERE id = $1",
                &[
                    &id,
                    &details.legal_name,
                    &details.registered_address,
                    &details.country,
                    &details.company_number,
                    &details.vat_id,
                    &details.billing_email,
                    &details.privacy_contact_name,
                    &details.privacy_contact_email,
                    &details.security_contact_email,
                    &body.email,
                ],
            )
            .await?;
            id
        }
        None => {
            let id = format!("le_{}", uuid::Uuid::new_v4().simple());
            tx.execute(
                &format!(
                    "INSERT INTO legal_entities (id, {COLUMNS}, acts_as_business, created_by, updated_by)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, TRUE, $11, $11)"
                ),
                &[
                    &id,
                    &details.legal_name,
                    &details.registered_address,
                    &details.country,
                    &details.company_number,
                    &details.vat_id,
                    &details.billing_email,
                    &details.privacy_contact_name,
                    &details.privacy_contact_email,
                    &details.security_contact_email,
                    &body.email,
                ],
            )
            .await?;
            id
        }
    };

    tx.execute("UPDATE tenants SET legal_entity_id = $2 WHERE id = $1", &[&tenant_id, &entity_id]).await?;

    for document in to_record {
        tx.execute(
            "INSERT INTO legal_acceptances
                 (legal_entity_id, legal_name, tenant_id, tenant_name, document, version, accepted_by, authority_declared)
             VALUES ($1, $2, $3, $4, $5, $6, $7, TRUE)
             ON CONFLICT (legal_entity_id, document, version) DO NOTHING",
            &[
                &entity_id,
                &details.legal_name,
                &tenant_id,
                &tenant_name,
                &document.id,
                &document.version,
                &body.email,
            ],
        )
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

fn failure(status: actix_web::http::StatusCode, error: impl Into<String>) -> HttpResponse {
    HttpResponse::build(status).json(json!({ "success": false, "error": error.into() }))
}

fn failed(what: &str, e: String) -> HttpResponse {
    app_log!(error, error = %e, "Legal details: {}", what);
    failure(actix_web::http::StatusCode::INTERNAL_SERVER_ERROR, format!("Could not {what}"))
}

/// GET /api/user/legal-details?email=…
pub async fn get_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<EmailQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant = match get_default_tenant(&store, &query.email).await {
        Ok(t) => t,
        Err(e) => return failed("find the workspace", e.to_string()),
    };
    let editable = match can_edit(&store, &tenant.id, &query.email).await {
        Ok(e) => e,
        Err(e) => return failed("read the caller's role", e),
    };
    match read(&store, &tenant.id, &tenant.name, &query.email, editable).await {
        Ok(body) => HttpResponse::Ok().json(body),
        Err(e) => failed("read the legal details", e),
    }
}

/// PUT /api/user/legal-details
pub async fn put_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    body: web::Json<SaveBody>,
) -> impl Responder {
    use actix_web::http::StatusCode;

    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let tenant = match get_default_tenant(&store, &body.email).await {
        Ok(t) => t,
        Err(e) => return failed("find the workspace", e.to_string()),
    };
    // The active workspace is server state: it may have moved since the form opened.
    if tenant.id != body.tenant_id {
        return failure(StatusCode::CONFLICT, "Your active workspace changed. Reload the page and check the details.");
    }
    match can_edit(&store, &tenant.id, &body.email).await {
        Ok(true) => {}
        Ok(false) => return failure(StatusCode::FORBIDDEN, "Only the workspace's owners and admins can change its legal details"),
        Err(e) => return failed("read the caller's role", e),
    }
    let details = match clean(&body) {
        Ok(d) => d,
        Err(why) => return failure(StatusCode::BAD_REQUEST, why),
    };
    match save(&store, &tenant.id, &tenant.name, &body, &details).await {
        Ok(()) => HttpResponse::Ok().json(json!({ "success": true })),
        Err(SaveError::Invalid(why)) => failure(StatusCode::BAD_REQUEST, why),
        Err(SaveError::NotYourCompany) => failure(StatusCode::FORBIDDEN, "You do not manage a workspace of that company"),
        Err(SaveError::Failed(e)) => failed("save the legal details", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> SaveBody {
        SaveBody {
            email: "owner@example.com".into(),
            tenant_id: "t".into(),
            entity_id: None,
            legal_name: "  Example SA ".into(),
            registered_address: "1 Rue Exemple\n1200 Geneva".into(),
            country: "ch".into(),
            company_number: "".into(),
            vat_id: "CHE-123.456.789 MWST".into(),
            billing_email: "billing@example.com".into(),
            privacy_contact_name: "".into(),
            privacy_contact_email: "privacy@example.com".into(),
            security_contact_email: "security@example.com".into(),
            acts_as_business: true,
            authority_confirmed: true,
            accepted: vec!["terms".into()],
        }
    }

    const DOCS: &[Document] = &[
        Document { id: "terms", title: "Terms", version: "2", url: "" },
        Document { id: "dpa", title: "DPA", version: "1", url: "" },
    ];

    #[test]
    fn details_are_tidied() {
        let d = clean(&body()).unwrap();
        assert_eq!(d.legal_name, "Example SA");
        assert_eq!(d.country, "CH");
        assert_eq!(d.vat_id, "CHE123456789MWST");
        assert_eq!(d.registered_address, "1 Rue Exemple\n1200 Geneva");
    }

    #[test]
    fn a_consumer_is_turned_away() {
        let b = SaveBody { acts_as_business: false, ..body() };
        assert!(clean(&b).unwrap_err().contains("business"));
    }

    #[test]
    fn required_fields_and_their_limits_hold() {
        assert!(clean(&SaveBody { legal_name: "   ".into(), ..body() }).is_err());
        assert!(clean(&SaveBody { registered_address: "".into(), ..body() }).is_err());
        assert!(clean(&SaveBody { legal_name: "x".repeat(MAX_NAME + 1), ..body() }).is_err());
        assert!(clean(&SaveBody { legal_name: "a\0b".into(), ..body() }).is_err());
        assert!(clean(&SaveBody { country: "Switzerland".into(), ..body() }).is_err());
    }

    #[test]
    fn bad_emails_are_refused() {
        for bad in ["", "plain", "a@b", "@example.com", "a@.com", "a@example.", "a b@example.com", "a@b@example.com"] {
            assert!(clean(&SaveBody { billing_email: bad.into(), ..body() }).is_err(), "{bad}");
        }
    }

    #[test]
    fn vat_is_optional_but_must_be_shaped_like_one() {
        assert_eq!(vat_id("").unwrap(), "");
        assert_eq!(vat_id("de 123456789").unwrap(), "DE123456789");
        assert!(vat_id("123456789").is_err());
        assert!(vat_id("DE").is_err());
        assert!(vat_id("DE12345678901234567890").is_err());
    }

    #[test]
    fn every_document_due_must_be_ticked() {
        let ticked = vec!["terms".to_string()];
        assert!(newly_accepted(DOCS, &[], &ticked, true).unwrap_err().contains("DPA"));
        let both = vec!["terms".to_string(), "dpa".to_string()];
        assert_eq!(newly_accepted(DOCS, &[], &both, true).unwrap().len(), 2);
    }

    #[test]
    fn accepting_needs_the_authority_to_bind() {
        let both = vec!["terms".to_string(), "dpa".to_string()];
        assert!(newly_accepted(DOCS, &[], &both, false).is_err());
    }

    #[test]
    fn only_a_new_version_is_asked_again() {
        let current = vec![("terms".to_string(), "2".to_string()), ("dpa".to_string(), "1".to_string())];
        assert!(newly_accepted(DOCS, &current, &[], false).unwrap().is_empty());

        let old = vec![("terms".to_string(), "1".to_string()), ("dpa".to_string(), "1".to_string())];
        assert!(newly_accepted(DOCS, &old, &[], true).is_err());
        let due = newly_accepted(DOCS, &old, &["terms".to_string()], true).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].id, "terms");
    }

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL"]
    async fn records_survive_the_workspace_and_stay_with_the_company() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = Arc::new(EndpointStore::new(&url).await.expect("store"));
        let run = uuid::Uuid::new_v4().simple().to_string();
        let owner = format!("legal-{run}@example.com");
        let stranger = format!("stranger-{run}@example.com");
        let c = store.get_admin_conn().await.unwrap();

        let first = get_default_tenant(&store, &owner).await.unwrap();
        let other = format!("second-{run}");
        c.execute("INSERT INTO tenants (id, name) VALUES ($1, $1)", &[&other]).await.unwrap();
        c.execute("INSERT INTO tenant_users (tenant_id, email, role) VALUES ($1, $2, 'owner')", &[&other, &owner])
            .await
            .unwrap();

        let as_owner = |tenant: &str, entity: Option<String>, accepted: Vec<String>| SaveBody {
            email: owner.clone(),
            tenant_id: tenant.to_string(),
            entity_id: entity,
            accepted,
            ..body()
        };
        let details = clean(&body()).unwrap();

        // Nothing is written when the terms are not accepted.
        let refused = save(&store, &first.id, &first.name, &as_owner(&first.id, None, vec![]), &details).await;
        assert!(matches!(refused, Err(SaveError::Invalid(_))));
        let linked: Option<String> =
            c.query_one("SELECT legal_entity_id FROM tenants WHERE id = $1", &[&first.id]).await.unwrap().get(0);
        assert!(linked.is_none());

        let terms = vec!["terms".to_string()];
        assert!(save(&store, &first.id, &first.name, &as_owner(&first.id, None, terms.clone()), &details).await.is_ok());
        let seen = read(&store, &first.id, &first.name, &owner, true).await.unwrap();
        assert_eq!(seen["complete"], true);
        assert_eq!(seen["details"]["legal_name"], "Example SA");
        assert_eq!(seen["documents"][0]["accepted"]["by"], owner.as_str());

        // A member who cannot edit learns the state, not the contacts.
        let hidden = read(&store, &first.id, &first.name, &stranger, false).await.unwrap();
        assert_eq!(hidden["complete"], true);
        assert!(hidden["details"].is_null());

        // Later edits need no new acceptance and add no row.
        let edit = SaveBody { accepted: vec![], authority_confirmed: false, ..as_owner(&first.id, None, vec![]) };
        assert!(save(&store, &first.id, &first.name, &edit, &details).await.is_ok());

        // The second workspace joins the same company, which has already accepted.
        let elsewhere = read(&store, &other, &other, &owner, true).await.unwrap();
        let company = elsewhere["other_companies"][0]["id"].as_str().unwrap().to_string();
        let join = SaveBody { authority_confirmed: false, ..as_owner(&other, Some(company.clone()), vec![]) };
        assert!(save(&store, &other, &other, &join, &details).await.is_ok());
        assert_eq!(read(&store, &other, &other, &owner, true).await.unwrap()["complete"], true);

        // Somebody who manages no workspace of that company cannot attach to it.
        let their_own = get_default_tenant(&store, &stranger).await.unwrap();
        let grab = SaveBody { email: stranger.clone(), entity_id: Some(company.clone()), ..as_owner(&their_own.id, None, terms.clone()) };
        let grabbed = save(&store, &their_own.id, &their_own.name, &grab, &details).await;
        assert!(matches!(grabbed, Err(SaveError::NotYourCompany)));

        let rows = "SELECT COUNT(*) FROM legal_acceptances WHERE legal_entity_id = $1";
        assert_eq!(c.query_one(rows, &[&company]).await.unwrap().get::<_, i64>(0), 1);

        // Deleting the workspaces leaves the contract records.
        for tenant in [first.id.as_str(), other.as_str()] {
            c.execute("DELETE FROM tenant_users WHERE tenant_id = $1", &[&tenant]).await.unwrap();
            c.execute("DELETE FROM tenants WHERE id = $1", &[&tenant]).await.unwrap();
        }
        assert_eq!(c.query_one(rows, &[&company]).await.unwrap().get::<_, i64>(0), 1);
        let kept = "SELECT COUNT(*) FROM legal_entities WHERE id = $1";
        assert_eq!(c.query_one(kept, &[&company]).await.unwrap().get::<_, i64>(0), 1);
    }
}
