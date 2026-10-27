-- PostgreSQL Schema

-- Keep user_preferences table for preferences and credit
CREATE TABLE IF NOT EXISTS user_preferences (
    email VARCHAR NOT NULL,
    hidden_defaults TEXT NOT NULL DEFAULT '',
    credit_balance BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (email)
);

-- Tenants table
CREATE TABLE IF NOT EXISTS tenants (
    id VARCHAR PRIMARY KEY,
    name VARCHAR NOT NULL,
    credit_balance BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Tenant Users table (User-Tenant relationship)
CREATE TABLE IF NOT EXISTS tenant_users (
    tenant_id VARCHAR NOT NULL,
    email VARCHAR NOT NULL,
    role VARCHAR NOT NULL DEFAULT 'member', -- owner, member
    PRIMARY KEY (tenant_id, email),
    FOREIGN KEY (tenant_id) REFERENCES tenants(id),
    FOREIGN KEY (email) REFERENCES user_preferences(email)
);

-- Add default_tenant_id to user_preferences
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'user_preferences' AND column_name = 'default_tenant_id') THEN
        ALTER TABLE user_preferences ADD COLUMN default_tenant_id VARCHAR;
        -- We cannot easily FK to tenants here if we want to circular reference safely, but optional
        -- ALTER TABLE user_preferences ADD CONSTRAINT fk_default_tenant FOREIGN KEY (default_tenant_id) REFERENCES tenants(id);
    END IF;
END $$;

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
    suggested_sentence VARCHAR NOT NULL DEFAULT '',
    group_id VARCHAR,
    content_type VARCHAR DEFAULT NULL,
    body_template TEXT DEFAULT NULL,
    forward_identity BOOLEAN DEFAULT NULL,
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

-- Add tenant_id to appropriate tables
DO $$
BEGIN
    -- api_keys
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'api_keys' AND column_name = 'tenant_id') THEN
        ALTER TABLE api_keys ADD COLUMN tenant_id VARCHAR;
        CREATE INDEX idx_api_keys_tenant_id ON api_keys(tenant_id);
    END IF;

    -- api_groups
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'api_groups' AND column_name = 'tenant_id') THEN
        ALTER TABLE api_groups ADD COLUMN tenant_id VARCHAR;
        CREATE INDEX idx_api_groups_tenant_id ON api_groups(tenant_id);
    END IF;

    -- api_usage_logs: tenant_id
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'api_usage_logs' AND column_name = 'tenant_id') THEN
        ALTER TABLE api_usage_logs ADD COLUMN tenant_id VARCHAR;
        CREATE INDEX idx_usage_logs_tenant_id ON api_usage_logs(tenant_id);
    END IF;

    -- api_usage_logs: consumer_id — opaque end-consumer identifier supplied by the tenant (e.g. Firebase UID).
    -- Null when the tenant did not pass X-Consumer-Id. Never contains PII — tenant chooses the value.
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'api_usage_logs' AND column_name = 'consumer_id') THEN
        ALTER TABLE api_usage_logs ADD COLUMN consumer_id VARCHAR;
        CREATE INDEX idx_usage_logs_consumer_id ON api_usage_logs(consumer_id);
    END IF;
END $$;

-- Create indexes for better performance
CREATE INDEX IF NOT EXISTS idx_api_keys_email ON api_keys(email);
-- The admin user directory looks people up by email; tenant_users' primary key
-- leads with tenant_id, so without this every lookup scanned the table.
CREATE INDEX IF NOT EXISTS idx_tenant_users_email ON tenant_users(email);
-- When a user first appeared. Unknown (NULL) for users from before this column.
ALTER TABLE user_preferences ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ;
ALTER TABLE user_preferences ALTER COLUMN created_at SET DEFAULT NOW();
CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash);
CREATE INDEX IF NOT EXISTS idx_domains_email ON domains(email);
CREATE INDEX IF NOT EXISTS idx_domains_verified ON domains(verified);
CREATE INDEX IF NOT EXISTS idx_usage_logs_timestamp ON api_usage_logs(timestamp);
CREATE INDEX IF NOT EXISTS idx_usage_logs_key_id ON api_usage_logs(key_id);
CREATE INDEX IF NOT EXISTS idx_usage_logs_email ON api_usage_logs(email);

-- Reference Data table
CREATE TABLE IF NOT EXISTS reference_data (
    id VARCHAR PRIMARY KEY,
    email VARCHAR NOT NULL,
    name VARCHAR NOT NULL,
    data JSONB NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL,
    FOREIGN KEY (email) REFERENCES user_preferences(email)
);

CREATE INDEX IF NOT EXISTS idx_reference_data_email ON reference_data(email);

-- Credit transaction log: every balance change is recorded here
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'credit_transactions') THEN
        CREATE TABLE credit_transactions (
            id          BIGSERIAL PRIMARY KEY,
            tenant_id   VARCHAR NOT NULL,
            email       VARCHAR NOT NULL,
            amount      BIGINT  NOT NULL,
            balance_after BIGINT NOT NULL,
            action_type VARCHAR NOT NULL DEFAULT 'unknown',
            description TEXT,
            created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
        );
        CREATE INDEX idx_credit_tx_email      ON credit_transactions(email);
        CREATE INDEX idx_credit_tx_tenant_id  ON credit_transactions(tenant_id);
        CREATE INDEX idx_credit_tx_created_at ON credit_transactions(created_at);
    END IF;
END $$;

-- ── MCP Gateway additions ─────────────────────────────────────────────────────

-- provider_tenant_id on api_keys:
--   NULL  → regular key (tenant uses its own tools)
--   set   → consumer key (key owner is end-user; tools come from the provider tenant)
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'api_keys' AND column_name = 'provider_tenant_id'
    ) THEN
        ALTER TABLE api_keys ADD COLUMN provider_tenant_id VARCHAR REFERENCES tenants(id);
        CREATE INDEX idx_api_keys_provider_tenant ON api_keys(provider_tenant_id);
    END IF;
END $$;

