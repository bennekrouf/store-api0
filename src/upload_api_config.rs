use crate::endpoint_store::{generate_id_from_text, ApiGroupWithEndpoints, ApiStorage};
use crate::models::UploadRequest;
use crate::{endpoint_store::EndpointStore, formatter::YamlFormatter, models::UploadResponse};
use actix_web::{web, HttpResponse, Responder};
use base64::{engine::general_purpose, Engine};
use std::sync::Arc;

/// Detect if content is base64 encoded or plain text
fn is_base64_content(content: &str) -> bool {
    // Check if content looks like base64
    if content.is_empty() {
        return false;
    }

    // Base64 content should not contain typical YAML/JSON keywords at the start
    let trimmed = content.trim();
    if trimmed.starts_with("api_groups:")
        || trimmed.starts_with("{")
        || trimmed.starts_with("endpoints:")
    {
        return false;
    }

    // Check if all characters are valid base64 characters
    let cleaned: String = content.chars().filter(|c| !c.is_whitespace()).collect();

    // Base64 should have reasonable length and valid characters
    if cleaned.len() < 10 {
        return false;
    }

    cleaned
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
}

/// Clean and normalize base64 content
fn clean_base64_content(content: &str) -> String {
    content
        .chars()
        .filter(|c| !c.is_whitespace())
        .filter(|&c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        .collect()
}

/// Decode content - handles both base64 and plain text
fn decode_content(content: &str) -> Result<Vec<u8>, String> {
    if is_base64_content(content) {
        // Content appears to be base64 - try to decode it
        let cleaned_content = clean_base64_content(content);

        // Try multiple decoding strategies
        if let Ok(bytes) = general_purpose::STANDARD.decode(&cleaned_content) {
            return Ok(bytes);
        }

        if let Ok(bytes) = general_purpose::URL_SAFE.decode(&cleaned_content) {
            return Ok(bytes);
        }

        if let Ok(bytes) = general_purpose::URL_SAFE_NO_PAD.decode(&cleaned_content) {
            return Ok(bytes);
        }

        // Try adding padding
        let mut padded_content = cleaned_content.clone();
        while padded_content.len() % 4 != 0 {
            padded_content.push('=');
        }

        if let Ok(bytes) = general_purpose::STANDARD.decode(&padded_content) {
            return Ok(bytes);
        }

        Err("Failed to decode base64 content with all strategies".to_string())
    } else {
        // Content appears to be plain text - return as UTF-8 bytes
        Ok(content.as_bytes().to_vec())
    }
}

// Handler for uploading API configuration
pub async fn upload_api_config(
    store: web::Data<Arc<EndpointStore>>,
    formatter: web::Data<Arc<YamlFormatter>>,
    upload_data: web::Json<UploadRequest>,
) -> impl Responder {
    let is_base64 = is_base64_content(&upload_data.file_content);

    tracing::info!(
        email = %upload_data.email,
        filename = %upload_data.file_name,
        original_content_length = upload_data.file_content.len(),
        detected_format = if is_base64 { "base64" } else { "plain_text" },
        "Received HTTP upload request via Actix"
    );

    // Decode content (base64 or plain text)
    let file_bytes = match decode_content(&upload_data.file_content) {
        Ok(bytes) => {
            tracing::info!(
                decoded_size = bytes.len(),
                format_detected = if is_base64 { "base64" } else { "plain_text" },
                "Successfully processed file content"
            );
            bytes
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                content_sample = %upload_data.file_content.chars().take(100).collect::<String>(),
                "Failed to process file content"
            );
            return HttpResponse::BadRequest().json(UploadResponse {
                success: false,
                message: format!("Invalid file content: {}", e),
                imported_count: 0,
                group_count: 0,
            });
        }
    };

    // Convert to UTF-8 string
    let file_content = match String::from_utf8(file_bytes.clone()) {
        Ok(content) => content,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "File content is not valid UTF-8, attempting lossy conversion"
            );

            let lossy_content = String::from_utf8_lossy(&file_bytes);
            if lossy_content.trim().is_empty() {
                return HttpResponse::BadRequest().json(UploadResponse {
                    success: false,
                    message: "File content is empty or not valid text".to_string(),
                    imported_count: 0,
                    group_count: 0,
                });
            }

            tracing::info!("Using lossy UTF-8 conversion");
            lossy_content.to_string()
        }
    };

    // Format the content if it's YAML and formatter is available
    let processed_content =
        if upload_data.file_name.ends_with(".yaml") || upload_data.file_name.ends_with(".yml") {
            match formatter
                .format_yaml(file_content.as_bytes(), &upload_data.file_name)
                .await
            {
                Ok(formatted) => match String::from_utf8(formatted) {
                    Ok(content) => {
                        tracing::info!("Successfully formatted YAML content");
                        content
                    }
                    Err(_) => {
                        tracing::warn!("Formatted content is not valid UTF-8, using original");
                        file_content
                    }
                },
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "Failed to format YAML, proceeding with original content"
                    );
                    file_content
                }
            }
        } else if upload_data.file_name.ends_with(".json") {
            // Pretty print JSON if possible
            match serde_json::from_str::<serde_json::Value>(&file_content) {
                Ok(json_value) => serde_json::to_string_pretty(&json_value).unwrap_or(file_content),
                Err(_) => file_content,
            }
        } else {
            file_content
        };

    // Parse the content based on file extension
    let api_storage =
        if upload_data.file_name.ends_with(".yaml") || upload_data.file_name.ends_with(".yml") {
            match serde_yaml::from_str::<ApiStorage>(&processed_content) {
                Ok(storage) => storage,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        content_preview = %processed_content.chars().take(200).collect::<String>(),
                        "Failed to parse YAML content"
                    );
                    return HttpResponse::BadRequest().json(UploadResponse {
                        success: false,
                        message: format!("Invalid YAML format: {}", e),
                        imported_count: 0,
                        group_count: 0,
                    });
                }
            }
        } else if upload_data.file_name.ends_with(".json") {
            match serde_json::from_str::<ApiStorage>(&processed_content) {
                Ok(storage) => storage,
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        content_preview = %processed_content.chars().take(200).collect::<String>(),
                        "Failed to parse JSON content"
                    );
                    return HttpResponse::BadRequest().json(UploadResponse {
                        success: false,
                        message: format!("Invalid JSON format: {}", e),
                        imported_count: 0,
                        group_count: 0,
                    });
                }
            }
        } else {
            return HttpResponse::BadRequest().json(UploadResponse {
            success: false,
            message:
                "Unsupported file format. Please upload YAML (.yaml/.yml) or JSON (.json) files."
                    .to_string(),
            imported_count: 0,
            group_count: 0,
        });
        };

    // Validate and process API groups
    let group_count = api_storage.api_groups.len();
    if group_count == 0 {
        return HttpResponse::BadRequest().json(UploadResponse {
            success: false,
            message: "No API groups found in the file".to_string(),
            imported_count: 0,
            group_count: 0,
        });
    }

    // After parsing, before processing groups
    for group in &api_storage.api_groups {
        if group.group.base.trim().is_empty() {
            return HttpResponse::BadRequest().json(UploadResponse {
                success: false,
                message: format!("API group '{}' must have a base URL", group.group.name),
                imported_count: 0,
                group_count: 0,
            });
        }

        for endpoint in &group.endpoints {
            if endpoint.base.trim().is_empty() {
                return HttpResponse::BadRequest().json(UploadResponse {
                    success: false,
                    message: format!("Endpoint '{}' must have a base URL", endpoint.text),
                    imported_count: 0,
                    group_count: 0,
                });
            }
        }
    }

    // Process groups and endpoints
    let mut processed_groups = Vec::new();
    for mut group in api_storage.api_groups {
        // Generate ID for group if missing
        if group.group.id.trim().is_empty() {
            group.group.id = generate_id_from_text(&group.group.name);
        }

        // Process endpoints
        let mut processed_endpoints = Vec::new();
        for mut endpoint in group.endpoints {
            // Generate ID for endpoint if missing
            if endpoint.id.trim().is_empty() {
                endpoint.id = generate_id_from_text(&endpoint.text);
            }

            endpoint.group_id = group.group.id.clone();
            processed_endpoints.push(endpoint);
        }

        processed_groups.push(ApiGroupWithEndpoints {
            group: group.group,
            endpoints: processed_endpoints,
        });
    }

    // Save to database
    match store
        .replace_user_api_groups(&upload_data.email, processed_groups)
        .await
    {
        Ok(endpoint_count) => {
            tracing::info!(
                email = %upload_data.email,
                endpoint_count = endpoint_count,
                group_count = group_count,
                "Successfully imported API groups and endpoints"
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
                "Failed to import API groups"
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
