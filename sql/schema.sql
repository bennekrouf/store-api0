-- New schema for API groups and endpoints
PRAGMA foreign_keys = ON;

-- Keep user_preferences table, but focus it just on preferences and credit
CREATE TABLE IF NOT EXISTS user_preferences (
    email VARCHAR NOT NULL,
    hidden_defaults TEXT NOT NULL DEFAULT '', -- Comma-separated list of endpoint IDs
    credit_balance INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (email)
);

-- New table for API keys
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


-- API Groups table
CREATE TABLE IF NOT EXISTS api_groups (
    id VARCHAR PRIMARY KEY,
    name VARCHAR NOT NULL,
    description VARCHAR NOT NULL DEFAULT '',
    base VARCHAR NOT NULL DEFAULT 'http://localhost:3000',
    is_default VARCHAR NOT NULL DEFAULT true
);

-- Modified endpoints table with group reference
CREATE TABLE IF NOT EXISTS endpoints (
    id VARCHAR PRIMARY KEY,
    text VARCHAR NOT NULL,
    description VARCHAR NOT NULL DEFAULT '',
    is_default VARCHAR NOT NULL DEFAULT true,
    verb VARCHAR NOT NULL DEFAULT 'GET',
    base VARCHAR NOT NULL DEFAULT 'http://localhost:3000',
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

-- User endpoint associations (no change)
CREATE TABLE IF NOT EXISTS user_endpoints (
    email VARCHAR NOT NULL,
    endpoint_id VARCHAR NOT NULL,
    FOREIGN KEY (endpoint_id) REFERENCES endpoints(id),
    PRIMARY KEY (email, endpoint_id)
);

-- Parameters table (no change)
CREATE TABLE IF NOT EXISTS parameters (
    endpoint_id VARCHAR,
    name VARCHAR NOT NULL,
    description VARCHAR NOT NULL DEFAULT '',
    required VARCHAR NOT NULL DEFAULT false,
    FOREIGN KEY (endpoint_id) REFERENCES endpoints(id)
);

-- Parameter alternatives (no change)
CREATE TABLE IF NOT EXISTS parameter_alternatives (
    endpoint_id VARCHAR,
    parameter_name VARCHAR,
    alternative VARCHAR NOT NULL,
    FOREIGN KEY (endpoint_id) REFERENCES endpoints(id)
);

-- Create index on email for faster lookups
CREATE INDEX IF NOT EXISTS idx_api_keys_email ON api_keys(email);
-- Create index on key_hash for faster validation
CREATE INDEX IF NOT EXISTS idx_api_keys_hash ON api_keys(key_hash);