-- Tool registry: each tenant registers (tool_name → backend_url) mappings.
-- UNIQUE(tenant_id, tool_name) ensures no duplicate tool names within a tenant.
CREATE TABLE IF NOT EXISTS mcp_tools (
    id              VARCHAR         PRIMARY KEY DEFAULT gen_random_uuid()::VARCHAR,
    tenant_id       VARCHAR         NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    tool_name       VARCHAR         NOT NULL,
    backend_url     VARCHAR         NOT NULL,
    description     TEXT            NOT NULL DEFAULT '',
    input_schema    TEXT            NOT NULL DEFAULT '{"type":"object","properties":{}}',
    cost_credits    BIGINT          NOT NULL DEFAULT 1,
    timeout_ms      INTEGER         NOT NULL DEFAULT 30000,
    -- When set (GET, POST, PUT, DELETE, PATCH), the gateway forwards the call as a
    -- plain REST request instead of the MCP { tool, arguments } envelope.
    -- NULL means the backend speaks MCP format natively.
    http_verb       VARCHAR         DEFAULT NULL,
    -- ── Request shaping: what lets a tool front a third-party cloud API ───────
    -- Request Content-Type. NULL → application/json. Azure DevOps work items
    -- need application/json-patch+json.
    content_type    VARCHAR         DEFAULT NULL,
    -- JSON template rendered against the call arguments, for APIs whose request
    -- body is not the MCP arguments object. NULL → send the arguments verbatim.
    -- Placeholders are {arg}; an array element that cannot render completely is
    -- dropped, an object entry that cannot render is omitted.
    body_template   TEXT            DEFAULT NULL,
    -- Constant headers for this tool (JSON object). Applied under the tenant's
    -- downstream auth, so auth always wins a collision.
    static_headers  JSONB           DEFAULT NULL,
    -- Whether to forward api0's identity headers — X-Internal-Secret,
    -- X-User-Email, X-Tenant-Id, X-Provider-Tenant-Id. TRUE for first-party
    -- backends; set FALSE on tools pointed at a third-party API so no internal
    -- secret or end-user email leaves the platform.
    forward_identity BOOLEAN        NOT NULL DEFAULT TRUE,
    is_active       BOOLEAN         NOT NULL DEFAULT TRUE,
    created_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    updated_at      TIMESTAMPTZ     NOT NULL DEFAULT NOW(),
    UNIQUE(tenant_id, tool_name)
);

-- Idempotent backfill: add http_verb if table already exists from a previous deploy
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'mcp_tools' AND column_name = 'http_verb'
    ) THEN
        ALTER TABLE mcp_tools ADD COLUMN http_verb VARCHAR DEFAULT NULL;
    END IF;
END $$;

-- Idempotent backfill: request-shaping columns on tables from an earlier deploy.
ALTER TABLE mcp_tools ADD COLUMN IF NOT EXISTS content_type     VARCHAR DEFAULT NULL;
ALTER TABLE mcp_tools ADD COLUMN IF NOT EXISTS body_template    TEXT    DEFAULT NULL;
ALTER TABLE mcp_tools ADD COLUMN IF NOT EXISTS static_headers   JSONB   DEFAULT NULL;
ALTER TABLE mcp_tools ADD COLUMN IF NOT EXISTS forward_identity BOOLEAN NOT NULL DEFAULT TRUE;
-- Tools synced from an endpoint upload, as opposed to registered by hand. An
-- upload switches off its workspace's imported tools that the new file no
-- longer contains; hand-registered tools are never touched.
ALTER TABLE mcp_tools ADD COLUMN IF NOT EXISTS from_import BOOLEAN NOT NULL DEFAULT FALSE;

-- Same request-shaping pair on endpoints, so an uploaded endpoint can describe a
-- body that is not a flat JSON object (a JSON Patch array, say). NULL keeps the
-- old behaviour: send the arguments as-is as application/json.
ALTER TABLE endpoints ADD COLUMN IF NOT EXISTS content_type  VARCHAR DEFAULT NULL;
ALTER TABLE endpoints ADD COLUMN IF NOT EXISTS body_template TEXT    DEFAULT NULL;

-- NULL means true: api0's identity headers travel with the call, as they always
-- have. Set false on anything pointed at a third-party API.
ALTER TABLE endpoints ADD COLUMN IF NOT EXISTS forward_identity BOOLEAN DEFAULT NULL;

CREATE INDEX IF NOT EXISTS idx_mcp_tools_lookup
    ON mcp_tools(tenant_id, tool_name, is_active);
CREATE INDEX IF NOT EXISTS idx_mcp_tools_tenant
    ON mcp_tools(tenant_id);

-- ── Tenant downstream auth ────────────────────────────────────────────────────
-- One row per tenant — defines how the MCP gateway authenticates against the
-- tenant's backend on every proxied call.

CREATE TABLE IF NOT EXISTS tenant_downstream_auth (
    tenant_id            VARCHAR PRIMARY KEY REFERENCES tenants(id) ON DELETE CASCADE,
    auth_mode            VARCHAR NOT NULL DEFAULT 'none',
    -- google_sa
    service_account_json TEXT    DEFAULT NULL,
    target_audience      VARCHAR DEFAULT NULL,
    -- static_bearer
    bearer_token         VARCHAR DEFAULT NULL,
    -- header_injection  (JSON object: {"Header-Name": "value", ...})
    custom_headers       JSONB   DEFAULT NULL,
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Per-provider OAuth client ID — allows each provider to have their own
-- client_id (e.g. "acme-mcp") that resolves to their provider_tenant_id.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'tenants' AND column_name = 'mcp_client_id'
    ) THEN
        ALTER TABLE tenants ADD COLUMN mcp_client_id VARCHAR UNIQUE;
        CREATE INDEX idx_tenants_mcp_client_id ON tenants(mcp_client_id);
    END IF;
END $$;

-- Per-provider Firebase config — the OAuth authorize page uses the provider's
-- own Firebase project for end-user sign-in (separate from api0's own project).
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'tenants' AND column_name = 'firebase_project_id'
    ) THEN
        ALTER TABLE tenants ADD COLUMN firebase_project_id VARCHAR;
        ALTER TABLE tenants ADD COLUMN firebase_api_key VARCHAR;
        ALTER TABLE tenants ADD COLUMN firebase_auth_domain VARCHAR;
    END IF;
END $$;

