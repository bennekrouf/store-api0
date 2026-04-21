import os
import psycopg2

db_url = os.environ.get("DATABASE_URL")
if not db_url:
    print("DATABASE_URL not set")
    exit(1)

conn = psycopg2.connect(db_url)
cur = conn.cursor()

print("--- API Groups ---")
cur.execute("SELECT id, name, tenant_id FROM api_groups")
rows = cur.fetchall()
for row in rows:
    print(f"Group: {row[0]} | Name: {row[1]} | Tenant: {row[2]}")

print("\n--- Tenants ---")
cur.execute("SELECT id, name, mcp_client_id FROM tenants")
rows = cur.fetchall()
for row in rows:
    print(f"Tenant: {row[0]} | Name: {row[1]} | MCP Client: {row[2]}")

print("\n--- User Groups ---")
cur.execute("SELECT email, group_id FROM user_groups")
rows = cur.fetchall()
for row in rows:
    print(f"User: {row[0]} | Group: {row[1]}")

cur.close()
conn.close()
