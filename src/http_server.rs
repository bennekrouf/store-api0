use crate::add_api_group::add_api_group;
use crate::downstream_auth_handler::{
    get_downstream_auth_handler, get_downstream_auth_by_id_handler, save_downstream_auth_handler,
};
use crate::admin_credit_handler::admin_credit_handler;
use crate::generate_consumer_key_handler::generate_consumer_key_handler;
use crate::mcp_tools_handler::{
    delete_mcp_tool_handler, get_mcp_tool_handler, list_mcp_tools_handler,
    upsert_mcp_tool_handler,
};
use crate::app_log;
use crate::delete_api_group::delete_api_group;
use crate::delete_endpoint::delete_endpoint;
use crate::endpoint_store::EndpointStore;
use crate::formatter::YamlFormatter;
use crate::generate_api_key::generate_api_key;
use crate::get_api_groups::get_api_groups;
use crate::get_api_key_usage::get_api_key_usage;
use crate::get_api_keys_status::get_api_keys_status;
use crate::get_api_usage_logs::get_api_usage_logs;
use crate::get_authorized_domains::get_authorized_domains;
use crate::get_credit_balance_handler::get_credit_balance_handler;
use crate::get_credit_transactions_handler::get_credit_transactions_handler;
use crate::get_user_preferences::get_user_preferences;
use crate::health_check;
use crate::log_api_usage::log_api_usage;
use crate::manage_endpoint::manage_endpoint;
use crate::payment_handler::{
    confirm_payment_handler, create_payment_intent_handler, get_payment_history_handler,
};
use crate::payment_service::PaymentService;
use crate::reset_user_preferences::reset_user_preferences;
use crate::revoke_all_api_keys_handler::revoke_all_api_keys_handler;
use crate::revoke_api_key_handler::revoke_api_key_handler;
use crate::update_api_group::update_api_group;
use crate::update_credit_balance_handler::update_credit_balance_handler;
use crate::update_user_preferences::update_user_preferences;

use crate::upload_api_config::upload_api_config;
use crate::upload_reference_data;
use crate::validate_api_key::validate_api_key;
use actix_cors::Cors;
use actix_web::middleware::Logger;
use actix_web::{web, App, HttpServer};
use std::sync::Arc;
// use actix_web::{web, HttpResponse, Responder};
use std::net::SocketAddr;
use tokio::task;

