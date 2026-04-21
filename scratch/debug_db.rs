use tokio_postgres::NoTls;
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_url = env::var("DATABASE_URL").expect("DATABASE_URL not set");
    let (client, connection) = tokio_postgres::connect(&db_url, NoTls).await?;

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    println!("--- API Groups ---");
    let rows = client.query("SELECT id, name, tenant_id FROM api_groups", &[]).await?;
    for row in rows {
        let id: String = row.get(0);
        let name: String = row.get(1);
        let tenant_id: Option<String> = row.get(2);
        println!("Group: {} | Name: {} | Tenant: {:?}", id, name, tenant_id);
    }

    println!("\n--- Tenants ---");
    let rows = client.query("SELECT id, name, mcp_client_id FROM tenants", &[]).await?;
    for row in rows {
        let id: String = row.get(0);
        let name: String = row.get(1);
        let mcp_client_id: Option<String> = row.get(2);
        println!("Tenant: {} | Name: {} | MCP Client: {:?}", id, name, mcp_client_id);
    }

    Ok(())
}
