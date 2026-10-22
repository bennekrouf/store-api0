use crate::admin::model_config::UploaderLlm;
use graflog::{app_log, app_span};
use reqwest::multipart::{Form, Part};
use std::error::Error;
use std::io::Write;
use tempfile::NamedTempFile;

/// What the uploader's AI call spent, read from its X-LLM-* response headers.
/// Tokens the provider left out are estimated from the sizes (≈ 4 chars a token).
#[derive(Debug, Clone, PartialEq)]
pub struct LlmSpend {
    pub provider: String,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub estimated: bool,
    pub response_time_ms: i64,
}

fn llm_spend(headers: &reqwest::header::HeaderMap, sent: usize, received: usize, started: std::time::Instant) -> Option<LlmSpend> {
    let text = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    let tokens = |name: &str| text(name).and_then(|v| v.parse::<i64>().ok());
    // An uploader older than these headers: nothing to record.
    let model = text("X-LLM-Model")?;
    let counted = tokens("X-LLM-Input-Tokens").zip(tokens("X-LLM-Output-Tokens"));
    let (input_tokens, output_tokens) = counted.unwrap_or(((sent / 4) as i64, (received / 4) as i64));
    Some(LlmSpend {
        provider: text("X-LLM-Provider").unwrap_or_default(),
        model,
        input_tokens,
        output_tokens,
        estimated: counted.is_none(),
        response_time_ms: started.elapsed().as_millis() as i64,
    })
}

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
    /// * The formatted content as bytes, and what its AI call spent
    pub async fn format_yaml(
        &self,
        content: &[u8],
        filename: &str,
        llm: &UploaderLlm,
    ) -> Result<(Vec<u8>, Option<LlmSpend>), Box<dyn Error>> {
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
        let started = std::time::Instant::now();
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
        let headers = response.headers().clone();
        let formatted_content = response.bytes().await?.to_vec();
        let spend = llm_spend(&headers, content.len(), formatted_content.len(), started);

        app_log!(info, "Successfully formatted YAML file");
        Ok((formatted_content, spend))
    }

    pub async fn format_reference_data(
        &self,
        content: &[u8],
        filename: &str,
        llm: &UploaderLlm,
    ) -> Result<(Vec<u8>, Option<LlmSpend>), Box<dyn Error>> {
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

        let started = std::time::Instant::now();
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

        let headers = response.headers().clone();
        let formatted_content = response.bytes().await?.to_vec();
        let spend = llm_spend(&headers, content.len(), formatted_content.len(), started);
        app_log!(info, "Successfully formatted reference data");
        Ok((formatted_content, spend))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::{HeaderMap, HeaderValue};

    #[test]
    fn the_uploaders_headers_give_the_calls_model_and_tokens() {
        let mut h = HeaderMap::new();
        h.insert("X-LLM-Provider", HeaderValue::from_static("deepseek"));
        h.insert("X-LLM-Model", HeaderValue::from_static("deepseek-chat"));
        h.insert("X-LLM-Input-Tokens", HeaderValue::from_static("1200"));
        h.insert("X-LLM-Output-Tokens", HeaderValue::from_static("300"));
        let s = llm_spend(&h, 10, 10, std::time::Instant::now()).unwrap();
        assert_eq!((s.provider.as_str(), s.model.as_str(), s.input_tokens, s.output_tokens, s.estimated),
            ("deepseek", "deepseek-chat", 1200, 300, false));
    }

    #[test]
    fn missing_token_counts_are_estimated_and_no_model_means_nothing_to_record() {
        let mut h = HeaderMap::new();
        h.insert("X-LLM-Model", HeaderValue::from_static("mistral-medium-latest"));
        let s = llm_spend(&h, 4000, 800, std::time::Instant::now()).unwrap();
        assert_eq!((s.input_tokens, s.output_tokens, s.estimated), (1000, 200, true));
        assert!(llm_spend(&HeaderMap::new(), 4000, 800, std::time::Instant::now()).is_none());
    }
}
