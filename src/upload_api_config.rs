use crate::endpoint_store::{generate_id_from_text, ApiGroupWithEndpoints, ApiStorage};
use crate::models::UploadRequest;
use crate::{endpoint_store::EndpointStore, formatter::YamlFormatter, models::UploadResponse};
use actix_web::{web, HttpResponse, Responder};
use base64::{engine::general_purpose, Engine};
use std::sync::Arc;

// Handler for uploading API configuration
pub async fn upload_api_config(
    store: web::Data<Arc<EndpointStore>>,
    formatter: web::Data<YamlFormatter>,
    upload_data: web::Json<UploadRequest>,
) -> impl Responder {
    tracing::info!(
        email = %upload_data.email,
        filename = %upload_data.file_name,
        "Received HTTP upload request via Actix"
    );

    // Decode base64 content
    let file_bytes = match general_purpose::STANDARD.decode(&upload_data.file_content) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::error!(error = %e, "Failed to decode base64 content");
            return HttpResponse::BadRequest().json(UploadResponse {
                success: false,
                message: format!("Invalid base64 encoding: {}", e),
                imported_count: 0,
                group_count: 0,
            });
        }
    };

    // Format the file if it's YAML
    let file_content =
        if upload_data.file_name.ends_with(".yaml") || upload_data.file_name.ends_with(".yml") {
            // Format the YAML content
            match formatter
                .format_yaml(&file_bytes, &upload_data.file_name)
                .await
            {
                Ok(formatted) => match String::from_utf8(formatted) {
                    Ok(content) => content,
                    Err(e) => {
                        tracing::error!(error = %e, "Formatted content is not valid UTF-8");
                        return HttpResponse::InternalServerError().json(UploadResponse {
                            success: false,
                            message: "Error processing formatted content".to_string(),
                            imported_count: 0,
                            group_count: 0,
                        });
                    }
                },
                Err(e) => {
                    tracing::error!(error = %e, "Failed to format YAML");
                    // Continue with original content
                    match String::from_utf8(file_bytes) {
                        Ok(content) => content,
                        Err(e) => {
                            tracing::error!(error = %e, "File content is not valid UTF-8");
                            return HttpResponse::BadRequest().json(UploadResponse {
                                success: false,
                                message: "File content must be valid UTF-8 text".to_string(),
                                imported_count: 0,
                                group_count: 0,
                            });
                        }
                    }
                }
            }
        } else {
            // Same as before for non-YAML files
            match String::from_utf8(file_bytes) {
                Ok(content) => content,
                Err(e) => {
                    tracing::error!(error = %e, "File content is not valid UTF-8");
                    return HttpResponse::BadRequest().json(UploadResponse {
                        success: false,
                        message: "File content must be valid UTF-8 text".to_string(),
                        imported_count: 0,
                        group_count: 0,
                    });
                }
            }
        };

    // // Convert to string
    // let file_content = match String::from_utf8(file_bytes) {
    //     Ok(content) => content,
    //     Err(e) => {
    //         tracing::error!(error = %e, "File content is not valid UTF-8");
    //         return HttpResponse::BadRequest().json(UploadResponse {
    //             success: false,
    //             message: "File content must be valid UTF-8 text".to_string(),
    //             imported_count: 0,
    //             group_count: 0,
    //         });
    //     }
    // };

    // Parse based on file extension
    let api_storage =
        if upload_data.file_name.ends_with(".yaml") || upload_data.file_name.ends_with(".yml") {
            // Parse YAML
            match serde_yaml::from_str::<ApiStorage>(&file_content) {
                Ok(storage) => storage,
                Err(e) => {
                    tracing::error!(error = %e, "Failed to parse YAML content");
                    return HttpResponse::BadRequest().json(UploadResponse {
                        success: false,
                        message: "Invalid YAML format. Expected structure with 'api_groups'."
                            .to_string(),
                        imported_count: 0,
                        group_count: 0,
                    });
                }
            }
        } else {
            return HttpResponse::BadRequest().json(UploadResponse {
                success: false,
                message: "Unsupported file format. Use YAML or JSON.".to_string(),
                imported_count: 0,
                group_count: 0,
            });
        };

    // Process and validate each API group
    let group_count = api_storage.api_groups.len();

    if group_count == 0 {
        return HttpResponse::BadRequest().json(UploadResponse {
            success: false,
            message: "No API groups found in the file".to_string(),
            imported_count: 0,
            group_count: 0,
        });
    }

    // Generate IDs for groups and endpoints if needed
    let mut processed_groups = Vec::new();

    for mut group in api_storage.api_groups {
        // Generate ID for group if not provided
        if group.group.id.is_empty() {
            group.group.id = generate_id_from_text(&group.group.name);
        }

        // Process endpoints
        let mut processed_endpoints = Vec::new();
        for mut endpoint in group.endpoints {
            // Generate ID for endpoint if not provided
            if endpoint.id.is_empty() {
                endpoint.id = generate_id_from_text(&endpoint.text);
            }

            // Set group_id reference
            endpoint.group_id = group.group.id.clone();

            processed_endpoints.push(endpoint);
        }

        let processed_group = ApiGroupWithEndpoints {
            group: group.group,
            endpoints: processed_endpoints,
        };

        processed_groups.push(processed_group);
    }

    // Replace user API groups
    match store
        .replace_user_api_groups(&upload_data.email, processed_groups)
        .await
    {
        Ok(endpoint_count) => {
            tracing::info!(
                email = %upload_data.email,
                endpoint_count = endpoint_count,
                group_count = group_count,
                "Successfully imported API groups and endpoints via HTTP API"
            );
            HttpResponse::Ok().json(UploadResponse {
                success: true,
                message: "API groups and endpoints successfully imported".to_string(),
                imported_count: endpoint_count as i32,
                group_count: group_count as i32,
            })
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                email = %upload_data.email,
                "Failed to import API groups via HTTP API"
            );
            HttpResponse::InternalServerError().json(UploadResponse {
                success: false,
                message: format!("Failed to import API groups: {}", e),
                imported_count: 0,
                group_count: 0,
            })
        }
    }
}
