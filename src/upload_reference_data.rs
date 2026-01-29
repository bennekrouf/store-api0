use crate::app_log;
use crate::endpoint_store::EndpointStore;
use crate::formatter::YamlFormatter;
use crate::models::{UploadReferenceDataRequest, UploadReferenceDataResponse};
use actix_web::{web, HttpResponse, Responder};
use base64::{engine::general_purpose, Engine};
use std::sync::Arc;

/// Detect if content is base64 encoded or plain text
fn is_base64_content(content: &str) -> bool {
    if content.is_empty() {
        return false;
    }

    let trimmed = content.trim();
    // Simple heuristic: if it looks like key-value pairs, it might not be base64
    if trimmed.contains('=') && !trimmed.ends_with('=') {
       // simplistic check, but good enough for now mostly
    }
    
    // Check if check chars are valid base64
    let cleaned: String = content.chars().filter(|c| !c.is_whitespace()).collect();
    if cleaned.len() < 4 {
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
        let cleaned_content = clean_base64_content(content);
        
        if let Ok(bytes) = general_purpose::STANDARD.decode(&cleaned_content) {
            return Ok(bytes);
        }
        if let Ok(bytes) = general_purpose::URL_SAFE.decode(&cleaned_content) {
            return Ok(bytes);
        }
        
        // Try padding
        let mut padded = cleaned_content.clone();
        while padded.len() % 4 != 0 {
            padded.push('=');
        }
        if let Ok(bytes) = general_purpose::STANDARD.decode(&padded) {
            return Ok(bytes);
        }
        
        // If failed to decode, assume it might be plain text? Or error?
        // Let's fallback to treating as bytes
        Ok(content.as_bytes().to_vec())
    } else {
        Ok(content.as_bytes().to_vec())
    }
}

pub async fn upload_reference_data(
    store: web::Data<Arc<EndpointStore>>,
    formatter: web::Data<Arc<YamlFormatter>>,
    upload_data: web::Json<UploadReferenceDataRequest>,
) -> impl Responder {
    app_log!(info, "Received reference data upload request for {}", upload_data.email);

    let file_bytes = match decode_content(&upload_data.file_content) {
        Ok(bytes) => bytes,
        Err(e) => {
             return HttpResponse::BadRequest().json(UploadReferenceDataResponse {
                success: false,
                message: format!("Invalid file content: {}", e),
                data: None,
            });
        }
    };



    // Optimization: If it's a JSON file, try to parse it directly first
    // This avoids the AI formatter overhead for clean data
    if upload_data.file_name.ends_with(".json") {
        if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&file_bytes) {
            app_log!(info, "Valid JSON detected, skipping AI formatting");
            // Save directly
             match store
                .save_reference_data(&upload_data.email, &upload_data.file_name, &json)
                .await
            {
                Ok(ref_data) => {
                    app_log!(info, "Successfully saved reference data");
                    return HttpResponse::Ok().json(UploadReferenceDataResponse {
                        success: true,
                        message: "Reference data uploaded successfully".to_string(),
                        data: Some(ref_data),
                    });
                }
                Err(e) => {
                    app_log!(error, "Failed to save reference data: {}", e);
                     return HttpResponse::InternalServerError().json(UploadReferenceDataResponse {
                        success: false,
                        message: format!("Failed to save to database: {}", e),
                        data: None,
                    });
                }
            }
        }
    }

    // Call ai-uploader to format/extract data
    let formatted_bytes = match formatter
        .format_reference_data(&file_bytes, &upload_data.file_name)
        .await
    {
        Ok(bytes) => bytes,
        Err(e) => {
            app_log!(error, "Failed to format reference data: {}", e);
             return HttpResponse::InternalServerError().json(UploadReferenceDataResponse {
                success: false,
                message: format!("Failed to extract data: {}", e),
                data: None,
            });
        }
    };

    // Parse JSON response from ai-uploader
    let data_json: serde_json::Value = match serde_json::from_slice(&formatted_bytes) {
        Ok(json) => json,
        Err(e) => {
            app_log!(error, "Failed to parse formatted data: {}", e);
            // Try as string if it's not JSON? No, we expect JSON.
             return HttpResponse::InternalServerError().json(UploadReferenceDataResponse {
                success: false,
                message: format!("Failed to parse extracted data: {}", e),
                data: None,
            });
        }
    };

    // Save to DB
    match store
        .save_reference_data(&upload_data.email, &upload_data.file_name, &data_json)
        .await
    {
        Ok(ref_data) => {
            app_log!(info, "Successfully saved reference data");
            HttpResponse::Ok().json(UploadReferenceDataResponse {
                success: true,
                message: "Reference data uploaded successfully".to_string(),
                data: Some(ref_data),
            })
        }
        Err(e) => {
            app_log!(error, "Failed to save reference data: {}", e);
            HttpResponse::InternalServerError().json(UploadReferenceDataResponse {
                success: false,
                message: format!("Failed to save to database: {}", e),
                data: None,
            })
        }
    }
}
