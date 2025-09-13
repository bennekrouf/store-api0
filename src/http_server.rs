use crate::add_api_group::add_api_group;
use crate::delete_api_group::delete_api_group;
use crate::endpoint_store::EndpointStore;
use crate::formatter::YamlFormatter;
use crate::generate_api_key::generate_api_key;
use crate::get_api_groups::get_api_groups;
use crate::get_api_key_usage::get_api_key_usage;
use crate::get_api_keys_status::get_api_keys_status;
use crate::get_credit_balance_handler::get_credit_balance_handler;
use crate::get_user_preferences::get_user_preferences;
use crate::health_check::health_check;
use crate::manage_endpoint::manage_endpoint;
use crate::record_api_key_usage::record_api_key_usage;
use crate::reset_user_preferences::reset_user_preferences;
use crate::revoke_all_api_keys_handler::revoke_all_api_keys_handler;
use crate::revoke_api_key_handler::revoke_api_key_handler;
use crate::update_api_group::update_api_group;
use crate::update_credit_balance_handler::update_credit_balance_handler;
use crate::update_user_preferences::update_user_preferences;
use crate::upload_api_config::upload_api_config;
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
    host: &str,
    port: u16,
) -> std::io::Result<()> {
    let addr = format!("{}:{}", host, port);
    let addr = addr.parse::<SocketAddr>().unwrap();
    let store_clone = store.clone();
    let formatter_clone = formatter.clone();

    // Run Actix Web in a blocking task to avoid Send issues
    let _ = task::spawn_blocking(move || {
        let sys = actix_web::rt::System::new();
        sys.block_on(async move {
            tracing::info!("Starting HTTP server at {}", addr);

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
                    .app_data(web::Data::new(formatter_clone.clone())) // Add formatter to app data
                    .service(
                        web::scope("/api")
                            // API groups endpoints
                            .route("/upload", web::post().to(upload_api_config))
                            .route("/key/usage", web::post().to(record_api_key_usage))
                            .route("/groups/{email}", web::get().to(get_api_groups))
                            .route("/group", web::post().to(add_api_group))
                            .route("/group", web::put().to(update_api_group))
                            .route("/endpoint", web::post().to(manage_endpoint))
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
                            // Key validation and usage
                            .route("/key/validate", web::post().to(validate_api_key))
                            .route(
                                "/key/usage/{email}/{key_id}",
                                web::get().to(get_api_key_usage),
                            )
                            .route("/health", web::get().to(health_check)),
                    )
                // Credit balance endpoints
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
