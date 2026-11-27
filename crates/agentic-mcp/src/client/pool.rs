//! MCP Client Pool - manages connections to multiple MCP servers.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

use super::config::{MCPConfig, MCPServerConfig};
use super::transport::Connection;
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
    #[serde(default)]
    pub content: Vec<ContentItem>,
    /// Whether this is an error result (`isError` on the wire)
    #[serde(default, rename = "isError", alias = "is_error")]
    pub is_error: bool,
    /// Structured result, when the tool declares an output schema.
    #[serde(
        default,
        rename = "structuredContent",
        skip_serializing_if = "Option::is_none"
    )]
    pub structured_content: Option<Value>,
}

impl ToolResult {
    /// All text the tool returned, one item per line. Embedded text resources
    /// are included; images and other binary content are not.
    #[must_use]
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(|item| match item {
                ContentItem::Text { text } => Some(text.as_str()),
                ContentItem::Resource { resource } => resource.text.as_deref(),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Content item in tool result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
        /// MIME type (e.g., "image/png"), `mimeType` on the wire.
        #[serde(rename = "mimeType", alias = "mime_type")]
        mime_type: String,
    },
    /// Audio content (base64 encoded).
    Audio {
        /// Base64 encoded audio data.
        data: String,
        /// MIME type (e.g., "audio/wav"), `mimeType` on the wire.
        #[serde(rename = "mimeType", alias = "mime_type")]
        mime_type: String,
    },
    /// A resource embedded in the result.
    Resource {
        /// The resource and its contents.
        resource: ResourceContents,
    },
    /// A link to a resource the client can fetch separately.
    ResourceLink {
        /// URI of the resource.
        uri: String,
        /// Short name of the resource.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// What the resource is.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// MIME type, `mimeType` on the wire.
        #[serde(default, rename = "mimeType", skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    /// A content type this client does not know. Kept so that one unfamiliar
    /// item does not discard the rest of the result.
    #[serde(other)]
    Unknown,
}

/// Contents of an embedded resource: text, or base64 `blob` for binary data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceContents {
    /// URI of the resource.
    pub uri: String,
    /// MIME type, `mimeType` on the wire.
    #[serde(default, rename = "mimeType", skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    /// Text contents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Base64 encoded binary contents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
}

/// Connected MCP server.
struct ConnectedServer {
    connection: Arc<Connection>,
    tools: Vec<ToolDefinition>,
}

/// Pool of MCP client connections.
///
/// Thread-safe, can be shared across all agents.
pub struct MCPClientPool {
    servers: Arc<RwLock<HashMap<String, ConnectedServer>>>,
    failures: Arc<RwLock<HashMap<String, String>>>,
    config: MCPConfig,
}

/// Upper bound on `tools/list` pages, against a server that never stops paging.
const MAX_TOOL_PAGES: usize = 100;

impl MCPClientPool {
    /// Create a new pool and connect to every configured server.
    ///
    /// A server that cannot be reached does not fail the pool: the others stay
    /// usable. Check [`connection_errors`](Self::connection_errors) to see
    /// which servers are missing and why.
    ///
    /// # Errors
    /// Currently never fails; the `Result` is kept for API stability.
    pub async fn new(config: MCPConfig) -> Result<Self, MCPError> {
        let pool = Self {
            servers: Arc::new(RwLock::new(HashMap::new())),
            failures: Arc::new(RwLock::new(HashMap::new())),
            config,
        };
        pool.connect_all().await;
        Ok(pool)
    }

    /// Connect to all configured servers, recording the ones that fail.
    async fn connect_all(&self) {
        for (name, server_config) in &self.config.servers {
            match self.connect_server(name, server_config).await {
                Ok(()) => info!("Connected to MCP server: {name}"),
                Err(e) => {
                    warn!("Failed to connect to {name}: {e}");
                    self.failures
                        .write()
                        .await
                        .insert(name.clone(), e.to_string());
                }
            }
        }
    }

    /// Servers from the configuration that could not be connected, with the
    /// reason for each.
    pub async fn connection_errors(&self) -> HashMap<String, String> {
        self.failures.read().await.clone()
    }

    /// Connect to a single server and discover its tools.
    async fn connect_server(&self, name: &str, config: &MCPServerConfig) -> Result<(), MCPError> {
        debug!("Connecting to MCP server: {name}");
        let connection = Connection::connect(config).await?;
        let tools = Self::discover_tools(&connection).await?;
        info!("Discovered {} tools from {name}", tools.len());

        self.servers.write().await.insert(
            name.to_string(),
            ConnectedServer {
                connection: Arc::new(connection),
                tools,
            },
        );
        Ok(())
    }

