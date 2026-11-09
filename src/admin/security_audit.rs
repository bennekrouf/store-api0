// src/admin/security_audit.rs
//
// A workspace's own security audit, run by its owners and admins.
//
//   GET /api/user/security-audit?email=…
//     → { tenant_id, tenant_name, run_by, generated_at, checks: [Check], probes: [Probe] }
//
// `evaluate` holds every rule and touches no database, so each rule is tested on
// its own. `probes` lists one address per application for the gateway, which adds
// the checks that need a network call and never passes the list on.
//
// A check is green only when the rule was really tested. Nothing to test is
// "na", and something only the workspace can judge is "info".

use crate::admin::llm_regions::Region;
use crate::admin::tenant_overview::{first_party_hosts, host_of, is_first_party, tools_of, ToolRow};
use crate::admin::tenant_region::{resolve, AssistantRegion};
use crate::app_log;
use crate::endpoint_store::tenant_management::get_default_tenant;
use crate::endpoint_store::EndpointStore;
use crate::infra::secret_box;
use crate::middleware::internal_secret::require_internal_secret;
use crate::payment::tenant_billing::can_edit;
use actix_web::{web, HttpRequest, HttpResponse, Responder};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

const SHARED_MODES: &[&str] = &["static_bearer", "header_injection", "google_sa", "client_credentials"];
const SECRET_PARAMS: &[&str] =
    &["token", "access_token", "api_key", "apikey", "key", "secret", "password", "pwd", "code", "sig"];
const MAX_PROBES: usize = 8;
const STALE_KEY_DAYS: i32 = 90;
const STALE_INVITE_DAYS: i32 = 30;

const ACCESS: &str = "Who can get in";
const CONNECTIONS: &str = "How api0 reaches your applications";
const TOOLS: &str = "What your tools expose";
const SECRETS: &str = "Secrets and keys";
const AI: &str = "AI and data";

const ACCESS_TAB: &str = "/?tab=access";
const TOOLS_TAB: &str = "/?tab=tools";
const CHANNELS_TAB: &str = "/?tab=channels";

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Pass,
    Action,
    Info,
    Na,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Severity {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize)]
struct Check {
    id: &'static str,
    section: &'static str,
    title: &'static str,
    status: Status,
    severity: Option<Severity>,
    detail: String,
    action: Option<String>,
    items: Vec<String>,
    link: Option<&'static str>,
}

impl Check {
    fn new(id: &'static str, section: &'static str, title: &'static str) -> Self {
        Check { id, section, title, status: Status::Na, severity: None, detail: String::new(), action: None, items: vec![], link: None }
    }
    fn pass(mut self, detail: impl Into<String>) -> Self {
        self.status = Status::Pass;
        self.detail = detail.into();
        self
    }
    fn na(mut self, detail: impl Into<String>) -> Self {
        self.status = Status::Na;
        self.detail = detail.into();
        self
    }
    fn info(mut self, detail: impl Into<String>) -> Self {
        self.status = Status::Info;
        self.detail = detail.into();
        self
    }
    fn act(mut self, severity: Severity, detail: impl Into<String>, action: impl Into<String>) -> Self {
        self.status = Status::Action;
        self.severity = Some(severity);
        self.detail = detail.into();
        self.action = Some(action.into());
        self
    }
    fn items(mut self, items: Vec<String>) -> Self {
        self.items = items;
        self
    }
    fn link(mut self, link: &'static str) -> Self {
        self.link = Some(link);
        self
    }
}

struct Connection {
    name: String,
    mode: String,
    oauth_urls: Vec<String>,
}