-- Google OAuth client_id — the provider's standard Google OAuth 2.0 Web Client ID.
-- The api0 authorize page uses this (via Google Identity Services) to sign in
-- end-users belonging to the provider's Google workspace / project.
-- Replaces the Firebase-specific columns; those remain but are no longer written.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'tenants' AND column_name = 'google_client_id'
    ) THEN
        ALTER TABLE tenants ADD COLUMN google_client_id VARCHAR;
    END IF;
END $$;

-- ── WhatsApp bridge ──────────────────────────────────────────────────────────

-- One row per tenant WhatsApp Business Account
CREATE TABLE IF NOT EXISTS whatsapp_channels (
    phone_number_id  VARCHAR     PRIMARY KEY,   -- Meta phone number ID
    tenant_id        VARCHAR     NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    wa_token         TEXT        NOT NULL,       -- Meta permanent token
    verify_token     VARCHAR     NOT NULL,       -- webhook verify token
    system_prompt    TEXT        NOT NULL DEFAULT '',
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(tenant_id)
);

-- Conversation history per (tenant, customer phone)
CREATE TABLE IF NOT EXISTS whatsapp_sessions (
    tenant_id       VARCHAR     NOT NULL,
    customer_phone  VARCHAR     NOT NULL,
    history         JSONB       NOT NULL DEFAULT '[]',
    last_active     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, customer_phone)
);

CREATE INDEX IF NOT EXISTS idx_whatsapp_sessions_tenant ON whatsapp_sessions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_whatsapp_channels_tenant ON whatsapp_channels(tenant_id);

-- Failed message dead-letter queue (for debugging and replay)
CREATE TABLE IF NOT EXISTS whatsapp_failed_messages (
    id              BIGSERIAL   PRIMARY KEY,
    tenant_id       VARCHAR     NOT NULL,
    customer_phone  VARCHAR     NOT NULL,
    message_text    TEXT        NOT NULL DEFAULT '',
    error_type      VARCHAR     NOT NULL,     -- e.g. "ClaudeApi", "StoreNetwork"
    error_detail    TEXT        NOT NULL DEFAULT '',
    payload         JSONB,                    -- original WA message payload
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_whatsapp_failed_tenant ON whatsapp_failed_messages(tenant_id, created_at DESC);

-- Which door the failed message came through. The table predates Telegram and
-- keeps its name; NULL rows are from before this column and are WhatsApp's.
ALTER TABLE whatsapp_failed_messages ADD COLUMN IF NOT EXISTS channel VARCHAR;

-- ── System-wide admin configuration ─────────────────────────────────────────

CREATE TABLE IF NOT EXISTS system_config (
    key        VARCHAR     PRIMARY KEY,
    value      TEXT        NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- API keys for the messaging assistant's AI provider, one per provider, set by
-- the super admin. Sealed with secret_box (tenant "platform"); never returned by
-- an admin route. The active provider and model are system_config's
-- assistant.provider and assistant.model.
CREATE TABLE IF NOT EXISTS assistant_llm_keys (
    provider    VARCHAR     PRIMARY KEY,
    key_enc     BYTEA       NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Default AI uploader config (idempotent)
INSERT INTO system_config (key, value) VALUES
    ('ai_uploader.provider', 'deepseek'),
    ('ai_uploader.model',    'deepseek-chat')
ON CONFLICT (key) DO NOTHING;

-- Cohere and Claude were dropped (DeepSeek default, Mistral alternative): move a
-- deployment still on either to the default.
UPDATE system_config SET value = 'deepseek', updated_at = NOW()
 WHERE key = 'ai_uploader.provider' AND value IN ('cohere', 'claude');
UPDATE system_config SET value = 'deepseek-chat', updated_at = NOW()
 WHERE key = 'ai_uploader.model' AND (value LIKE 'command-%' OR value LIKE 'claude-%');
UPDATE system_config SET value = 'deepseek', updated_at = NOW()
 WHERE key = 'assistant.provider' AND value = 'claude';
-- Claude is back as provider "anthropic": its models stay when it is chosen.
UPDATE system_config SET value = 'deepseek-chat', updated_at = NOW()
 WHERE key = 'assistant.model' AND value LIKE 'claude-%'
   AND NOT EXISTS (SELECT 1 FROM system_config WHERE key = 'assistant.provider' AND value = 'anthropic');

-- ── Platform-level user roles ───────────────────────────────────────────────
-- Roles: super_admin, admin, user (default)
-- super_admin can manage other admins; admin can access admin panel.
-- Users without a row are regular users.

CREATE TABLE IF NOT EXISTS user_roles (
    email       VARCHAR     PRIMARY KEY,
    role        VARCHAR     NOT NULL DEFAULT 'user',
    granted_by  VARCHAR,
    granted_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Seed the initial super admin (idempotent)
INSERT INTO user_roles (email, role, granted_by) VALUES
    ('mohamed.bennekrouf@gmail.com', 'super_admin', 'system')
ON CONFLICT (email) DO NOTHING;

-- ── Per-user, per-front-end preferences ─────────────────────────────────────
-- A free-form JSON document a signed-in person keeps about themselves on one
-- of the sites that authenticate against the gateway. `app` is the Firebase
-- project that signed them in (app.api0.ai and mayorana.ch use different
-- ones), so the same email holds one document per site and neither site has
-- to know the other's schema. Not tenant data: no RLS, like user_roles.

CREATE TABLE IF NOT EXISTS app_user_prefs (
    app         VARCHAR     NOT NULL,
    email       VARCHAR     NOT NULL,
    prefs       JSONB       NOT NULL DEFAULT '{}'::jsonb,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (app, email)
);

-- ── Email engagement tracking ────────────────────────────────────────────────
-- first_call_at: set on first successful API call (FirstCallMilestone + Tier-3 nudge guard)
-- nudge_sent_at: set when 7-day nudge is sent (prevents resending)
-- winback_sent_at: set when 30-day win-back is sent (prevents resending)
-- welcome_sent on user_preferences: prevents duplicate welcome emails

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'tenants' AND column_name = 'first_call_at') THEN
        ALTER TABLE tenants ADD COLUMN first_call_at TIMESTAMP WITH TIME ZONE;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'tenants' AND column_name = 'nudge_sent_at') THEN
        ALTER TABLE tenants ADD COLUMN nudge_sent_at TIMESTAMP WITH TIME ZONE;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'tenants' AND column_name = 'winback_sent_at') THEN
        ALTER TABLE tenants ADD COLUMN winback_sent_at TIMESTAMP WITH TIME ZONE;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'user_preferences' AND column_name = 'welcome_sent') THEN
        ALTER TABLE user_preferences ADD COLUMN welcome_sent BOOLEAN NOT NULL DEFAULT false;
    END IF;
