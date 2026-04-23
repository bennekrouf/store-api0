-- Enable RLS on core multi-tenant tables
ALTER TABLE tenants ENABLE ROW LEVEL SECURITY;
ALTER TABLE tenant_users ENABLE ROW LEVEL SECURITY;
ALTER TABLE api_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE api_usage_logs ENABLE ROW LEVEL SECURITY;
ALTER TABLE credit_transactions ENABLE ROW LEVEL SECURITY;
ALTER TABLE mcp_tools ENABLE ROW LEVEL SECURITY;

-- 1. Tenant Isolation Policy
-- Users can only see/modify their own tenant
CREATE POLICY tenant_isolation ON tenants
    USING (id = current_setting('app.current_tenant_id', true));

-- 2. API Key Isolation Policy
CREATE POLICY api_key_isolation ON api_keys
    USING (tenant_id = current_setting('app.current_tenant_id', true));

-- 3. Usage Log Isolation Policy
CREATE POLICY api_usage_log_isolation ON api_usage_logs
    USING (tenant_id = current_setting('app.current_tenant_id', true));

-- 4. Credit Transaction Isolation Policy
CREATE POLICY credit_transaction_isolation ON credit_transactions
    USING (tenant_id = current_setting('app.current_tenant_id', true));

-- 5. MCP Tool Isolation Policy
CREATE POLICY mcp_tool_isolation ON mcp_tools
    USING (tenant_id = current_setting('app.current_tenant_id', true));

-- 6. Tenant User Membership Isolation
-- This one is tricky: we usually need to see our memberships to find our tenants.
-- However, if we've already resolved the tenant_id in the app, we can just check it.
CREATE POLICY tenant_user_isolation ON tenant_users
    USING (tenant_id = current_setting('app.current_tenant_id', true));

-- Special "Internal" Bypass
-- For certain operations (like listing a user's tenants or verifying access during login),
-- the application needs to query WITHOUT a specific tenant_id set.
-- We can create a policy that allows access if 'app.bypass_rls' is set to 'true'.
-- OR we can just use the superuser/owner for these (default behavior).
-- But safer to have an explicit bypass variable.

CREATE POLICY bypass_rls_tenants ON tenants
    FOR SELECT
    USING (current_setting('app.bypass_rls', true) = 'true');

CREATE POLICY bypass_rls_tenant_users ON tenant_users
    FOR SELECT
    USING (current_setting('app.bypass_rls', true) = 'true');
