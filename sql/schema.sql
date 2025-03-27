-- New schema for API groups and endpoints


-- User preferences table to store hidden default endpoints
CREATE TABLE IF NOT EXISTS user_preferences (
    email VARCHAR NOT NULL,
    hidden_defaults TEXT NOT NULL, -- Comma-separated list of endpoint IDs
    PRIMARY KEY (email)
);

-- API Groups table
CREATE TABLE IF NOT EXISTS api_groups (
    id VARCHAR PRIMARY KEY,
    name VARCHAR NOT NULL,
    description VARCHAR NOT NULL DEFAULT '',
    base VARCHAR NOT NULL DEFAULT 'http://localhost:3000',
    is_default BOOLEAN NOT NULL DEFAULT true
);

-- Modified endpoints table with group reference
CREATE TABLE IF NOT EXISTS endpoints (
    id VARCHAR PRIMARY KEY,
    text VARCHAR NOT NULL,
    description VARCHAR NOT NULL DEFAULT '',
    is_default BOOLEAN NOT NULL DEFAULT true,
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
    required BOOLEAN NOT NULL DEFAULT false,
    FOREIGN KEY (endpoint_id) REFERENCES endpoints(id)
);

-- Parameter alternatives (no change)
CREATE TABLE IF NOT EXISTS parameter_alternatives (
    endpoint_id VARCHAR,
    parameter_name VARCHAR,
    alternative VARCHAR NOT NULL,
    FOREIGN KEY (endpoint_id) REFERENCES endpoints(id)
);
