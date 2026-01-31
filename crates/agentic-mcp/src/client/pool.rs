//! MCP Client Pool - manages connections to multiple MCP servers.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::config::{MCPConfig, MCPServerConfig, TransportType};
use crate::MCPError;

/// Tool definition from an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: Option<String>,
    /// Input schema (JSON Schema)
    pub input_schema: Value,
}

/// Result from calling an MCP tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Content returned by the tool
    pub content: Vec<ContentItem>,
    /// Whether this is an error result
    #[serde(default)]
    pub is_error: bool,
}

/// Content item in tool result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum ContentItem {
    /// Text content with the text value.
    Text {
        /// The text content.
        text: String,
    },
    /// Image content (base64 encoded).
    Image {
        /// Base64 encoded image data.
        data: String,
        /// MIME type (e.g., "image/png").
        mime_type: String,
    },
    /// Resource content with URI reference.
    Resource {
        /// URI of the resource.
        uri: String,
        /// Optional text content.
        text: Option<String>,
    },
}

/// Connected MCP server.
struct ConnectedServer {
    config: MCPServerConfig,
    tools: Vec<ToolDefinition>,
}

/// Pool of MCP client connections.
///
/// Thread-safe, can be shared across all agents.
pub struct MCPClientPool {
    servers: Arc<RwLock<HashMap<String, ConnectedServer>>>,
    config: MCPConfig,
}

impl MCPClientPool {
    /// Create a new pool from configuration.
    ///
    /// # Errors
    /// Returns an error if connection to any server fails.
    pub async fn new(config: MCPConfig) -> Result<Self, MCPError> {
        let pool = Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
            config,
        };

        // Connect to all configured servers
        pool.connect_all().await?;