// Server startup function
pub async fn start_http_server(
    store: Arc<EndpointStore>,
    formatter: Arc<YamlFormatter>,
    payment_service: Arc<PaymentService>,
    firebase_project_id: String,
    host: &str,
    port: u16,
) -> std::io::Result<()> {
    let addr = format!("{}:{}", host, port);
    let addr = addr.parse::<SocketAddr>().unwrap();
    let store_clone = store.clone();
    let formatter_clone = formatter.clone();
    let payment_service_clone = payment_service.clone();
    let firebase_project_id_clone = firebase_project_id.clone();

    // Run Actix Web in a blocking task to avoid Send issues
    let _ = task::spawn_blocking(move || {
        let sys = actix_web::rt::System::new();
        sys.block_on(async move {
            app_log!(info, "Starting HTTP server at {}", addr);

            HttpServer::new(move || {
                // Configure CORS
                let cors = Cors::default()
                    .allow_any_origin()
                    .allow_any_method()
                    .allow_any_header()
                    .max_age(3600);

                App::new()
                    .wrap(Logger::default())
                    .wrap(cors)
                    // .wrap(ApiKeyAuth::new(store_clone.clone()))
                    .app_data(web::Data::new(store_clone.clone()))
                    .app_data(web::Data::new(formatter_clone.clone()))
                    .app_data(web::Data::new(payment_service_clone.clone()))
                    // Firebase project ID — used by AdminUser extractor
                    .app_data(web::Data::new(firebase_project_id_clone.clone()))
                    .service(
                        web::scope("/api")
                            // API groups endpoints
                            .route("/upload", web::post().to(upload_api_config))
                            .route(
                                "/reference-data/upload",
                                web::post().to(upload_reference_data::upload_reference_data),
                            )
                            .route("/groups/{email}", web::get().to(get_api_groups))
                            .route("/group", web::post().to(add_api_group))
                            .route("/group", web::put().to(update_api_group))
                            .route("/endpoint", web::post().to(manage_endpoint))
                            .route("/domains/authorized", web::get().to(get_authorized_domains))
                            .route("/user/usage/log", web::post().to(log_api_usage))
                            .route(
                                "/user/usage/logs/{email}/{key_id}",
                                web::get().to(get_api_usage_logs),
                            )
                            .route(
                                "/groups/{email}/{group_id}",
                                web::delete().to(delete_api_group),
                            )
                            // User preferences endpoints
                            .route(
                                "/user/preferences/{email}",
                                web::get().to(get_user_preferences),
                            )
                            .route("/user/preferences", web::post().to(update_user_preferences))
                            .route(
                                "/user/preferences/{email}",
                                web::delete().to(reset_user_preferences),
                            )
                            // Updated API key endpoints
                            .route("/user/keys/{email}", web::get().to(get_api_keys_status))
                            .route("/user/keys", web::post().to(generate_api_key))
                            .route(
                                "/user/keys/{email}/{key_id}",
                                web::delete().to(revoke_api_key_handler),
                            )
                            .route(
                                "/user/keys/{email}",
                                web::delete().to(revoke_all_api_keys_handler),
                            )
                            // Credit balance endpoints
                            .route(
                                "/user/credits/{email}",
                                web::get().to(get_credit_balance_handler),
                            )
                            .route(
                                "/user/credits",
                                web::post().to(update_credit_balance_handler),
                            )
                            .route(
                                "/user/credit-transactions/{email}",
                                web::get().to(get_credit_transactions_handler),
                            )
                            // Key validation and usage
                            .route("/key/validate", web::post().to(validate_api_key))
                            .route(
                                "/endpoints/{email}/{endpoint_id}",
                                web::delete().to(delete_endpoint),
                            )
                            .route(
                                "/key/usage/{email}/{key_id}",
                                web::get().to(get_api_key_usage),
                            )
                            .route("/health", web::get().to(health_check::health_check))
                            // Payment (Stripe) endpoints
                            .route("/payments/intent", web::post().to(create_payment_intent_handler))
                            .route("/payments/confirm", web::post().to(confirm_payment_handler))
                            .route("/payments/history/{email}", web::get().to(get_payment_history_handler))
                            // Admin endpoints (Firebase JWT, admin email only)
                            .route("/admin/credits", web::post().to(admin_credit_handler))
                            // MCP tool registry
                            .route("/mcp-tools", web::post().to(upsert_mcp_tool_handler))
                            .route("/mcp-tools/{tenant_id}", web::get().to(list_mcp_tools_handler))
                            .route("/mcp-tools/{tenant_id}/{tool_name}", web::get().to(get_mcp_tool_handler))
                            .route("/mcp-tools/{tenant_id}/{tool_name}", web::delete().to(delete_mcp_tool_handler))
                            // Consumer key generation (B2B2C)
                            .route("/consumer-keys", web::post().to(generate_consumer_key_handler))
                            // Downstream auth (tenant-level)
                            .route("/user/downstream-auth", web::get().to(get_downstream_auth_handler))
                            .route("/user/downstream-auth", web::put().to(save_downstream_auth_handler))
                            // Internal: gateway uses tenant_id directly
                            .route("/tenant/downstream-auth/{tenant_id}", web::get().to(get_downstream_auth_by_id_handler)),
                    )
            })
            .bind(addr)?
            .workers(1)
            .run()
            .await
        })
    })
    .await
    .expect("Actix system panicked");

    Ok(())
}