END $$;

-- ── Security hardening ────────────────────────────────────────────────────────

-- Key expiration: NULL = no expiry (admin/tenant keys).
-- Consumer keys generated via the self-service endpoint default to 1 year.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'api_keys' AND column_name = 'expires_at'
    ) THEN
        ALTER TABLE api_keys ADD COLUMN expires_at TIMESTAMP WITH TIME ZONE;
        CREATE INDEX idx_api_keys_expires_at ON api_keys(expires_at)
            WHERE expires_at IS NOT NULL;
    END IF;
END $$;

-- ── Per-user downstream credentials ──────────────────────────────────────────
-- One credential per (tenant, user), so a tool call to a third-party API acts as
-- the person who made it rather than as one shared service identity. The secret
-- is sealed by infra::secret_box (AES-256-GCM); the plaintext never reaches this
-- table, and nothing but the gateway ever reads it back.
--
--   kind = 'pat'            an API token the user pasted (Azure DevOps, GitHub…)
--   kind = 'entra_refresh'  an OAuth refresh token, once federation exists
CREATE TABLE IF NOT EXISTS user_downstream_credentials (
    tenant_id   VARCHAR NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_email  VARCHAR NOT NULL,
    kind        VARCHAR NOT NULL DEFAULT 'pat',
    secret      BYTEA   NOT NULL,
    -- What the user called it, so a dashboard can show which token this is
    -- without ever decrypting it.
    label       VARCHAR NOT NULL DEFAULT '',
    -- The expiry the user told us about. Advisory only — we cannot verify it,
    -- and the provider is the authority — but it is what lets us warn before a
    -- token dies rather than after.
    expires_at  TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, user_email, kind)
);

CREATE INDEX IF NOT EXISTS idx_user_downstream_credentials_tenant
    ON user_downstream_credentials(tenant_id);

-- Per-user auth needs the tenant to say how a secret becomes a header. Azure
-- DevOps wants HTTP Basic with an empty username; most APIs want a bearer.
--   per_user_scheme = 'basic_pat' | 'bearer' | 'raw'
--   per_user_header = header name, defaults to Authorization
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS per_user_scheme VARCHAR;
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS per_user_header VARCHAR;

-- ── Encryption of the legacy secret columns ──────────────────────────────────
-- These columns predate infra::secret_box and hold plaintext. Rather than
-- change their types in place — which would break any running instance mid
-- deploy — each gets a BYTEA sibling holding the sealed value.
--
-- Reads prefer the sealed column and fall back to the plaintext one, so a row
-- that has not been migrated yet still works. Writes only ever populate the
-- sealed column and NULL the plaintext one, so anything written after this
-- deploy is encrypted with no migration needed at all.
--
-- The backfill of existing rows is deliberately NOT automatic: it is a one-shot
-- rewrite of live credentials and wants a database backup taken first. Run it
-- with POST /api/internal/encrypt-legacy-secrets. It is re-runnable and skips
-- rows that are already sealed.
--
-- Once the backfill has run and been verified, the plaintext columns can be
-- dropped in a later release.
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS bearer_token_enc         BYTEA;
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS custom_headers_enc       BYTEA;
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS service_account_json_enc BYTEA;
ALTER TABLE mcp_tools              ADD COLUMN IF NOT EXISTS static_headers_enc       BYTEA;

-- ── How a tenant's people sign in ────────────────────────────────────────────
-- A tenant registered as an OAuth client (mcp_client_id) must say how its users
-- authenticate, or the consent flow refuses it — a tenant that is registered but
-- not configured must fail, never fall through to a broader identity pool.
--
-- Two ways to be configured:
--   google_client_id set   → users sign in through that Google workspace
--   allow_api0_signin true → users sign in with their own api0 account
--
-- The second is deliberately opt-in and defaults to false, because it means
-- *any* api0 account may connect to this workspace. That is reasonable for a
-- tenant whose tools use per-user credentials — a stranger who connects still
-- has no token of their own and can do nothing — and a poor idea for one with a
-- shared downstream credential, where connecting would borrow it.
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS allow_api0_signin BOOLEAN NOT NULL DEFAULT FALSE;

-- ── Verifying a per-user credential on save ──────────────────────────────────
-- A pasted token is a bearer credential: whoever holds it acts as its owner, and
-- nothing about storing it proves it belongs to the person who pasted it. If one
-- person stores another's token, every action they take is attributed to that
-- other person, silently.
--
-- So a tenant can name a read-only endpoint that answers "who is this token?".
-- On save the gateway calls it with the token and records the answer, which
-- turns an assumption into something displayed next to the token — and rejects
-- a token that does not authenticate at all, at paste time rather than at the
-- first tool call.
--
--   per_user_verify_url      GET endpoint, called with the user's credential
--   per_user_identity_pointer  RFC 6901 JSON pointer into its response
--
-- Azure DevOps, for example:
--   url      https://dev.azure.com/<org>/_apis/connectionData?api-version=7.1
--   pointer  /authenticatedUser/properties/Account/$value
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS per_user_verify_url       VARCHAR;
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS per_user_identity_pointer VARCHAR;

-- ── auth_mode 'client_credentials' ──────────────────────────────────────────
-- OAuth 2.0 client credentials (RFC 6749 §4.4): the default machine-to-machine
-- mechanism across enterprise SaaS — Salesforce, Microsoft Graph, ServiceNow,
-- Workday, Okta and the rest.
--
-- It is the one shape the other modes could not express. static_bearer and
-- header_injection hold a credential that never changes; google_sa mints a
-- short-lived one but only ever from Google. A tenant on such a backend had to
-- paste an access token into static_bearer, which worked until it expired an
-- hour later and then returned 401s that read like a bad credential.
--
-- The secret is sealed like every other, and there is deliberately no plaintext
-- column to fall back to: nothing predates this, so nothing needs a backfill.
--
--   oauth_token_url          the provider's token endpoint
--   oauth_client_id          the registered client
--   oauth_client_secret_enc  sealed by infra::secret_box
--   oauth_scope              space-separated, when the provider needs them
--   oauth_client_auth_style  'post' (default) | 'basic'
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS oauth_token_url         VARCHAR;
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS oauth_client_id         VARCHAR;
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS oauth_client_secret_enc BYTEA;
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS oauth_scope             VARCHAR;
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS oauth_client_auth_style VARCHAR;