        Ok(pool)
    }

    /// Connect to all configured servers.
    async fn connect_all(&self) -> Result<(), MCPError> {
        for (name, server_config) in &self.config.servers {
            match self.connect_server(name, server_config).await {
                Ok(()) => info!("Connected to MCP server: {name}"),
                Err(e) => warn!("Failed to connect to {name}: {e}"),
            }
        }
        Ok(())
    }

    /// Connect to a single server and discover its tools.
    async fn connect_server(&self, name: &str, config: &MCPServerConfig) -> Result<(), MCPError> {
        debug!("Connecting to MCP server: {name}");

        // Discover tools
        let tools = self.discover_tools(name, config).await?;
        info!("Discovered {} tools from {name}", tools.len());

        // Store connection
        self.servers.write().await.insert(
            name.to_string(),
            ConnectedServer {
                config: config.clone(),
                tools,
            },
        );

        Ok(())
    }

    /// Discover tools from a server.
    async fn discover_tools(
        &self,
        name: &str,
        config: &MCPServerConfig,
    ) -> Result<Vec<ToolDefinition>, MCPError> {
        match config.transport_type() {
            TransportType::Http => self.discover_tools_http(name, config).await,
            TransportType::Stdio => Self::discover_tools_stdio(name, config),
        }
    }

    /// Discover tools via HTTP transport.
    async fn discover_tools_http(
        &self,
        _name: &str,
        config: &MCPServerConfig,
    ) -> Result<Vec<ToolDefinition>, MCPError> {
        let url = config
            .url
            .as_ref()
            .ok_or_else(|| MCPError::ConnectionFailed("No URL configured".into()))?;

        // Build HTTP client with headers
        let mut headers = reqwest::header::HeaderMap::new();

        // MCP servers require Accept header for both JSON and SSE
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json, text/event-stream"),
        );

        for (key, value) in &config.headers {
            // Convert CONTEXT7_API_KEY to Authorization: Bearer format
            if key == "CONTEXT7_API_KEY" || key == "API_KEY" {
                let bearer = format!("Bearer {value}");
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    reqwest::header::HeaderValue::from_str(&bearer).map_err(|e| {
                        MCPError::ConnectionFailed(format!("Invalid auth value: {e}"))
                    })?,
                );
            } else {
                headers.insert(
                    reqwest::header::HeaderName::from_bytes(key.as_bytes()).map_err(|e| {
                        MCPError::ConnectionFailed(format!("Invalid header name: {e}"))
                    })?,
                    reqwest::header::HeaderValue::from_str(value).map_err(|e| {
                        MCPError::ConnectionFailed(format!("Invalid header value: {e}"))
                    })?,
                );
            }
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| MCPError::ConnectionFailed(format!("Failed to create client: {e}")))?;

        // Call tools/list
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        });

        let response = client
            .post(url)
            .json(&request)
            .send()
            .await
            .map_err(|e| MCPError::ConnectionFailed(format!("HTTP request failed: {e}")))?;

        let response_json: Value = response
            .json()
            .await
            .map_err(|e| MCPError::SerializationError(format!("Invalid response: {e}")))?;

        // Parse tools from response
        let tools = response_json["result"]["tools"]
            .as_array()
            .ok_or_else(|| MCPError::DiscoveryFailed("No tools in response".into()))?
            .iter()
            .map(|t| serde_json::from_value(t.clone()))
            .collect::<Result<Vec<ToolDefinition>, _>>()
            .map_err(|e| MCPError::SerializationError(format!("Failed to parse tools: {e}")))?;

        Ok(tools)
    }

    /// Discover tools via stdio transport.
    fn discover_tools_stdio(
        _name: &str,
        _config: &MCPServerConfig,
    ) -> Result<Vec<ToolDefinition>, MCPError> {
        // TODO: Implement stdio transport using tokio::process
        // This requires spawning a child process and communicating via stdin/stdout
        Err(MCPError::ConnectionFailed(
            "Stdio transport not yet implemented".into(),
        ))
    }

    /// List all connected servers.
    pub async fn list_servers(&self) -> Vec<String> {
        let servers = self.servers.read().await;
        servers.keys().cloned().collect()
    }

    /// List tools from a specific server.
    ///
    /// # Errors
    /// Returns an error if the server is not found.
    pub async fn list_tools(&self, server_name: &str) -> Result<Vec<ToolDefinition>, MCPError> {
        let tools = self
            .servers
            .read()
            .await
            .get(server_name)
            .ok_or_else(|| MCPError::ServerNotFound(server_name.to_string()))?
            .tools
            .clone();

        Ok(tools)
    }

    /// List all tools from all servers.
    pub async fn list_all_tools(&self) -> HashMap<String, Vec<ToolDefinition>> {
        let servers = self.servers.read().await;
        servers
            .iter()
            .map(|(name, server)| (name.clone(), server.tools.clone()))
            .collect()
    }

    /// Call a tool on a server.
    ///
    /// # Errors
    /// Returns an error if the server is not found or the tool call fails.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<ToolResult, MCPError> {
        let config = self
            .servers
            .read()
            .await
            .get(server_name)
            .ok_or_else(|| MCPError::ServerNotFound(server_name.to_string()))?
            .config
            .clone();

        match config.transport_type() {
            TransportType::Http => self.call_tool_http(&config, tool_name, arguments).await,
            TransportType::Stdio => Err(MCPError::CallFailed(
                "Stdio transport not yet implemented".into(),
            )),
        }
    }

    /// Call tool via HTTP.
    async fn call_tool_http(
        &self,
        config: &MCPServerConfig,
        tool_name: &str,
        arguments: Value,
    ) -> Result<ToolResult, MCPError> {
        let url = config
            .url
            .as_ref()
            .ok_or_else(|| MCPError::CallFailed("No URL configured".into()))?;

        let mut headers = reqwest::header::HeaderMap::new();

        // MCP servers require Accept header for both JSON and SSE
        headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("application/json, text/event-stream"),
        );

        for (key, value) in &config.headers {
            // Convert API_KEY headers to Authorization: Bearer format
            if key == "CONTEXT7_API_KEY" || key == "API_KEY" {
                let bearer = format!("Bearer {value}");
                if let Ok(val) = reqwest::header::HeaderValue::from_str(&bearer) {
                    headers.insert(reqwest::header::AUTHORIZATION, val);
                }
            } else if let (Ok(header_name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(key.as_bytes()),
                reqwest::header::HeaderValue::from_str(value),
            ) {
                headers.insert(header_name, val);
            }
        }

        let client = reqwest::Client::builder()
            .default_headers(headers)
            .timeout(std::time::Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| MCPError::CallFailed(format!("Failed to create client: {e}")))?;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": tool_name,
                "arguments": arguments
            }
        });

        let response = client
            .post(url)
            .json(&request)
            .send()
            .await
            .map_err(|e| MCPError::CallFailed(format!("HTTP request failed: {e}")))?;

        let response_json: Value = response
            .json()
            .await
            .map_err(|e| MCPError::SerializationError(format!("Invalid response: {e}")))?;

        // Check for error
        if let Some(error) = response_json.get("error") {
            return Err(MCPError::CallFailed(error.to_string()));
        }

        // Parse result
        let result = response_json
            .get("result")
            .ok_or_else(|| MCPError::CallFailed("No result in response".into()))?;

        serde_json::from_value(result.clone())
            .map_err(|e| MCPError::SerializationError(format!("Failed to parse result: {e}")))
    }

    /// Generate TypeScript files for all tools.
    ///
    /// Creates a `./servers/` directory structure with TypeScript modules
    /// that agents can import and use.
    ///
    /// # Errors
    /// Returns an error if file operations fail.
    pub async fn generate_typescript(&self, output_dir: &std::path::Path) -> Result<(), MCPError> {
        for (server_name, server) in self.servers.read().await.iter() {
            let server_dir = output_dir.join("servers").join(server_name);
            std::fs::create_dir_all(&server_dir)
                .map_err(|e| MCPError::ConnectionFailed(format!("Failed to create dir: {e}")))?;

            // Generate index.ts
            let mut index_exports = Vec::new();

            for tool in &server.tools {
                let ts_code = Self::generate_tool_ts(server_name, tool);
                let filename = format!("{}.ts", tool.name);
                let filepath = server_dir.join(&filename);

                std::fs::write(&filepath, ts_code).map_err(|e| {
                    MCPError::ConnectionFailed(format!("Failed to write file: {e}"))
                })?;

                index_exports.push(format!(
                    "export {{ {} }} from './{}';\n",
                    tool.name, tool.name
                ));
            }

            // Write index.ts
            let index_path = server_dir.join("index.ts");
            std::fs::write(&index_path, index_exports.join(""))
                .map_err(|e| MCPError::ConnectionFailed(format!("Failed to write index: {e}")))?;
        }

        // Generate client.ts helper
        let client_ts = Self::generate_client_ts();
        std::fs::write(output_dir.join("client.ts"), client_ts)
            .map_err(|e| MCPError::ConnectionFailed(format!("Failed to write client: {e}")))?;

        Ok(())
    }

    /// Generate TypeScript for a single tool.
    fn generate_tool_ts(server_name: &str, tool: &ToolDefinition) -> String {
        let description = tool.description.as_deref().unwrap_or("No description");
        let input_schema = serde_json::to_string_pretty(&tool.input_schema).unwrap_or_default();

        format!(
            r"import {{ callMCPTool }} from '../../client';

/**
 * {description}
 *
 * Input Schema:
 * ```json
 * {input_schema}
 * ```
 */
export async function {name}(input: any): Promise<any> {{
  return callMCPTool('{server}', '{name}', input);
}}
",
            description = description,
            input_schema = input_schema,
            name = tool.name,
            server = server_name,
        )
    }

    /// Generate the client.ts helper file.
    fn generate_client_ts() -> String {
        r"// MCP Client - Auto-generated
// This file handles communication with MCP servers

const MCP_ENDPOINT = process.env.MCP_ENDPOINT || 'http://localhost:3000/mcp';

export async function callMCPTool(server: string, tool: string, input: any): Promise<any> {
  const response = await fetch(`${MCP_ENDPOINT}/${server}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: Date.now(),
      method: 'tools/call',
      params: { name: tool, arguments: input }
    })
  });

  const json = await response.json();

  if (json.error) {
    throw new Error(json.error.message || JSON.stringify(json.error));
  }

  return json.result;
}
"
        .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_definition_parse() {
        let json = r#"{
            "name": "query-docs",
            "description": "Query documentation",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            }
        }"#;

        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(tool.name, "query-docs");
    }

    #[test]
    fn test_tool_result_parse() {
        let json = r#"{
            "content": [
                { "type": "text", "text": "Hello world" }
            ],
            "isError": false
        }"#;

        let result: ToolResult = serde_json::from_str(json).unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content.len(), 1);
    }
}
