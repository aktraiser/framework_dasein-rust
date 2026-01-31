//! MCP Client configuration.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::MCPError;

/// Transport type for MCP server connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransportType {
    /// HTTP/SSE transport (for remote servers like Context7)
    Http,
    /// Stdio transport (for local processes like npx servers)
    Stdio,
}

/// Configuration for a single MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPServerConfig {
    /// Server URL (for HTTP transport)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// HTTP headers (for authentication)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,

    /// Command to run (for stdio transport)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// Arguments for the command
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,

    /// Environment variables
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,

    /// Connection timeout in milliseconds
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

fn default_timeout() -> u64 {
    30000
}

impl MCPServerConfig {
    /// Determine transport type from config.
    pub fn transport_type(&self) -> TransportType {
        if self.url.is_some() {
            TransportType::Http
        } else {
            TransportType::Stdio
        }
    }

    /// Create HTTP server config.
    pub fn http(url: impl Into<String>) -> Self {
        Self {
            url: Some(url.into()),
            headers: HashMap::new(),
            command: None,
            args: Vec::new(),
            env: HashMap::new(),
            timeout_ms: default_timeout(),
        }
    }

    /// Create stdio server config.
    pub fn stdio(command: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            url: None,
            headers: HashMap::new(),
            command: Some(command.into()),
            args,
            env: HashMap::new(),
            timeout_ms: default_timeout(),
        }
    }

    /// Add HTTP header.
    pub fn header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Add environment variable.
    pub fn env_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set timeout.
    pub fn timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }
}

/// Root configuration containing all MCP servers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPConfig {
    /// Map of server name to configuration
    #[serde(rename = "mcpServers")]
    pub servers: HashMap<String, MCPServerConfig>,
}

impl MCPConfig {
    /// Create empty config.
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
        }
    }

    /// Load config from JSON file.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, MCPError> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| MCPError::ConnectionFailed(format!("Failed to read config: {}", e)))?;

        Self::from_json(&content)
    }

    /// Parse config from JSON string.
    pub fn from_json(json: &str) -> Result<Self, MCPError> {
        serde_json::from_str(json)
            .map_err(|e| MCPError::SerializationError(format!("Invalid config: {}", e)))
    }

    /// Add a server configuration.
    pub fn add_server(mut self, name: impl Into<String>, config: MCPServerConfig) -> Self {
        self.servers.insert(name.into(), config);
        self
    }

    /// Get server configuration by name.
    pub fn get(&self, name: &str) -> Option<&MCPServerConfig> {
        self.servers.get(name)
    }

    /// List all server names.
    pub fn server_names(&self) -> Vec<&str> {
        self.servers.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for MCPConfig {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_context7_config() {
        let json = r#"{
            "mcpServers": {
                "context7": {
                    "url": "https://mcp.context7.com/mcp",
                    "headers": {
                        "CONTEXT7_API_KEY": "test-key"
                    }
                }
            }
        }"#;

        let config = MCPConfig::from_json(json).unwrap();
        let server = config.get("context7").unwrap();

        assert_eq!(server.url.as_deref(), Some("https://mcp.context7.com/mcp"));
        assert_eq!(
            server.headers.get("CONTEXT7_API_KEY"),
            Some(&"test-key".to_string())
        );
        assert!(matches!(server.transport_type(), TransportType::Http));
    }

    #[test]
    fn test_parse_stdio_config() {
        let json = r#"{
            "mcpServers": {
                "github": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "env": {
                        "GITHUB_TOKEN": "ghp_xxx"
                    }
                }
            }
        }"#;

        let config = MCPConfig::from_json(json).unwrap();
        let server = config.get("github").unwrap();

        assert_eq!(server.command.as_deref(), Some("npx"));
        assert_eq!(
            server.args,
            vec!["-y", "@modelcontextprotocol/server-github"]
        );
        assert!(matches!(server.transport_type(), TransportType::Stdio));
    }

    #[test]
    fn test_builder() {
        let config = MCPConfig::new()
            .add_server(
                "context7",
                MCPServerConfig::http("https://mcp.context7.com/mcp")
                    .header("CONTEXT7_API_KEY", "test-key")
                    .timeout(60000),
            )
            .add_server(
                "github",
                MCPServerConfig::stdio(
                    "npx",
                    vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
                )
                .env_var("GITHUB_TOKEN", "ghp_xxx"),
            );

        assert_eq!(config.server_names().len(), 2);
    }
}