-- ── auth_mode 'per_user_oauth' ──────────────────────────────────────────────
-- Three-legged OAuth: each person connects their own account once, and api0
-- keeps the access token fresh from a refresh token.
--
-- The difference from 'per_user' is who does the work. per_user means every
-- person creates a token by hand in the provider's UI and replaces it when it
-- expires; this means they press "Connect" once and never think about it again.
--
-- Provider settings reuse the oauth_* columns added for client_credentials --
-- the token exchange is the same, only the grant differs -- plus the authorize
-- endpoint, which only the three-legged flow needs.
ALTER TABLE tenant_downstream_auth ADD COLUMN IF NOT EXISTS oauth_authorize_url VARCHAR;

-- Per-person tokens live in user_downstream_credentials, which is already keyed
-- (tenant_id, user_email, kind) and already carries expires_at. Two new kinds
-- rather than a new table:
--
--   kind 'oauth_refresh'  the long-lived refresh token; expires_at usually NULL
--   kind 'oauth_access'   the current access token, with a real expires_at
--
-- Storing the access token at all is what stops every tool call paying for a
-- refresh round trip. It is sealed exactly like the refresh token: an access
-- token is a bearer credential, and a leaked one is usable until it expires.

-- Short-lived record of an authorization in flight. The state parameter is the
-- CSRF defence: the callback is an unauthenticated endpoint the provider
-- redirects a browser to, so the only thing tying that request back to a person
-- is this row. Single use, and swept on the way in.
CREATE TABLE IF NOT EXISTS downstream_oauth_requests (
    state         VARCHAR     PRIMARY KEY,
    tenant_id     VARCHAR     NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_email    VARCHAR     NOT NULL,
    -- PKCE. Sent as a challenge on the way out and proved here on the way back,
    -- so an intercepted authorization code cannot be redeemed by anyone else.
    code_verifier VARCHAR     NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at    TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_downstream_oauth_requests_expiry
    ON downstream_oauth_requests(expires_at);

-- Who the stored token turned out to be. NULL when the tenant configured no
-- verification, or when it was stored before verification existed.
ALTER TABLE user_downstream_credentials ADD COLUMN IF NOT EXISTS verified_identity VARCHAR;

-- ── Inbound identity: how a tenant's people prove who they are to api0 ───────
-- Modelled as protocol + issuer rather than a list of vendor names. With OIDC
-- discovery the endpoints and signing keys are fetched from the issuer at run
-- time, so Entra, Okta, Auth0 and Google are configuration rather than code.
--
--   idp_issuer         e.g. https://login.microsoftonline.com/<directory>/v2.0
--   idp_client_id      the app registration's client id
--   idp_client_secret  sealed by infra::secret_box — never leaves the server
--
-- A tenant with an issuer set signs its people in there. Without one it falls
-- back to google_client_id, or to api0's own sign-in when allow_api0_signin.
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS idp_issuer        VARCHAR;
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS idp_client_id     VARCHAR;
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS idp_client_secret BYTEA;

-- One row per in-flight sign-in, holding the PKCE verifier.
--
-- In Postgres rather than memory on purpose: the authorize leg and the callback
-- are separate requests, and both processes run single-instance today. An
-- in-memory map would work now and start failing on a fraction of logins the
-- day someone sets instances > 1 — intermittently, with an error that looks
-- like the identity provider's fault.
--
-- Deleted on use, so it is also replay protection for the authorization code.
CREATE TABLE IF NOT EXISTS idp_auth_requests (
    state_nonce VARCHAR     PRIMARY KEY,
    tenant_id   VARCHAR     NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    verifier    VARCHAR     NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_idp_auth_requests_created
    ON idp_auth_requests(created_at);


-- ── Redeemed OAuth authorization codes ───────────────────────────────────────
-- The gateway's authorization codes are signed JWTs, so they need no row to be
-- valid — but without one, a code could be exchanged again and again until it
-- expired, minting a new API key each time. The gateway records each code's
-- `jti` here at exchange; the primary key is what refuses the second attempt.
--
-- Codes live five minutes. A row only has to outlive its code, and older ones
-- are swept on the next redemption.
CREATE TABLE IF NOT EXISTS oauth_code_redemptions (
    jti         VARCHAR     PRIMARY KEY,
    redeemed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_oauth_code_redemptions_redeemed
    ON oauth_code_redemptions(redeemed_at);


-- ── Consumer memberships ─────────────────────────────────────────────────────
-- A consumer key (one carrying provider_tenant_id) records that somebody reaches
-- a provider's tools through a connector. Until now that relationship lived only
-- on api_keys, so a provider tenant showed one member — its owner — no matter how
-- many people used it.
--
-- The role is 'consumer', which grants nothing: verify_tenant_access and the
-- tenant picker both exclude it. It is a relationship, not a permission.
--
-- Idempotent backfill for keys issued before the link was written at issue time.
-- The email written must be the exact value user_preferences holds: the foreign
-- key matches on value, not case, so inserting a lowercased copy of a
-- differently-cased stored address fails the constraint — and a failure here
-- aborts schema setup and stops the store from starting.
-- Wrapped so it cannot stop the store from starting. schema.sql runs as one
-- batch at boot and any error aborts the rest of it, which is the correct
-- behaviour for a table definition and the wrong one for a data backfill: a
-- surprise in existing rows should cost the backfill, not the service.
DO $$
BEGIN
    INSERT INTO tenant_users (tenant_id, email, role)
    SELECT DISTINCT k.provider_tenant_id, up.email, 'consumer'
      FROM api_keys k
      JOIN user_preferences up ON LOWER(up.email) = LOWER(k.email)
      JOIN tenants t ON t.id = k.provider_tenant_id
     WHERE k.provider_tenant_id IS NOT NULL
       AND k.email IS NOT NULL
    ON CONFLICT (tenant_id, email) DO NOTHING;
EXCEPTION WHEN OTHERS THEN
    RAISE WARNING 'Consumer backfill skipped: %', SQLERRM;
END $$;

-- ── Messaging channel identities ─────────────────────────────────────────────
-- A messaging channel identifies a person by whatever it has — a phone number,
-- a Telegram user id — and api0 identifies them by email. These two tables are
-- the bridge between the two, and they are the reason a message from a phone can
-- run a tool as a specific person, with that person's own credentials and that
-- person's name on whatever it creates.
--
-- Nothing here is specific to any channel. `channel` is a label; `external_id`
-- is whatever that channel calls a user.

-- A short code a signed-in person mints in the dashboard and sends to the bot.
-- Proves "this messaging identity is mine". Ten minutes, single use.
CREATE TABLE IF NOT EXISTS channel_link_codes (
    code        VARCHAR     PRIMARY KEY,
    user_email  VARCHAR     NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at  TIMESTAMPTZ NOT NULL
);

-- A linked identity. The api0 key is minted at link time, pinned to the
-- channel's tenant, and sealed by infra::secret_box — the bridge reads it back
-- through one internal route and uses it exactly as Claude uses its own key.
CREATE TABLE IF NOT EXISTS channel_identities (
    channel      VARCHAR     NOT NULL,          -- 'whatsapp' | 'telegram' | 'teams' | …
    external_id  VARCHAR     NOT NULL,          -- phone number, chat user id, …
    tenant_id    VARCHAR     NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    user_email   VARCHAR     NOT NULL,
    api_key_id   VARCHAR     NOT NULL,          -- so unlinking can revoke it
    api_key_enc  BYTEA       NOT NULL,          -- sealed
    linked_at    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (channel, external_id, tenant_id)
);

CREATE INDEX IF NOT EXISTS idx_channel_identities_user
    ON channel_identities(tenant_id, user_email);

-- ── Messaging channels (generic) ─────────────────────────────────────────────
-- A bot a tenant owns on some messaging platform. whatsapp_channels predates
-- this and keeps its own shape; every channel added from here on lives here.
--
--   channel        'telegram' | …
--   channel_ref    what the platform uses to route to this bot — the Telegram
--                  bot id (the number before the colon in its token). Public.
--   credential_enc the bot token, sealed by infra::secret_box
--   display_ref    something a person can recognise — the @username
--   webhook_secret sent by the platform on every update, checked by the bridge
CREATE TABLE IF NOT EXISTS messaging_channels (
    channel        VARCHAR     NOT NULL,
    channel_ref    VARCHAR     NOT NULL,
    tenant_id      VARCHAR     NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    credential_enc BYTEA       NOT NULL,
    display_ref    VARCHAR     NOT NULL DEFAULT '',
    webhook_secret VARCHAR     NOT NULL,
    system_prompt  TEXT        NOT NULL DEFAULT '',
    created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (channel, channel_ref),
    UNIQUE (channel, tenant_id)
);

-- A workspace's own name for each connected channel ("Support bot", "Sales
-- WhatsApp"), shown in its dashboard. Empty means unnamed.
ALTER TABLE messaging_channels ADD COLUMN IF NOT EXISTS label VARCHAR NOT NULL DEFAULT '';
ALTER TABLE whatsapp_channels  ADD COLUMN IF NOT EXISTS label VARCHAR NOT NULL DEFAULT '';

-- What a tenant tells the model about its own tools, returned as `instructions`
-- in the MCP initialize result, after the guidance api0 generates from the
-- tenant's tool groups. Plain text, written by the tenant — for example which
-- project is the main one, or how its teams are named. NULL means none.
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS mcp_instructions TEXT;

-- The Meta app's App Secret, per tenant. Meta signs every webhook POST with the
-- secret of the app the webhook is configured on, and every tenant brings its
-- own app — so one platform-wide secret verifies at most one tenant and rejects
-- the rest. NULL falls back to the platform's META_APP_SECRET.
ALTER TABLE whatsapp_channels ADD COLUMN IF NOT EXISTS app_secret TEXT;

-- The last time a platform delivered a message to the bridge, written at the
-- door before identity, rate limits or the model are involved. The only trace a
-- message from someone not yet linked leaves, and so the only proof that a
-- webhook set up in Meta or registered with Telegram is really being called.
CREATE TABLE IF NOT EXISTS channel_inbound (
    tenant_id        VARCHAR     NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    channel          VARCHAR     NOT NULL,          -- 'whatsapp' | 'telegram' | 'teams'
    last_inbound_at  TIMESTAMPTZ NOT NULL,
    last_unlinked_at TIMESTAMPTZ,                   -- last one from an unlinked sender
    PRIMARY KEY (tenant_id, channel)
);

-- The Stripe PaymentIntent a top-up came from. Unique, so a payment is credited
-- once however many times it is reported: by the browser's confirm call, by the
-- payment_intent.succeeded webhook, or by both, in either order. NULL for every
-- other kind of balance change.
ALTER TABLE credit_transactions ADD COLUMN IF NOT EXISTS stripe_payment_intent_id VARCHAR;
CREATE UNIQUE INDEX IF NOT EXISTS idx_credit_tx_stripe_intent
    ON credit_transactions(stripe_payment_intent_id)
    WHERE stripe_payment_intent_id IS NOT NULL;

-- ── Monthly plans: the only way to buy ──────────────────────────────────────
-- Someone subscribes to a plan with their email; the subscription gets one
-- licence number, emailed to them, which unlocks the plan's apps and pays for
-- AI calls from its monthly credits. Apps check it online (gateway /v1/licence).
-- The plans on sale, set by the super admin in the dashboard (payment::plans).
-- The Stripe price is created by the store whenever the amount or currency
-- changes; the old one is archived, and subscribers already on it keep it.
--   monthly_credits  the AI credits each paid month brings; unused ones lapse
CREATE TABLE IF NOT EXISTS plans (
    id                VARCHAR     PRIMARY KEY,
    name              VARCHAR     NOT NULL,
    description       TEXT        NOT NULL DEFAULT '',
    price_cents       BIGINT      NOT NULL,
    currency          VARCHAR     NOT NULL DEFAULT 'eur',
    monthly_credits   BIGINT      NOT NULL,
    stripe_product_id VARCHAR,
    stripe_price_id   VARCHAR,
    active            BOOLEAN     NOT NULL DEFAULT TRUE,
    sort_order        INTEGER     NOT NULL DEFAULT 0,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- One row per Stripe subscription, kept in step by the webhooks
-- (payment::subscription). The month's credits live here, not in
-- tenants.credit_balance: they reset with every paid period, and the API
-- credits a tenant buys never do.
--   period_start/end      the paid month Stripe reports
--   credits_granted/used  this period's allowance; a new period sets used to 0
CREATE TABLE IF NOT EXISTS subscriptions (
    id                   VARCHAR     PRIMARY KEY,
    tenant_id            VARCHAR     REFERENCES tenants(id),
    email                VARCHAR     NOT NULL,
    plan_id              VARCHAR     NOT NULL REFERENCES plans(id),
    stripe_customer_id   VARCHAR     NOT NULL,
    status               VARCHAR     NOT NULL,
    cancel_at_period_end BOOLEAN     NOT NULL DEFAULT FALSE,
    period_start         TIMESTAMPTZ NOT NULL,
    period_end           TIMESTAMPTZ NOT NULL,
    credits_granted      BIGINT      NOT NULL DEFAULT 0,
    credits_used         BIGINT      NOT NULL DEFAULT 0,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_subscriptions_tenant ON subscriptions(tenant_id);
CREATE INDEX IF NOT EXISTS idx_subscriptions_email ON subscriptions(email);

-- The apps a plan makes Pro, by the id each app checks for ("*" for all).
ALTER TABLE plans ADD COLUMN IF NOT EXISTS products TEXT[] NOT NULL DEFAULT '{}';
-- A subscriber needs no api0 account: the email is enough.
ALTER TABLE subscriptions ALTER COLUMN tenant_id DROP NOT NULL;
--   licence             the licence number, set when the subscription is first synced
--   licence_emailed_at  when it was emailed: once, as soon as the subscription is live
ALTER TABLE subscriptions ADD COLUMN IF NOT EXISTS licence VARCHAR;
ALTER TABLE subscriptions ADD COLUMN IF NOT EXISTS licence_emailed_at TIMESTAMPTZ;
CREATE UNIQUE INDEX IF NOT EXISTS idx_subscriptions_licence ON subscriptions(licence);
-- A free plan's subscription: no Stripe behind it (stripe_customer_id is ''),
-- one per email, and the store itself starts each new month. Its licence moves
-- to the paid subscription the same email takes out later.
ALTER TABLE subscriptions ADD COLUMN IF NOT EXISTS is_free BOOLEAN NOT NULL DEFAULT FALSE;
CREATE UNIQUE INDEX IF NOT EXISTS idx_subscriptions_one_free ON subscriptions(email) WHERE is_free AND licence IS NOT NULL;
--   cancel_emailed_at   when the "your plan is cancelled" email went out; cleared
--                       if they resume, so a later cancel is confirmed again
--   comeback_emailed_at when the one "come back" reminder went out, some days
--                       after a cancelled plan ended
ALTER TABLE subscriptions ADD COLUMN IF NOT EXISTS cancel_emailed_at TIMESTAMPTZ;
ALTER TABLE subscriptions ADD COLUMN IF NOT EXISTS comeback_emailed_at TIMESTAMPTZ;

-- ── The default lineup ───────────────────────────────────────────────────────
-- The apps themselves are the free version; plans are for Pro and AI credits.
-- Every paid plan makes every app Pro and they differ by credits, as on most
-- credit-based pricing pages: Basic to get started, Pro for daily use, Max for
-- heavy AI use. Inactive until the super admin reviews them and saves each one
-- on sale, which is also what creates its Stripe price.
--
-- Free is not an offer: a small monthly allowance of AI credits, no Pro, never
-- listed on a pricing page. An app hands it out by email (POST /api/plans/free).
--
-- Once per database (marker plans.default_lineup): the first seeds, Starter and
-- Pro with no apps, are replaced if nobody edited or sold them, and the free
-- allowance is set up, or raised from its first 20 credits if untouched. After
-- that the super admin's edits are never overwritten.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM system_config WHERE key = 'plans.default_lineup') THEN
        RETURN;
    END IF;
    DELETE FROM plans p
     WHERE (p.id, p.name, p.price_cents, p.monthly_credits) IN (('starter', 'Starter', 1900, 500), ('pro', 'Pro', 4900, 2000))
       AND p.products = '{}' AND p.stripe_price_id IS NULL AND NOT p.active
       AND NOT EXISTS (SELECT 1 FROM subscriptions s WHERE s.plan_id = p.id);
    INSERT INTO plans (id, name, description, price_cents, currency, monthly_credits, products, active, sort_order) VALUES
        ('basic', 'Basic', 'For getting started.', 1000, 'usd',  500, '{*}', FALSE, 1),
        ('pro',   'Pro',   'For daily use.',       3000, 'usd', 1750, '{*}', FALSE, 2),
        ('max',   'Max',   'For heavy AI use.',    6000, 'usd', 4000, '{*}', FALSE, 3),
        ('free',  'Free',  'A few AI credits every month, to try the apps'' AI. Not sold.', 0, 'usd', 50, '{}', TRUE, 0)
    ON CONFLICT (id) DO NOTHING;
    UPDATE plans SET monthly_credits = 50, active = TRUE, updated_at = NOW()
     WHERE id = 'free' AND price_cents = 0 AND products = '{}' AND monthly_credits = 20;
    INSERT INTO system_config (key, value) VALUES ('plans.default_lineup', 'basic-pro-max');
END $$;

-- What an AI call costs in plan credits: ceil(tokens × multiplier ÷ tokens_per_credit),
-- at least 1. The multipliers are a JSON object, model → factor; a model not in
-- it counts as 1.
INSERT INTO system_config (key, value) VALUES
    ('plans.tokens_per_credit', '10000'),
    ('plans.model_multipliers', '{"deepseek-reasoner": 3, "mistral-medium-latest": 2}')
ON CONFLICT (key) DO NOTHING;
-- What one use of each app's AI costs, for the pricing page's "what your
-- credits buy" (payment::plans::AppUse), by the product id the licences use.
-- The super admin adjusts them.
INSERT INTO system_config (key, value) VALUES
    ('plans.app_uses', '[{"app":"gitagent","credits":50},{"app":"shipscreen","credits":25},{"app":"splitter","credits":2},{"app":"small-video","credits":20},{"app":"jobscout","credits":100},{"app":"spreadwatch","credits":10},{"app":"surface-watch","credits":50}]')
ON CONFLICT (key) DO NOTHING;

-- ── Tenant names are unique, ignoring case ───────────────────────────────────
-- "solanize" and "Solanize" are two workspaces nobody can tell apart. The store
-- checks before every write (tenant_management::NAME_TAKEN_SQL); this index is
-- what holds when two writes race.
--
-- Existing duplicates make the CREATE fail. That must not stop the store from
-- starting — this file runs as one batch at boot — so the failure is a warning:
-- rename or delete the duplicates and the index is created on the next start.
DO $$
BEGIN
    CREATE UNIQUE INDEX IF NOT EXISTS tenants_name_ci_key ON tenants (lower(btrim(name)));
EXCEPTION WHEN unique_violation THEN
    RAISE WARNING 'tenants_name_ci_key not created: some tenant names collide ignoring case';
END $$;

-- ── Workspace invitations ────────────────────────────────────────────────────
-- tenant_users.email is a foreign key onto user_preferences, so someone who has
-- never signed in cannot be a member yet. Their invitation waits here and is
-- turned into a membership inside their first sign-in (tenant_members.rs).
CREATE TABLE IF NOT EXISTS tenant_invites (
    tenant_id   VARCHAR     NOT NULL REFERENCES tenants(id) ON DELETE CASCADE,
    email       VARCHAR     NOT NULL,   -- lowercased
    role        VARCHAR     NOT NULL,   -- owner | admin | member
    invited_by  VARCHAR     NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, email)
);
CREATE INDEX IF NOT EXISTS idx_tenant_invites_email ON tenant_invites(email);

-- ── User directory: activity and age ────────────────────────────────────────
-- "Last active" is the latest tool call made as or for the user: every MCP
-- call — from Claude or a Telegram/WhatsApp bot — lands in api_usage_logs.
CREATE INDEX IF NOT EXISTS idx_usage_logs_email_ts ON api_usage_logs(email, timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_usage_logs_consumer_ts ON api_usage_logs(consumer_id, timestamp DESC);

-- user_preferences.created_at arrived after most users did. Give those the
-- earliest trace they left (first key, first call, first credit movement,
-- first channel link); someone with none stays unknown. Runs once in effect:
-- only NULL rows are touched.
UPDATE user_preferences up
   SET created_at = first.at
  FROM (
        SELECT e.email, min(e.at) AS at FROM (
            SELECT email, generated_at AS at FROM api_keys
            UNION ALL SELECT email, timestamp FROM api_usage_logs
            UNION ALL SELECT email, created_at FROM credit_transactions
            UNION ALL SELECT user_email, linked_at FROM channel_identities
        ) e GROUP BY e.email
       ) first
 WHERE up.email = first.email AND up.created_at IS NULL;

-- Addresses that opted out of the optional emails (EmailKind::is_optional):
-- the digest, nudge, win-back and "what's new". Stored lowercased; see
-- src/email/unsubscribe.rs.
CREATE TABLE IF NOT EXISTS email_opt_outs (
    email      TEXT PRIMARY KEY,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ── LLM provider: usage ─────────────────────────────────────────────────────
-- A tenant named by LLM_PROVIDER_TENANT_ID serves chat completions to the
-- desktop apps (gateway: POST /v1/chat/completions). The app presents its
-- licence number; that subscription's monthly credits pay.
-- See payment::llm_wallet.
-- One row per completion. Not api_usage_logs: a licence is not an api key.
-- license_id is the subscription the licence belongs to; product is the app.
CREATE TABLE IF NOT EXISTS llm_usage (
    id               VARCHAR     PRIMARY KEY,
    tenant_id        VARCHAR     NOT NULL,        -- the provider tenant
    license_id       VARCHAR     NOT NULL,
    email            VARCHAR     NOT NULL,
    product          VARCHAR     NOT NULL,
    provider         VARCHAR     NOT NULL,        -- upstream: deepseek, mistral
    model            VARCHAR     NOT NULL,
    input_tokens     BIGINT      NOT NULL,
    output_tokens    BIGINT      NOT NULL,
    total_tokens     BIGINT      NOT NULL,
    estimated        BOOLEAN     NOT NULL,        -- counted by us, not the provider
    credits          BIGINT      NOT NULL,
    status_code      INTEGER     NOT NULL,
    response_time_ms BIGINT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_llm_usage_email_ts ON llm_usage(email, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_llm_usage_tenant_ts ON llm_usage(tenant_id, created_at DESC);
-- api0's own AI calls (YAML import) land here too, with no licence and no
-- credits, so the super admin sees every LLM call in one ledger.
ALTER TABLE llm_usage ALTER COLUMN license_id DROP NOT NULL;
CREATE INDEX IF NOT EXISTS idx_llm_usage_ts ON llm_usage(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_usage_logs_llm_ts ON api_usage_logs(timestamp DESC) WHERE method = 'LLM';
-- The region the app chose (admin/llm_regions.rs), which is all its user sees
-- of the call; provider and model say what ran. NULL: a model named directly.
ALTER TABLE llm_usage ADD COLUMN IF NOT EXISTS region VARCHAR;
-- The answer stopped at max_tokens (finish_reason "length"): a model that
-- starts looping shows as a rising share of these per region and model.
ALTER TABLE llm_usage ADD COLUMN IF NOT EXISTS cut_off BOOLEAN NOT NULL DEFAULT FALSE;

-- Who pays for a tenant's paid tools — one or the other, never both:
--   'tenant'  the tenant's credit_balance pays for every call made with its keys,
--             whoever the caller is;
--   'user'    each caller pays from their own plan's monthly credits
--             (subscriptions), and the tenant's balance is never touched.
-- See payment::tenant_billing.
ALTER TABLE tenants ADD COLUMN IF NOT EXISTS billing_mode VARCHAR NOT NULL DEFAULT 'tenant'
    CHECK (billing_mode IN ('tenant', 'user'));
