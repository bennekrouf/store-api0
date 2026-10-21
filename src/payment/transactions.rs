use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use serde::Deserialize;
use std::sync::Arc;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 1000;

#[derive(Deserialize)]
pub struct TransactionsQuery {
    limit: Option<i64>,
}

pub async fn get_credit_transactions_handler(
    store: web::Data<Arc<EndpointStore>>,
    tenant_id: web::Path<String>,
    query: web::Query<TransactionsQuery>,
) -> impl Responder {
    let limit = query.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let mut tenant_id = tenant_id.into_inner();
    app_log!(info, tenant_id = %tenant_id, "Received HTTP get credit transactions request");

    // If tenant_id looks like an email, resolve it to the actual tenant ID
    if tenant_id.contains('@') {
        use crate::endpoint_store::tenant_management;
        // find_ rather than get_: listing transactions must not create an account.
        match tenant_management::find_default_tenant(&store, &tenant_id).await {
            Ok(Some(t)) => {
                app_log!(info, email = %tenant_id, resolved_tenant_id = %t.id, "Resolved email to tenant ID");
                tenant_id = t.id;
            }
            // No account means no transactions — an empty list, same shape.
            Ok(None) => {
                app_log!(info, email = %tenant_id, "No tenant for this email; reporting no transactions");
                return HttpResponse::Ok().json(serde_json::json!({
                    "success": true,
                    "transactions": []
                }));
            }
            Err(e) => {
                app_log!(error, email = %tenant_id, error = %e, "Failed to resolve tenant for transactions lookup");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": "Account resolution failed"
                }));
            }
        }
    }

    match store.get_credit_transactions(&tenant_id, limit).await {
        Ok(transactions) => {
            HttpResponse::Ok().json(serde_json::json!({
                "success": true,
                "transactions": transactions,
            }))
        }
        Err(e) => {
            app_log!(error, error = %e, tenant_id = %tenant_id, "Failed to retrieve credit transactions");
            HttpResponse::InternalServerError().json(serde_json::json!({
                "success": false,
                "message": format!("Error retrieving credit transactions: {}", e),
            }))
        }
    }
}