enum RegionState {
    NotChosen,
    Live(&'static Region),
    Unavailable(&'static Region),
}

struct Facts {
    has_idp: bool,
    idp_issuer: Option<String>,
    has_google_client: bool,
    open_sign_in: bool,
    editors: i64,
    stale_invites: Vec<String>,
    connections: Vec<Connection>,
    tools: Vec<ToolRow>,
    own_hosts: Vec<String>,
    encryption_ready: bool,
    unencrypted_secrets: i64,
    former_with_credentials: Vec<String>,
    former_with_links: Vec<String>,
    expired_credentials: Vec<String>,
    stale_keys: Vec<String>,
    /// Connected accounts as (api0 email, account named by the provider).
    accounts: Vec<(String, Option<String>)>,
    channels: i64,
    region: RegionState,
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", n, if n == 1 { one } else { many })
}

fn is_external(tool: &ToolRow, own: &[String]) -> bool {
    host_of(&tool.url).is_some_and(|h| !is_first_party(&h, own))
}

fn is_write(tool: &ToolRow) -> bool {
    !matches!(tool.verb.as_deref().map(str::to_uppercase).as_deref(), Some("GET" | "HEAD"))
}

/// A Microsoft address that accepts people from any directory.
fn is_multi_tenant_microsoft(url: &str) -> bool {
    let u = url.to_lowercase();
    ["/common", "/organizations", "/consumers"]
        .iter()
        .any(|p| u.contains(&format!("login.microsoftonline.com{p}")))
}

/// A credential written into a URL, where every log along the way can read it.
fn has_secret_in_url(url: &str) -> bool {
    let Some((_, query)) = url.split_once('?') else { return false };
    query.split('&').any(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        SECRET_PARAMS.contains(&key.to_lowercase().as_str()) && !value.is_empty() && !value.starts_with('{')
    })
}

fn evaluate(f: &Facts) -> Vec<Check> {
    let external: Vec<&ToolRow> = f.tools.iter().filter(|t| is_external(t, &f.own_hosts)).collect();
    let shared: Vec<String> = f
        .connections
        .iter()
        .filter(|c| SHARED_MODES.contains(&c.mode.as_str()))
        .map(|c| c.name.clone())
        .chain(f.tools.iter().filter(|t| t.has_static_headers).map(|t| format!("{} (fixed headers)", t.name)))
        .collect();

    vec![
        sign_in(f, &shared),
        editors(f),
        invites(f),
        former_people(f),
        per_person_access(f, &external, &shared),
        microsoft_single_directory(f),
        accounts_match(f),
        tools_https(&external),
        secrets_in_urls(&f.tools),
        identity_headers(&external),
        write_tools(&f.tools),
        encrypted(f),
        expired_credentials(f),
        stale_keys(f),
        ai_region(f),
        retention(),
    ]
}

fn sign_in(f: &Facts, shared: &[String]) -> Check {
    let c = Check::new("sign_in", ACCESS, "Sign-in is limited to your people").link(ACCESS_TAB);
    if f.open_sign_in && !shared.is_empty() {
        return c
            .act(
                Severity::High,
                "Any api0 account can connect to this workspace, and it holds a shared credential. Whoever connects uses it.",
                "Set up your identity provider, or give each person their own credential.",
            )
            .items(shared.to_vec());
    }
    if f.open_sign_in {
        return c.act(
            Severity::Medium,
            "Any api0 account can connect to this workspace.",
            "Set up your identity provider so that only your directory can sign in.",
        );
    }
    if f.has_idp {
        return c.pass("People sign in through your own identity provider.");
    }
    if f.has_google_client {
        return c.pass("People sign in through your own Google client.");
    }
    c.na("No sign-in method is set up, so nobody can connect from an AI client yet.")
}

fn editors(f: &Facts) -> Check {
    let c = Check::new("editors", ACCESS, "More than one person can administer the workspace").link(ACCESS_TAB);
    if f.editors >= 2 {
        c.pass(format!("{} owners or admins.", f.editors))
    } else {
        c.act(
            Severity::Low,
            "Only one person is owner or admin. If that account is lost, nobody can change this workspace.",
            "Add a second owner or admin.",
        )
    }
}

fn invites(f: &Facts) -> Check {
    let c = Check::new("invites", ACCESS, "No old invitations are still open");
    if f.stale_invites.is_empty() {
        return c.pass(format!("No invitation is older than {STALE_INVITE_DAYS} days."));
    }
    c.act(
        Severity::Low,
        format!("{} older than {STALE_INVITE_DAYS} days.", plural(f.stale_invites.len(), "invitation is", "invitations are")),
        "Cancel the invitations that are no longer needed.",
    )
    .items(f.stale_invites.clone())
    .link(ACCESS_TAB)
}

fn former_people(f: &Facts) -> Check {
    let c = Check::new("former_people", ACCESS, "Only members hold credentials or linked chats");
    let mut items: Vec<String> = f.former_with_credentials.iter().map(|e| format!("{e}: stored credential")).collect();
    items.extend(f.former_with_links.iter().map(|e| format!("{e}: linked chat account")));
    if items.is_empty() {
        return c.pass("Everyone holding a credential or a linked chat is a member.");
    }
    c.act(
        Severity::High,
        format!("{} not in the member list.", plural(items.len(), "access belongs to someone", "accesses belong to people")),
        "Unlink these chat accounts and remove these credentials, or add the people back as members.",
    )
    .items(items)
    .link(ACCESS_TAB)
}

fn per_person_access(f: &Facts, external: &[&ToolRow], shared: &[String]) -> Check {
    let c = Check::new("per_person", CONNECTIONS, "Each person reaches your application as themselves").link(ACCESS_TAB);
    if external.is_empty() {
        return c.na("No tool calls an application outside api0.");
    }
    if !shared.is_empty() {
        return c
            .act(
                Severity::Medium,
                "These use one credential for everyone, so your application cannot apply each person's rights.",
                "Use per-person sign-in (OAuth) where your application supports it, and limit the shared credential to read-only access.",
            )
            .items(shared.to_vec());
    }
    let personal = f.connections.iter().any(|c| matches!(c.mode.as_str(), "per_user_oauth" | "per_user"));
    if personal {
        return c.pass("Calls carry the credential of the person asking.");
    }
    c.act(
        Severity::Medium,
        "Your tools call your application without any credential.",
        "Require a credential on your application and set it up in Access.",
    )
}

fn microsoft_single_directory(f: &Facts) -> Check {
    let c = Check::new("microsoft_directory", CONNECTIONS, "Microsoft sign-in is limited to your directory").link(ACCESS_TAB);
    let mut microsoft = false;
    let mut open: Vec<String> = vec![];
    for conn in &f.connections {
        if conn.oauth_urls.iter().any(|u| u.to_lowercase().contains("login.microsoftonline.com")) {
            microsoft = true;
        }
        if conn.oauth_urls.iter().any(|u| is_multi_tenant_microsoft(u)) {
            open.push(conn.name.clone());
        }
    }
    if let Some(issuer) = &f.idp_issuer {
        if issuer.to_lowercase().contains("login.microsoftonline.com") {
            microsoft = true;
        }
        if is_multi_tenant_microsoft(issuer) {
            open.push("Workspace sign-in".to_string());
        }
    }
    if !microsoft {
        return c.na("This workspace does not use Microsoft sign-in.");
    }
    if open.is_empty() {
        return c.pass("Addresses name your own directory.");
    }
    c.act(
        Severity::High,
        "These accept accounts from any Microsoft directory.",
        "Replace /common or /organizations in the address with your directory (tenant) ID.",
    )
    .items(open)
}

fn accounts_match(f: &Facts) -> Check {
    let c = Check::new("accounts_match", CONNECTIONS, "Connected accounts are known and expected").link(ACCESS_TAB);
    if f.accounts.is_empty() {
        return c.na("Nobody has connected a personal account.");
    }
    let unknown: Vec<String> =
        f.accounts.iter().filter(|(_, a)| a.is_none()).map(|(e, _)| format!("{e}: account not recorded")).collect();
    if !unknown.is_empty() {
        return c
            .act(
                Severity::Low,
                "These connections predate account recording, so api0 cannot show whose account they use.",
                "Ask these people to reconnect.",
            )
            .items(unknown);
    }
    let different: Vec<String> = f
        .accounts
        .iter()
        .filter_map(|(e, a)| a.as_ref().filter(|a| !a.eq_ignore_ascii_case(e)).map(|a| format!("{e} connects as {a}")))
        .collect();
    if different.is_empty() {
        return c.pass(format!("{} use the same address as in api0.", plural(f.accounts.len(), "connected account", "connected accounts")));
    }
    c.info("These people act under a different address in your application. Check that each one is expected.")
        .items(different)
}

fn tools_https(external: &[&ToolRow]) -> Check {
    let c = Check::new("tools_https", TOOLS, "Tools only call HTTPS addresses").link(TOOLS_TAB);
    if external.is_empty() {
        return c.na("No tool calls an application outside api0.");
    }
    let plain: Vec<String> = external
        .iter()
        .filter(|t| t.url.trim().to_lowercase().starts_with("http://"))
        .map(|t| format!("{} ({})", t.name, host_of(&t.url).unwrap_or_default()))
        .collect();
    if plain.is_empty() {
        return c.pass(format!("{} checked.", plural(external.len(), "tool", "tools")));
    }
    c.act(
        Severity::High,
        "These tools send credentials and data without encryption.",
        "Change their addresses to https://.",
    )
    .items(plain)
}

fn secrets_in_urls(tools: &[ToolRow]) -> Check {
    let c = Check::new("secrets_in_urls", TOOLS, "No secret is written in a tool address").link(TOOLS_TAB);
    if tools.is_empty() {
        return c.na("This workspace has no tools.");
    }
    let found: Vec<String> = tools.iter().filter(|t| has_secret_in_url(&t.url)).map(|t| t.name.clone()).collect();
    if found.is_empty() {
        return c.pass(format!("{} checked.", plural(tools.len(), "tool address", "tool addresses")));
    }
    c.act(
        Severity::High,
        "A key in an address ends up in server and proxy logs.",
        "Change the key at its source, then send the new one in a header from Access.",
    )
    .items(found)
}

fn identity_headers(external: &[&ToolRow]) -> Check {
    let c = Check::new("identity_headers", TOOLS, "Caller details go only where needed").link(TOOLS_TAB);
    if external.is_empty() {
        return c.na("No tool calls an application outside api0.");
    }
    let sending: Vec<String> = external
        .iter()
        .filter(|t| t.forward)
        .map(|t| format!("{} ({})", t.name, host_of(&t.url).unwrap_or_default()))
        .collect();
    if sending.is_empty() {
        return c.pass("No tool sends the caller's email to an outside application.");
    }
    c.act(
        Severity::Medium,
        "These tools send the caller's email and workspace identifiers to the application.",
        "If the application does not use them, turn off identity forwarding on these tools.",
    )
    .items(sending)
}

fn write_tools(tools: &[ToolRow]) -> Check {
    let c = Check::new("write_tools", TOOLS, "Tools that can change data").link(TOOLS_TAB);
    if tools.is_empty() {
        return c.na("This workspace has no tools.");
    }
    let writing: Vec<String> = tools.iter().filter(|t| is_write(t)).map(|t| t.name.clone()).collect();
    if writing.is_empty() {
        return c.pass("All tools only read.");
    }
    c.info("The AI model decides when to call these tools. Keep only the ones people need.").items(writing)
}

fn encrypted(f: &Facts) -> Check {
    let c = Check::new("encrypted", SECRETS, "Stored secrets are encrypted");
    if !f.encryption_ready {
        return c.act(
            Severity::High,
            "Encryption is not switched on for this api0 service.",
            "Contact api0 support.",
        );
    }
    if f.unencrypted_secrets > 0 {
        return c.act(
            Severity::High,
            format!("{} still stored without encryption.", plural(f.unencrypted_secrets as usize, "secret is", "secrets are")),
            "Contact api0 support to finish encrypting them.",
        );
    }
    c.pass("Every secret saved for this workspace is encrypted (AES-256-GCM).")
}

fn expired_credentials(f: &Facts) -> Check {
    let c = Check::new("expired_credentials", SECRETS, "No expired personal token is kept").link(ACCESS_TAB);
    if f.expired_credentials.is_empty() {
        return c.pass("No stored personal token is past its expiry date.");
    }
    c.act(
        Severity::Low,
        "These tokens have expired and no longer work.",
        "Ask these people to replace or remove them.",
    )
    .items(f.expired_credentials.clone())
}

fn stale_keys(f: &Facts) -> Check {
    let c = Check::new("stale_keys", SECRETS, "No unused API key is left active");
    if f.stale_keys.is_empty() {
        return c.pass(format!("Every active key was used in the last {STALE_KEY_DAYS} days."));
    }
    c.act(
        Severity::Low,
        format!("{} not been used for {STALE_KEY_DAYS} days.", plural(f.stale_keys.len(), "active key has", "active keys have")),
        "Revoke the keys that are no longer needed.",
    )
    .items(f.stale_keys.clone())
}

fn ai_region(f: &Facts) -> Check {
    let c = Check::new("ai_region", AI, "You chose where your messages are processed").link(CHANNELS_TAB);
    if f.channels == 0 {
        return c.na("No messaging channel is connected. With an AI client of your own, that client's model reads the data.");
    }
    match f.region {
        RegionState::Live(r) => c.pass(format!("Messaging assistant runs in: {}.", r.label)),
        RegionState::Unavailable(r) => c.act(
            Severity::High,
            format!("{} is not on offer right now, so the assistant is not answering.", r.label),
            "Choose another region, or wait for this one to come back.",
        ),
        RegionState::NotChosen => c.act(
            Severity::Medium,
            "Messages, and what your tools return, go to api0's default AI provider.",
            "Choose an AI region in the Channels tab.",
        ),
    }
}

fn retention() -> Check {
    Check::new("retention", AI, "How long conversations are kept").info(
        "Assistant conversations are deleted after 30 days without activity, and messages that failed after 90 days. Treat both as able to contain what your tools returned.",
    )
}

/// One tool per outside application that can be called with nothing filled in.
fn probes(tools: &[ToolRow], own: &[String]) -> Vec<serde_json::Value> {
    let mut seen = HashSet::new();
    tools
        .iter()
        .filter(|t| is_external(t, own) && !is_write(t) && !t.url.contains('{'))
        .filter(|t| seen.insert(host_of(&t.url)))
        .take(MAX_PROBES)
        .map(|t| serde_json::json!({ "tool": t.name, "url": t.url }))
        .collect()
}

async fn strings(
    client: &deadpool_postgres::Object,
    sql: &str,
    tenant: &str,
) -> Result<Vec<String>, tokio_postgres::Error> {
    Ok(client.query(sql, &[&tenant]).await?.iter().map(|r| r.get(0)).collect())
}

async fn collect(store: &EndpointStore, tenant: &str) -> Result<Facts, String> {
    let client = store.get_admin_conn().await.map_err(|e| e.to_string())?;
    let e = |e: tokio_postgres::Error| e.to_string();

    let t = client
        .query_one(
            "SELECT idp_issuer, idp_client_id, idp_client_secret IS NOT NULL, google_client_id, allow_api0_signin
               FROM tenants WHERE id = $1",
            &[&tenant],
        )
        .await
        .map_err(e)?;
    let filled = |v: Option<String>| v.filter(|s| !s.trim().is_empty());
    let issuer = filled(t.get(0));
    let client_id = filled(t.get(1));
    let has_secret: bool = t.get(2);
    let has_idp = issuer.is_some() && client_id.is_some() && has_secret;

    let editors: i64 = client
        .query_one("SELECT count(*) FROM tenant_users WHERE tenant_id = $1 AND role IN ('owner','admin')", &[&tenant])
        .await
        .map_err(e)?
        .get(0);

    let stale_invites = strings(
        &client,
        &format!("SELECT email FROM tenant_invites WHERE tenant_id = $1 AND created_at < NOW() - INTERVAL '{STALE_INVITE_DAYS} days' ORDER BY email"),
        tenant,
    )
    .await
    .map_err(e)?;

    let connections = client
        .query(
            "SELECT connection_name, auth_mode, oauth_authorize_url, oauth_token_url
               FROM tenant_downstream_auth WHERE tenant_id = $1 ORDER BY connection_name",
            &[&tenant],
        )
        .await
        .map_err(e)?
        .iter()
        .map(|r| Connection {
            name: r.get(0),
            mode: r.get(1),
            oauth_urls: [r.get::<_, Option<String>>(2), r.get::<_, Option<String>>(3)].into_iter().flatten().collect(),
        })
        .collect();

    let unencrypted_secrets: i64 = client
        .query_one(
            "SELECT
               (SELECT count(*) FROM tenant_downstream_auth WHERE tenant_id = $1
                 AND (bearer_token IS NOT NULL OR custom_headers IS NOT NULL OR service_account_json IS NOT NULL))
             + (SELECT count(*) FROM mcp_tools WHERE tenant_id = $1 AND static_headers IS NOT NULL)
             + (SELECT count(*) FROM whatsapp_channels WHERE tenant_id = $1
                 AND (wa_token IS NOT NULL OR verify_token IS NOT NULL OR app_secret IS NOT NULL))
             + (SELECT count(*) FROM messaging_channels WHERE tenant_id = $1
                 AND webhook_secret IS NOT NULL AND webhook_secret <> '')",
            &[&tenant],
        )
        .await
        .map_err(e)?
        .get(0);

    let former_with_credentials = strings(
        &client,
        "SELECT DISTINCT c.user_email FROM user_downstream_credentials c
          WHERE c.tenant_id = $1
            AND NOT EXISTS (SELECT 1 FROM tenant_users u
                             WHERE u.tenant_id = c.tenant_id AND LOWER(u.email) = LOWER(c.user_email))
          ORDER BY 1",
        tenant,
    )
    .await
    .map_err(e)?;

    let former_with_links = strings(
        &client,
        "SELECT DISTINCT c.user_email FROM channel_identities c
          WHERE c.tenant_id = $1
            AND NOT EXISTS (SELECT 1 FROM tenant_users u
                             WHERE u.tenant_id = c.tenant_id AND LOWER(u.email) = LOWER(c.user_email))
          ORDER BY 1",
        tenant,
    )
    .await
    .map_err(e)?;

    let expired_credentials = strings(
        &client,
        "SELECT DISTINCT user_email FROM user_downstream_credentials
          WHERE tenant_id = $1 AND split_part(kind, '@', 1) = 'pat' AND expires_at < NOW()
          ORDER BY 1",
        tenant,
    )
    .await
    .map_err(e)?;

    let stale_keys = strings(
        &client,
        &format!(
            "SELECT key_name || ' (' || key_prefix || '…)' FROM api_keys
              WHERE tenant_id = $1 AND is_active
                AND (expires_at IS NULL OR expires_at > NOW())
                AND COALESCE(last_used, generated_at) < NOW() - INTERVAL '{STALE_KEY_DAYS} days'
              ORDER BY 1"
        ),
        tenant,
    )
    .await
    .map_err(e)?;

    // An OAuth connection has a refresh row and an access row; the account name
    // may be on either.
    let accounts = client
        .query(
            "SELECT r.user_email, COALESCE(a.verified_identity, r.verified_identity)
               FROM user_downstream_credentials r
               LEFT JOIN user_downstream_credentials a
                 ON a.tenant_id = r.tenant_id AND a.user_email = r.user_email
                AND a.kind = replace(r.kind, 'oauth_refresh', 'oauth_access')
              WHERE r.tenant_id = $1 AND split_part(r.kind, '@', 1) = 'oauth_refresh'
              ORDER BY 1",
            &[&tenant],
        )
        .await
        .map_err(e)?
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();

    let channels: i64 = client
        .query_one(
            "SELECT (SELECT count(*) FROM whatsapp_channels WHERE tenant_id = $1)
                  + (SELECT count(*) FROM messaging_channels WHERE tenant_id = $1)",
            &[&tenant],
        )
        .await
        .map_err(e)?
        .get(0);

    let tools = tools_of(&client, Some(tenant)).await.map_err(e)?;

    let region = match resolve(store, tenant).await? {
        AssistantRegion::NotChosen => RegionState::NotChosen,
        AssistantRegion::Live { region, .. } => RegionState::Live(region),
        AssistantRegion::Unavailable(region) => RegionState::Unavailable(region),
    };

    Ok(Facts {
        has_idp,
        idp_issuer: issuer,
        has_google_client: filled(t.get(3)).is_some(),
        open_sign_in: t.get(4),
        editors,
        stale_invites,
        connections,
        tools,
        own_hosts: first_party_hosts(),
        encryption_ready: secret_box::is_configured(),
        unencrypted_secrets,
        former_with_credentials,
        former_with_links,
        expired_credentials,
        stale_keys,
        accounts,
        channels,
        region,
    })
}

#[derive(Deserialize)]
pub struct AuditQuery {
    pub email: String,
}

pub async fn run_audit_handler(
    req: HttpRequest,
    store: web::Data<Arc<EndpointStore>>,
    query: web::Query<AuditQuery>,
) -> impl Responder {
    if let Some(deny) = require_internal_secret(&req) {
        return deny;
    }
    let failed = |what: &str, e: String| {
        app_log!(error, error = %e, "Security audit: {}", what);
        HttpResponse::InternalServerError()
            .json(serde_json::json!({ "success": false, "error": "The audit could not be completed" }))
    };
    let tenant = match get_default_tenant(&store, &query.email).await {
        Ok(t) => t,
        Err(e) => return failed("find the tenant", e.to_string()),
    };
    match can_edit(&store, &tenant.id, &query.email).await {
        Ok(true) => {}
        Ok(false) => {
            return HttpResponse::Forbidden().json(serde_json::json!({
                "success": false,
                "error": "Only the workspace's owners and admins can run a security audit",
            }))
        }
        Err(e) => return failed("read the caller's role", e),
    }
    let facts = match collect(&store, &tenant.id).await {
        Ok(f) => f,
        Err(e) => return failed("read the workspace", e),
    };
    let checks = evaluate(&facts);
    app_log!(info, tenant_id = %tenant.id, email = %query.email, checks = checks.len(), "Security audit run");
    HttpResponse::Ok().json(serde_json::json!({
        "success": true,
        "tenant_id": tenant.id,
        "tenant_name": tenant.name,
        "run_by": query.email,
        "generated_at": chrono::Utc::now().to_rfc3339(),
        "checks": checks,
        "probes": probes(&facts.tools, &facts.own_hosts),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own() -> Vec<String> {
        vec!["api0.ai".to_string()]
    }

    fn tool(name: &str, url: &str) -> ToolRow {
        ToolRow {
            name: name.into(),
            url: url.into(),
            forward: false,
            verb: Some("GET".into()),
            has_static_headers: false,
        }
    }

    /// A workspace with nothing wrong.
    fn clean() -> Facts {
        Facts {
            has_idp: true,
            idp_issuer: Some("https://login.microsoftonline.com/1111-2222/v2.0".into()),
            has_google_client: false,
            open_sign_in: false,
            editors: 2,
            stale_invites: vec![],
            connections: vec![Connection {
                name: "default".into(),
                mode: "per_user_oauth".into(),
                oauth_urls: vec!["https://login.microsoftonline.com/1111-2222/oauth2/v2.0/token".into()],
            }],
            tools: vec![tool("orders", "https://app.example.com/orders")],
            own_hosts: own(),
            encryption_ready: true,
            unencrypted_secrets: 0,
            former_with_credentials: vec![],
            former_with_links: vec![],
            expired_credentials: vec![],
            stale_keys: vec![],
            accounts: vec![("a@example.com".into(), Some("A@example.com".into()))],
            channels: 1,
            region: RegionState::Live(&crate::admin::llm_regions::REGIONS[0]),
        }
    }

    fn check(f: &Facts, id: &str) -> Check {
        evaluate(f).into_iter().find(|c| c.id == id).unwrap_or_else(|| panic!("no check {id}"))
    }

    #[test]
    fn a_clean_workspace_has_nothing_to_do() {
        let actions: Vec<_> = evaluate(&clean()).into_iter().filter(|c| c.status == Status::Action).map(|c| c.id).collect();
        assert!(actions.is_empty(), "{actions:?}");
    }

    #[test]
    fn every_action_says_what_to_do_and_how_urgent() {
        let mut f = clean();
        f.open_sign_in = true;
        f.editors = 1;
        f.region = RegionState::NotChosen;
        f.unencrypted_secrets = 2;
        for c in evaluate(&f).into_iter().filter(|c| c.status == Status::Action) {
            assert!(c.action.is_some() && c.severity.is_some(), "{}", c.id);
        }
    }

    #[test]
    fn open_sign_in_is_worse_with_a_shared_credential() {
        let mut f = clean();
        f.open_sign_in = true;
        assert_eq!(check(&f, "sign_in").severity, Some(Severity::Medium));
        f.connections[0].mode = "static_bearer".into();
        assert_eq!(check(&f, "sign_in").severity, Some(Severity::High));
    }

    #[test]
    fn a_shared_credential_is_flagged_and_fixed_headers_count() {
        let mut f = clean();
        f.connections[0].mode = "client_credentials".into();
        assert_eq!(check(&f, "per_person").status, Status::Action);

        let mut f = clean();
        f.tools[0].has_static_headers = true;
        let c = check(&f, "per_person");
        assert_eq!(c.status, Status::Action);
        assert_eq!(c.items, vec!["orders (fixed headers)".to_string()]);
    }

    #[test]
    fn first_party_tools_do_not_count_as_outside_applications() {
        let mut f = clean();
        f.tools = vec![tool("me", "https://gateway.api0.ai/me")];
        for id in ["per_person", "tools_https", "identity_headers"] {
            assert_eq!(check(&f, id).status, Status::Na, "{id}");
        }
    }

    #[test]
    fn microsoft_addresses_open_to_any_directory_are_flagged() {
        let mut f = clean();
        f.connections[0].oauth_urls = vec!["https://login.microsoftonline.com/common/oauth2/v2.0/token".into()];
        let c = check(&f, "microsoft_directory");
        assert_eq!(c.severity, Some(Severity::High));
        assert_eq!(c.items, vec!["default".to_string()]);

        let mut f = clean();
        f.connections[0].oauth_urls = vec!["https://accounts.google.com/o/oauth2/token".into()];
        f.idp_issuer = None;
        assert_eq!(check(&f, "microsoft_directory").status, Status::Na);
    }

    #[test]
    fn people_outside_the_member_list_are_listed() {
        let mut f = clean();
        f.former_with_links = vec!["gone@example.com".into()];
        let c = check(&f, "former_people");
        assert_eq!(c.severity, Some(Severity::High));
        assert_eq!(c.items, vec!["gone@example.com: linked chat account".to_string()]);
    }

    #[test]
    fn a_different_connected_address_is_for_review_not_an_error() {
        let mut f = clean();
        f.accounts = vec![("admin@gmail.com".into(), Some("demo@contoso.onmicrosoft.com".into()))];
        assert_eq!(check(&f, "accounts_match").status, Status::Info);
        f.accounts = vec![("admin@gmail.com".into(), None)];
        assert_eq!(check(&f, "accounts_match").status, Status::Action);
    }

    #[test]
    fn plain_http_is_flagged() {
        let mut f = clean();
        f.tools = vec![tool("orders", "http://app.example.com/orders")];
        assert_eq!(check(&f, "tools_https").severity, Some(Severity::High));
    }

    #[test]
    fn a_key_in_an_address_is_found_and_a_placeholder_is_not() {
        assert!(has_secret_in_url("https://f.azurewebsites.net/api/x?code=abc123"));
        assert!(has_secret_in_url("https://a.example.com/x?a=1&API_KEY=z"));
        assert!(!has_secret_in_url("https://a.example.com/x?code={code}"));
        assert!(!has_secret_in_url("https://a.example.com/x?page=2"));
        assert!(!has_secret_in_url("https://a.example.com/token"));
    }

    #[test]
    fn identity_forwarding_to_an_outside_application_is_flagged() {
        let mut f = clean();
        f.tools[0].forward = true;
        assert_eq!(check(&f, "identity_headers").status, Status::Action);
    }

    #[test]
    fn a_tool_without_a_verb_counts_as_writing() {
        let mut f = clean();
        f.tools[0].verb = None;
        assert_eq!(check(&f, "write_tools").status, Status::Info);
        f.tools[0].verb = Some("get".into());
        assert_eq!(check(&f, "write_tools").status, Status::Pass);
    }

    #[test]
    fn the_ai_region_only_matters_with_a_channel() {
        let mut f = clean();
        f.region = RegionState::NotChosen;
        assert_eq!(check(&f, "ai_region").severity, Some(Severity::Medium));
        f.region = RegionState::Unavailable(&crate::admin::llm_regions::REGIONS[0]);
        assert_eq!(check(&f, "ai_region").severity, Some(Severity::High));
        f.channels = 0;
        assert_eq!(check(&f, "ai_region").status, Status::Na);
    }

    #[test]
    fn unencrypted_secrets_are_never_green() {
        let mut f = clean();
        f.unencrypted_secrets = 1;
        assert_eq!(check(&f, "encrypted").status, Status::Action);
        let mut f = clean();
        f.encryption_ready = false;
        assert_eq!(check(&f, "encrypted").status, Status::Action);
    }

    #[test]
    fn probes_take_one_readable_address_per_outside_application() {
        let mut write = tool("create", "https://w.example.com/x");
        write.verb = Some("POST".into());
        let tools = vec![
            tool("a", "https://app.example.com/a"),
            tool("b", "https://app.example.com/b"),
            tool("needs-id", "https://other.example.com/{id}"),
            tool("own", "https://gateway.api0.ai/x"),
            write,
        ];
        let p = probes(&tools, &own());
        assert_eq!(p.len(), 1);
        assert_eq!(p[0]["tool"], "a");
    }

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL and API0_ENCRYPTION_KEY"]
    async fn the_audit_reads_a_real_workspace() {
        let url = std::env::var("TEST_DATABASE_URL").expect("TEST_DATABASE_URL");
        let store = Arc::new(EndpointStore::new(&url).await.expect("store"));
        let run = uuid::Uuid::new_v4().simple().to_string();
        let email = format!("audit-{run}@example.com");
        let tenant = get_default_tenant(&store, &email).await.unwrap();
        let t = tenant.id.as_str();
        let c = store.get_admin_conn().await.unwrap();

        // A credential held by someone who is not a member, and a legacy secret.
        c.execute(
            "INSERT INTO user_downstream_credentials (tenant_id, user_email, kind, secret) VALUES ($1, 'gone@example.com', 'pat', '\\x00')",
            &[&t],
        ).await.unwrap();
        c.execute(
            "INSERT INTO tenant_downstream_auth (tenant_id, auth_mode, bearer_token) VALUES ($1, 'static_bearer', 'old')
             ON CONFLICT DO NOTHING",
            &[&t],
        ).await.unwrap();

        let facts = collect(&store, t).await.expect("facts");
        assert_eq!(facts.former_with_credentials, vec!["gone@example.com".to_string()]);
        assert_eq!(facts.unencrypted_secrets, 1);
        assert_eq!(facts.connections.len(), 1);
        assert!(!evaluate(&facts).is_empty());
    }
}
