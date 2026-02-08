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

    -- api_usage_logs
    IF NOT EXISTS (SELECT 1 FROM information_schema.columns WHERE table_name = 'api_usage_logs' AND column_name = 'tenant_id') THEN
        ALTER TABLE api_usage_logs ADD COLUMN tenant_id VARCHAR;
        CREATE INDEX idx_usage_logs_tenant_id ON api_usage_logs(tenant_id);
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
