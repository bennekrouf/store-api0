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
CREATE POLICY tenant_isolation ON tenants
    USING (current_setting('app.bypass_rls', true) = 'true' OR id = current_setting('app.current_tenant_id', true));

-- 2. API Key Isolation Policy
CREATE POLICY api_key_isolation ON api_keys
    USING (current_setting('app.bypass_rls', true) = 'true' OR tenant_id = current_setting('app.current_tenant_id', true));

-- 3. Usage Log Isolation Policy
CREATE POLICY api_usage_log_isolation ON api_usage_logs
    USING (current_setting('app.bypass_rls', true) = 'true' OR tenant_id = current_setting('app.current_tenant_id', true));

-- 4. Credit Transaction Isolation Policy
CREATE POLICY credit_transaction_isolation ON credit_transactions
    USING (current_setting('app.bypass_rls', true) = 'true' OR tenant_id = current_setting('app.current_tenant_id', true));

-- 5. MCP Tool Isolation Policy
CREATE POLICY mcp_tool_isolation ON mcp_tools
    USING (current_setting('app.bypass_rls', true) = 'true' OR tenant_id = current_setting('app.current_tenant_id', true));

-- 6. Tenant User Membership Isolation
CREATE POLICY tenant_user_isolation ON tenant_users
    USING (current_setting('app.bypass_rls', true) = 'true' OR tenant_id = current_setting('app.current_tenant_id', true));
