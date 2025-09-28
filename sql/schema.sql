-- Updated schema without is_default concept
PRAGMA foreign_keys = ON;

-- Keep user_preferences table for preferences and credit
CREATE TABLE IF NOT EXISTS user_preferences (
    email VARCHAR NOT NULL,
    hidden_defaults TEXT NOT NULL DEFAULT '',
    credit_balance INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (email)
);

-- API usage logs table for detailed tracking
CREATE TABLE IF NOT EXISTS api_usage_logs (
    id VARCHAR NOT NULL,
    key_id VARCHAR NOT NULL,
    email VARCHAR NOT NULL,
    endpoint_path VARCHAR NOT NULL,
    method VARCHAR NOT NULL,
    timestamp VARCHAR NOT NULL,
    response_status INTEGER,
    response_time_ms INTEGER,
    request_size INTEGER,
    response_size INTEGER,
    ip_address VARCHAR,
    user_agent VARCHAR,
    usage_estimated BOOLEAN,
    input_tokens INTEGER,
    output_tokens INTEGER,
    total_tokens INTEGER,
    model_used VARCHAR,
    PRIMARY KEY (id),
    FOREIGN KEY (key_id) REFERENCES api_keys(id)
);

-- API keys table
CREATE TABLE IF NOT EXISTS api_keys (
    id VARCHAR NOT NULL,
    email VARCHAR NOT NULL,
    key_hash VARCHAR NOT NULL,
    key_prefix VARCHAR NOT NULL,
    key_name VARCHAR NOT NULL,
    generated_at VARCHAR NOT NULL,
    last_used VARCHAR,
    usage_count INTEGER NOT NULL DEFAULT 0,
    is_active VARCHAR NOT NULL DEFAULT true,
    PRIMARY KEY (id),
    FOREIGN KEY (email) REFERENCES user_preferences(email)
);

-- API Groups table (no is_default column)
CREATE TABLE IF NOT EXISTS api_groups (
    id VARCHAR PRIMARY KEY,
    name VARCHAR NOT NULL,
    description VARCHAR NOT NULL DEFAULT '',
    base VARCHAR NOT NULL DEFAULT ''
);

-- Endpoints table with group reference (no is_default column)
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
    required VARCHAR NOT NULL DEFAULT false,
    FOREIGN KEY (endpoint_id) REFERENCES endpoints(id)
);

-- Parameter alternatives
CREATE TABLE IF NOT EXISTS parameter_alternatives (
    endpoint_id VARCHAR,
    parameter_name VARCHAR,
    alternative VARCHAR NOT NULL,
    FOREIGN KEY (endpoint_id) REFERENCES endpoints(id)
);


CREATE TABLE IF NOT EXISTS domains (
    id VARCHAR NOT NULL,
    email VARCHAR NOT NULL,
    domain VARCHAR NOT NULL,
    verified BOOLEAN NOT NULL DEFAULT false,
    added_at VARCHAR NOT NULL,
    last_used VARCHAR,
    verification_token VARCHAR,
    PRIMARY KEY (id),
    UNIQUE(email, domain)
);

CREATE INDEX IF NOT EXISTS idx_domains_email ON domains(email);
CREATE INDEX IF NOT EXISTS idx_domains_verified ON domains(verified);

-- Create indexes for better performance
CREATE INDEX IF NOT EXISTS idx_api_keys_email ON api_keys(email);
CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash);
CREATE INDEX IF NOT EXISTS idx_usage_logs_key_id ON api_usage_logs(key_id);
CREATE INDEX IF NOT EXISTS idx_usage_logs_email ON api_usage_logs(email);
CREATE INDEX IF NOT EXISTS idx_usage_logs_timestamp ON api_usage_logs(timestamp);