    /// List every tool of a server, following pagination.
    async fn discover_tools(connection: &Connection) -> Result<Vec<ToolDefinition>, MCPError> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..MAX_TOOL_PAGES {
            let params = cursor.as_ref().map_or_else(
                || serde_json::json!({}),
                |c| serde_json::json!({ "cursor": c }),
            );
            let result = connection
                .request("tools/list", params)
                .await
                .map_err(|e| match e {
                    MCPError::Rpc { .. } => MCPError::DiscoveryFailed(e.to_string()),
                    other => other,
                })?;
            let page = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| MCPError::DiscoveryFailed("No tools in response".into()))?;
            for tool in page {
                tools.push(serde_json::from_value(tool.clone()).map_err(|e| {
                    MCPError::SerializationError(format!("Failed to parse tools: {e}"))
                })?);
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                break;
            }
        }
        Ok(tools)
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
    /// A tool that ran and reported a failure returns `Ok` with
    /// [`ToolResult::is_error`] set; `Err` means the call itself did not
    /// complete (unknown server, transport failure, protocol error).
    ///
    /// # Errors
    /// Returns an error if the server is not found or the tool call fails.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        arguments: Value,
    ) -> Result<ToolResult, MCPError> {
        // Clone the handle so the registry lock is not held during the call.
        let connection = self
            .servers
            .read()
            .await
            .get(server_name)
            .ok_or_else(|| MCPError::ServerNotFound(server_name.to_string()))?
            .connection
            .clone();

        let params = serde_json::json!({ "name": tool_name, "arguments": arguments });
        let result = connection
            .request("tools/call", params)
            .await
            .map_err(|e| match e {
                MCPError::Rpc { .. } => MCPError::CallFailed(e.to_string()),
                other => other,
            })?;
        serde_json::from_value(result)
            .map_err(|e| MCPError::SerializationError(format!("Failed to parse result: {e}")))
    }

    /// Generate TypeScript files for all tools.
    ///
    /// Creates a `./servers/` directory structure with TypeScript modules
    /// that agents can import and use. Server and tool names come from remote
    /// servers, so they are reduced to safe file names and identifiers:
    /// `query-docs` becomes the function `query_docs` in `query_docs.ts`.
    ///
    /// # Errors
    /// Returns an error if file operations fail.
    pub async fn generate_typescript(&self, output_dir: &std::path::Path) -> Result<(), MCPError> {
        let io_error =
            |what: &str, e: std::io::Error| MCPError::TransportError(format!("{what}: {e}"));
        for (server_name, server) in self.servers.read().await.iter() {
            let server_dir = output_dir
                .join("servers")
                .join(Self::safe_file_stem(server_name));
            std::fs::create_dir_all(&server_dir)
                .map_err(|e| io_error("Failed to create dir", e))?;

            let mut index_exports = Vec::new();
            let mut used = std::collections::HashSet::new();
            for tool in &server.tools {
                // Two tool names may reduce to one identifier: keep both.
                let base = Self::ts_identifier(&tool.name);
                let mut ident = base.clone();
                let mut suffix = 2;
                while !used.insert(ident.clone()) {
                    ident = format!("{base}_{suffix}");
                    suffix += 1;
                }
                let ts_code = Self::generate_tool_ts(server_name, tool, &ident);
                std::fs::write(server_dir.join(format!("{ident}.ts")), ts_code)
                    .map_err(|e| io_error("Failed to write file", e))?;
                index_exports.push(format!("export {{ {ident} }} from './{ident}';\n"));
            }

            std::fs::write(server_dir.join("index.ts"), index_exports.join(""))
                .map_err(|e| io_error("Failed to write index", e))?;
        }

        std::fs::write(output_dir.join("client.ts"), Self::generate_client_ts())
            .map_err(|e| io_error("Failed to write client", e))?;

        Ok(())
    }

    /// Reduce a name to a single safe path component.
    fn safe_file_stem(name: &str) -> String {
        let stem: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if stem.is_empty() {
            "_".into()
        } else {
            stem
        }
    }

    /// Reduce a tool name to a valid TypeScript identifier.
    fn ts_identifier(name: &str) -> String {
        const RESERVED: &[&str] = &[
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "debugger",
            "default",
            "delete",
            "do",
            "else",
            "enum",
            "export",
            "extends",
            "false",
            "finally",
            "for",
            "function",
            "if",
            "import",
            "in",
            "instanceof",
            "new",
            "null",
            "return",
            "super",
            "switch",
            "this",
            "throw",
            "true",
            "try",
            "typeof",
            "var",
            "void",
            "while",
            "with",
            "let",
            "static",
            "yield",
            "await",
            "async",
        ];
        let mut ident: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        if ident.is_empty()
            || ident.starts_with(|c: char| c.is_ascii_digit())
            || RESERVED.contains(&ident.as_str())
        {
            ident.insert(0, '_');
        }
        ident
    }

    /// Generate TypeScript for a single tool, exported as `ident`.
    fn generate_tool_ts(server_name: &str, tool: &ToolDefinition, ident: &str) -> String {
        // Text from the server must not be able to close the comment.
        let in_comment = |text: &str| text.replace("*/", "*\\/");
        let description = in_comment(tool.description.as_deref().unwrap_or("No description"));
        let input_schema =
            in_comment(&serde_json::to_string_pretty(&tool.input_schema).unwrap_or_default());
        // JSON string literals are valid TypeScript string literals.
        let server = serde_json::to_string(server_name).unwrap_or_default();
        let name = serde_json::to_string(&tool.name).unwrap_or_default();

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
export async function {ident}(input: any): Promise<any> {{
  return callMCPTool({server}, {name}, input);
}}
"
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

    #[test]
    fn test_tool_error_and_image_use_wire_names() {
        let json = r#"{
            "content": [
                { "type": "image", "data": "aGk=", "mimeType": "image/png" }
            ],
            "isError": true
        }"#;

        let result: ToolResult = serde_json::from_str(json).unwrap();
        assert!(
            result.is_error,
            "a failed tool call must not look successful"
        );
        assert!(matches!(
            &result.content[0],
            ContentItem::Image { mime_type, .. } if mime_type == "image/png"
        ));
        let wire = serde_json::to_value(&result).unwrap();
        assert_eq!(wire["isError"], true);
        assert_eq!(wire["content"][0]["mimeType"], "image/png");
    }
}
