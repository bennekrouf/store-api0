use crate::admin::model_config::UploaderLlm;
use graflog::{app_log, app_span};
use reqwest::multipart::{Form, Part};
use std::error::Error;
use std::io::Write;
use tempfile::NamedTempFile;

#[derive(Clone)]
pub struct YamlFormatter {
    formatter_url: String,
}

/// The provider, model and key go with the file, so the uploader formats with
/// what the super admin chose in the dashboard and needs no key of its own.
/// Fields come before the file: the uploader reads them in order.
fn form_with(llm: &UploaderLlm, part: Part) -> Form {
    let mut form = Form::new()
        .text("provider", llm.provider.clone())
        .text("model", llm.model.clone());
    if let Some(key) = &llm.api_key {
        form = form.text("api_key", key.clone());
    }
    form.part("file", part)
}

impl YamlFormatter {
    pub fn new(formatter_url: &str) -> Self {
        Self {
            formatter_url: formatter_url.to_string(),
        }
    }

    /// Format a YAML file using the formatter service
    ///
    /// # Arguments
    /// * `content` - The file content as bytes
    /// * `filename` - The original filename (for logging purposes)
    ///
    /// # Returns
    /// * `Result<Vec<u8>, Box<dyn Error>>` - The formatted content as bytes
    pub async fn format_yaml(
        &self,
        content: &[u8],
        filename: &str,
        llm: &UploaderLlm,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        app_span!(
            "format_yaml_file",
            filename = %filename,
            "Formatting YAML file through formatter service"
        );

        // Create a temporary file with the content
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(content)?;
        let _temp_path = temp_file.path().to_string_lossy().to_string();

        // Create a multipart form with the file
        // Create a multipart form with the content directly
        let part = Part::bytes(content.to_vec()).file_name(filename.to_string());

        let form = form_with(llm, part);
        // Send the request to the formatter service
        let client = reqwest::Client::new();
        let response = client
            .post(&self.formatter_url)
            .multipart(form)
            .send()
            .await?;

        // Check if request was successful
        if !response.status().is_success() {
            let error_message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error from formatter service".to_string());
            app_span!("format_yaml_file", error = %error_message, "Formatter service returned an error");
            return Err(format!("Failed to format YAML: {}", error_message).into());
        }

        // Get the formatted content
        let formatted_content = response.bytes().await?.to_vec();

        app_log!(info, "Successfully formatted YAML file");
        Ok(formatted_content)
    }

    pub async fn format_reference_data(
        &self,
        content: &[u8],
        filename: &str,
        llm: &UploaderLlm,
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        app_span!(
            "format_reference_data",
            filename = %filename,
            "Formatting reference data through formatter service"
        );

        let part = Part::bytes(content.to_vec()).file_name(filename.to_string());
        let form = form_with(llm, part);

        // Construct the URL for reference data formatting
        // Assuming the formatter_url is something like "http://localhost:6666/format-yaml"
        // We need to change the endpoint to "/format-reference-data"
        let base_url = self.formatter_url.replace("/format-yaml", "");
        let url = format!("{}/format-reference-data", base_url);

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .multipart(form)
            .send()
            .await?;

        if !response.status().is_success() {
            let error_message = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error from formatter service".to_string());
            app_span!("format_reference_data", error = %error_message, "Formatter service returned an error");
            return Err(format!("Failed to format reference data: {}", error_message).into());
        }

        let formatted_content = response.bytes().await?.to_vec();
        app_log!(info, "Successfully formatted reference data");
        Ok(formatted_content)
    }
}
