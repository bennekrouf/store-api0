-- Row-level security policies.
--
-- Applied on every start with batch_execute, which runs this file as a single
-- transaction. Postgres has no CREATE POLICY IF NOT EXISTS, so each policy is
-- dropped first: without that, the second start aborts at the first duplicate
-- and every statement below it — including policies added later — silently
-- never runs. The failure surfaces only as a warning in the log.

-- Enable RLS on core multi-tenant tables
ALTER TABLE tenants ENABLE ROW LEVEL SECURITY;
ALTER TABLE tenant_users ENABLE ROW LEVEL SECURITY;
ALTER TABLE api_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE api_usage_logs ENABLE ROW LEVEL SECURITY;
ALTER TABLE credit_transactions ENABLE ROW LEVEL SECURITY;
ALTER TABLE mcp_tools ENABLE ROW LEVEL SECURITY;

-- Global Bypass Policy (for administrative tasks)
-- This allows access if 'app.bypass_rls' is set to 'true'.

-- 1. Tenant Isolation Policy
DROP POLICY IF EXISTS tenant_isolation ON tenants;
CREATE POLICY tenant_isolation ON tenants
    USING (current_setting('app.bypass_rls', true) = 'true' OR id = current_setting('app.current_tenant_id', true));

-- 2. API Key Isolation Policy
DROP POLICY IF EXISTS api_key_isolation ON api_keys;
CREATE POLICY api_key_isolation ON api_keys
    USING (current_setting('app.bypass_rls', true) = 'true' OR tenant_id = current_setting('app.current_tenant_id', true));

-- 3. Usage Log Isolation Policy
DROP POLICY IF EXISTS api_usage_log_isolation ON api_usage_logs;
CREATE POLICY api_usage_log_isolation ON api_usage_logs
    USING (current_setting('app.bypass_rls', true) = 'true' OR tenant_id = current_setting('app.current_tenant_id', true));

-- 4. Credit Transaction Isolation Policy
DROP POLICY IF EXISTS credit_transaction_isolation ON credit_transactions;
CREATE POLICY credit_transaction_isolation ON credit_transactions
    USING (current_setting('app.bypass_rls', true) = 'true' OR tenant_id = current_setting('app.current_tenant_id', true));

-- 5. MCP Tool Isolation Policy
DROP POLICY IF EXISTS mcp_tool_isolation ON mcp_tools;
CREATE POLICY mcp_tool_isolation ON mcp_tools
    USING (current_setting('app.bypass_rls', true) = 'true' OR tenant_id = current_setting('app.current_tenant_id', true));

-- 6. Tenant User Membership Isolation
DROP POLICY IF EXISTS tenant_user_isolation ON tenant_users;
CREATE POLICY tenant_user_isolation ON tenant_users
    USING (current_setting('app.bypass_rls', true) = 'true' OR tenant_id = current_setting('app.current_tenant_id', true));

-- 7. Per-user downstream credential isolation
ALTER TABLE user_downstream_credentials ENABLE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS user_downstream_credential_isolation ON user_downstream_credentials;
CREATE POLICY user_downstream_credential_isolation ON user_downstream_credentials
    USING (current_setting('app.bypass_rls', true) = 'true'
           OR tenant_id = current_setting('app.current_tenant_id', true));
