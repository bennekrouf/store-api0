use crate::api::tenant_usage::get_tenant_stats;
use crate::api::group_add::add_api_group;
use crate::mcp::downstream_auth::{
    get_downstream_auth_handler, get_downstream_auth_by_id_handler, save_downstream_auth_handler,
};
use crate::mcp::client_id::{get_by_client_id_handler, set_client_id_handler};
use crate::payment::admin::admin_credit_handler;
use crate::api::key_consumer::generate_consumer_key_handler;
use crate::api::providers::list_providers_handler;
use crate::api::key_consumer_self_service::{
    generate_self_service_key, list_self_service_keys,
};
use crate::mcp::tools::{
    delete_mcp_tool_handler, get_mcp_tool_handler, list_mcp_tools_handler,
    upsert_mcp_tool_handler,
};
use crate::app_log;
use crate::api::group_delete::delete_api_group;
use crate::api::endpoint_delete::delete_endpoint;
use crate::endpoint_store::EndpointStore;
use crate::infra::formatter::YamlFormatter;
use crate::api::key_generate::generate_api_key;
use crate::api::group_get::get_api_groups;
use crate::api::usage_key::get_api_key_usage;
use crate::api::key_status::get_api_keys_status;
use crate::api::usage_get_logs::get_api_usage_logs;
use crate::api::domains::get_authorized_domains;
use crate::payment::balance::get_credit_balance_handler;
use crate::payment::transactions::get_credit_transactions_handler;
use crate::user::get::get_user_preferences;
use crate::infra::health;
use crate::api::usage_log::log_api_usage;
use crate::api::endpoint_manage::manage_endpoint;
use crate::payment::payment::{
    confirm_payment_handler, create_payment_intent_handler, get_payment_history_handler,
};
use crate::payment::service::PaymentService;
use crate::user::reset::reset_user_preferences;
use crate::api::key_revoke_all::revoke_all_api_keys_handler;
use crate::api::key_revoke::revoke_api_key_handler;
use crate::api::group_update::update_api_group;
use crate::payment::update_balance::update_credit_balance_handler;
use crate::user::update::update_user_preferences;
use crate::api::tenant_name::update_tenant_name_handler;
use crate::api::config_upload::upload_api_config;
use crate::api::reference_upload;
use crate::api::key_validate::validate_api_key;
use crate::middleware::error_handler::handle_internal_server_error;
use actix_cors::Cors;
use actix_web::middleware::{ErrorHandlers, Logger};
use actix_web::{http::StatusCode, web, App, HttpServer};
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
                    .wrap(
                        ErrorHandlers::new()
                            .handler(StatusCode::INTERNAL_SERVER_ERROR, handle_internal_server_error)
                    )
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
                                web::post().to(reference_upload::upload_reference_data),
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
                            // Alias used by the gateway's OAuth token-exchange flow
                            .route("/key/generate", web::post().to(generate_api_key))
                            .route(
                                "/endpoints/{email}/{endpoint_id}",
                                web::delete().to(delete_endpoint),
                            )
                            .route(
                                "/key/usage/{email}/{key_id}",
                                web::get().to(get_api_key_usage),
                            )
                            .route("/health", web::get().to(health::health_check))
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
                            // Consumer key generation (B2B2C — internal, requires X-Internal-Secret)
                            .route("/consumer-keys", web::post().to(generate_consumer_key_handler))
                            // Self-service consumer keys (end-users, Firebase JWT auth)
                            .route("/consumer-keys/me", web::post().to(generate_self_service_key))
                            .route("/consumer-keys/me", web::get().to(list_self_service_keys))
                            // Provider discovery (public)
                            .route("/providers", web::get().to(list_providers_handler))
                            // Downstream auth (tenant-level)
                            .route("/user/downstream-auth", web::get().to(get_downstream_auth_handler))
                            .route("/user/downstream-auth", web::put().to(save_downstream_auth_handler))
                            .route("/user/tenant/name", web::put().to(update_tenant_name_handler))
                            // Internal: gateway uses tenant_id directly
                            .route("/tenant/downstream-auth/{tenant_id}", web::get().to(get_downstream_auth_by_id_handler))
                            // Per-provider OAuth client ID resolution
                            .route("/tenant/by-client-id/{client_id}", web::get().to(get_by_client_id_handler))
                            .route("/user/mcp-client-id", web::put().to(set_client_id_handler))
                            // Governance / stats: all MCP requests for the tenant
                            .route("/tenant/stats/{email}", web::get().to(get_tenant_stats)),
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
