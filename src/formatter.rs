use reqwest::multipart::{Form, Part};
use std::error::Error;
use std::io::Write;
use tempfile::NamedTempFile;
use tracing::{error, info};
pub struct YamlFormatter {
    formatter_url: String,
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
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        info!(
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

        let form = Form::new().part("file", part);
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
            error!(error = %error_message, "Formatter service returned an error");
            return Err(format!("Failed to format YAML: {}", error_message).into());
        }

        // Get the formatted content
        let formatted_content = response.bytes().await?.to_vec();

        info!("Successfully formatted YAML file");
        Ok(formatted_content)
    }
}
