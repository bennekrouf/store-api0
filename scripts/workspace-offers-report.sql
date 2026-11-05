-- scripts/workspace-offers-report.sql
--
-- Read-only: every workspace with what decides its workspace plan offer —
-- members (seats), who pays, prepaid balance, credits spent in the last 30
-- days, tools published, and whether it already has a workspace plan.
-- Personal workspaces with one member and no spend are left out: they are
-- mayorana app-plan users, not workspace customers, and keep their app plan.
--
--   psql "$DATABASE_URL" -f scripts/workspace-offers-report.sql
--
-- Suggested offer: by seats first, then by monthly spend against each plan's
-- allowance (Starter 3 seats / 20k, Growth 10 / 100k, Business 25 / 500k).

WITH members AS (
    SELECT tenant_id, COUNT(*) AS seats,
           MIN(email) FILTER (WHERE role = 'owner') AS owner
    FROM tenant_users WHERE role IN ('owner', 'admin', 'member')
    GROUP BY tenant_id
),
spend AS (
    SELECT tenant_id, -SUM(amount) FILTER (WHERE amount < 0) AS spent_30d
    FROM credit_transactions WHERE created_at > NOW() - INTERVAL '30 days'
    GROUP BY tenant_id
),
tools AS (
    SELECT tenant_id, COUNT(*) AS tools FROM mcp_tools GROUP BY tenant_id
)
SELECT t.id                         AS tenant_id,
       t.name,
       m.owner,
       COALESCE(m.seats, 0)         AS seats,
       t.billing_mode,
       t.credit_balance,
       COALESCE(s.spent_30d, 0)     AS spent_30d,
       COALESCE(tl.tools, 0)        AS tools,
       ws.plan_id                   AS workspace_plan,
       CASE
           WHEN COALESCE(m.seats, 0) > 25 OR COALESCE(s.spent_30d, 0) > 500000 THEN 'enterprise'
           WHEN COALESCE(m.seats, 0) > 10 OR COALESCE(s.spent_30d, 0) > 100000 THEN 'business'
           WHEN COALESCE(m.seats, 0) > 3  OR COALESCE(s.spent_30d, 0) > 20000  THEN 'growth'
           ELSE 'starter'
       END                          AS suggested
FROM tenants t
LEFT JOIN members m  ON m.tenant_id = t.id
LEFT JOIN spend s    ON s.tenant_id = t.id
LEFT JOIN tools tl   ON tl.tenant_id = t.id
LEFT JOIN workspace_subscriptions ws
       ON ws.tenant_id = t.id AND ws.status IN ('active', 'trialing') AND ws.period_end > NOW()
WHERE COALESCE(m.seats, 0) > 1 OR COALESCE(s.spent_30d, 0) > 0 OR COALESCE(tl.tools, 0) > 0
ORDER BY spent_30d DESC, seats DESC;
