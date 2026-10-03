use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    // A `grpc` section in an older config.yaml is ignored: serde skips
    // unknown fields, so existing deployments keep starting.
    pub http: HttpServerConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct HttpServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub server: ServerConfig,
    formatter_host: Option<String>,
    formatter_port: Option<u16>,
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        let config: Config = serde_yaml::from_str(&content)?;
        Ok(config)
    }

    pub fn stripe_secret_key(&self) -> String {
        std::env::var("STRIPE_SECRET_KEY").unwrap_or_else(|_| "sk_test_placeholder".to_string())
    }

    pub fn http_host(&self) -> &str {
        &self.server.http.host
    }

    pub fn http_port(&self) -> u16 {
        self.server.http.port
    }

    pub fn formatter_url(&self) -> String {
        let host = self.formatter_host.as_deref().unwrap_or("localhost");
        let port = self.formatter_port.unwrap_or(6001);
        format!("http://{}:{}/format-yaml", host, port)
    }
}

// Default implementation for testing or when config file is missing
impl Default for Config {
    fn default() -> Self {
        Self {
            // whoami: "api-store".to_string(),
            // level: "debug".to_string(),
            server: ServerConfig {
                http: HttpServerConfig {
                    host: "127.0.0.1".to_string(),
                    port: 5007,
                },
            },
            formatter_port: Some(6001),
            formatter_host: Some("localhost".to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_config_that_still_has_a_grpc_section_loads() {
        // Deployed config.yaml files predate the gRPC server's removal.
        let yaml = r#"
server:
  grpc:
    host: "0.0.0.0"
    port: 50057
  http:
    host: "127.0.0.1"
    port: 5007
formatter_port: 6001
"#;
        let config: Config = serde_yaml::from_str(yaml).expect("old config parses");
        assert_eq!(config.http_host(), "127.0.0.1");
        assert_eq!(config.http_port(), 5007);
    }
}
