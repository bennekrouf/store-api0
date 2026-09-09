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
-- client_id (e.g. "cvenom-mcp") that resolves to their provider_tenant_id.
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

-- ── System-wide admin configuration ─────────────────────────────────────────

CREATE TABLE IF NOT EXISTS system_config (
    key        VARCHAR     PRIMARY KEY,
    value      TEXT        NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Default AI uploader config (idempotent)
INSERT INTO system_config (key, value) VALUES
    ('ai_uploader.provider', 'cohere'),
    ('ai_uploader.model',    'command-r7b-12-2024')
ON CONFLICT (key) DO NOTHING;

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
