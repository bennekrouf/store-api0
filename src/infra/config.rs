use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Clone)]
pub struct ServerConfig {
    pub grpc: GrpcServerConfig,
    pub http: HttpServerConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GrpcServerConfig {
    pub host: String,
    pub port: u16,
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

    pub fn grpc_address(&self) -> String {
        format!("{}:{}", self.server.grpc.host, self.server.grpc.port)
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
            // output: "grpc".to_string(),
            // level: "debug".to_string(),
            server: ServerConfig {
                grpc: GrpcServerConfig {
                    host: "0.0.0.0".to_string(),
                    port: 50055,
                },
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
