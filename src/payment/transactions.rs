use crate::app_log;
use crate::endpoint_store::EndpointStore;
use actix_web::{web, HttpResponse, Responder};
use std::sync::Arc;

pub async fn get_credit_transactions_handler(
    store: web::Data<Arc<EndpointStore>>,
    tenant_id: web::Path<String>,
) -> impl Responder {
    let mut tenant_id = tenant_id.into_inner();
    app_log!(info, tenant_id = %tenant_id, "Received HTTP get credit transactions request");

    // If tenant_id looks like an email, resolve it to the actual tenant ID
    if tenant_id.contains('@') {
        use crate::endpoint_store::tenant_management;
        match tenant_management::get_default_tenant(&store, &tenant_id).await {
            Ok(t) => {
                app_log!(info, email = %tenant_id, resolved_tenant_id = %t.id, "Resolved email to tenant ID");
                tenant_id = t.id;
            },
            Err(e) => {
                app_log!(error, email = %tenant_id, error = %e, "Failed to resolve tenant for transactions lookup");
                return HttpResponse::InternalServerError().json(serde_json::json!({
                    "success": false,
                    "message": "Account resolution failed"
                }));
            }
        }
    }

    match store.get_credit_transactions(&tenant_id, 50).await {
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
