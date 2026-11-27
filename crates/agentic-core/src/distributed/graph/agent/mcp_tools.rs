//! MCP tools as agent tools.
//!
//! A [`ChatAgent`] can take its tools from the servers of an
//! [`MCPClientPool`]: every tool the servers advertise is registered with a
//! handler that calls it, so the model can use them like any local tool.
//!
//! ```rust,ignore
//! use dasein_agentic_mcp::client::{MCPClientPool, MCPConfig};
//!
//! let pool = Arc::new(MCPClientPool::new(MCPConfig::from_file("mcp_servers.json")?).await?);
//! let agent = ChatAgent::new("assistant", llm)
//!     .with_mcp_tools(pool)   // or .with_mcp_server(pool, "github")
//!     .await?;
//! let answer = agent.run(vec![ChatMessage::user("...")], &mut thread).await?;
//! ```
//!
//! Requires the `mcp` feature of `dasein-agentic-core`.
//!
//! Servers are third parties. Their tool descriptions go into the model's
//! context and their results are read by the model: only connect servers you
//! trust, and treat what they return as untrusted input, as you would a web
//! page. The agent's own limits (tool rounds, per-tool timeout) apply.

use std::collections::HashSet;
use std::sync::Arc;

use dasein_agentic_llm::LLMAdapter;
use dasein_agentic_mcp::client::{MCPClientPool, ToolDefinition, ToolResult};

use super::{AgentError, ChatAgent, Tool, ToolParameters};

/// Providers accept tool names of at most this many characters.
const MAX_TOOL_NAME: usize = 64;

/// Most text of one tool result passed on to the model.
const MAX_RESULT_CHARS: usize = 64 * 1024;

impl<L: LLMAdapter> ChatAgent<L> {
    /// Give the agent every tool of one MCP server.
    ///
    /// Tools are named `{server}__{tool}`, reduced to the characters model
    /// providers accept, so tools of different servers never collide.
    ///
    /// # Errors
    /// Fails if the pool has no connected server of that name.
    pub async fn with_mcp_server(
        mut self,
        pool: Arc<MCPClientPool>,
        server: &str,
    ) -> Result<Self, AgentError> {
        let tools = pool
            .list_tools(server)
            .await
            .map_err(|e| AgentError::tool(server, e.to_string()))?;
        let mut taken = HashSet::new();
        for definition in tools {
            let name = unique_name(server, &definition.name, &mut taken);
            let (pool, server, remote) = (pool.clone(), server.to_owned(), definition.name.clone());
            let label = name.clone();
            self = self.with_tool_handler(agent_tool(name, &definition), move |arguments| {
                let (pool, server, remote, label) =
                    (pool.clone(), server.clone(), remote.clone(), label.clone());
                async move {
                    let result = pool
                        .call_tool(&server, &remote, arguments)
                        .await
                        .map_err(|e| AgentError::tool(&label, e.to_string()))?;
                    model_view(&label, result)
                }
            });
        }
        Ok(self)
    }

    /// Give the agent the tools of every server connected in the pool.
    ///
    /// # Errors
    /// Fails if a server cannot list its tools.
    pub async fn with_mcp_tools(mut self, pool: Arc<MCPClientPool>) -> Result<Self, AgentError> {
        let mut servers = pool.list_servers().await;
        // Stable order, so the tool list sent to the model does not shuffle.
        servers.sort();
        for server in servers {
            self = self.with_mcp_server(pool.clone(), &server).await?;
        }
        Ok(self)
    }
}

/// `{server}__{tool}` with only `[A-Za-z0-9_-]`, at most 64 characters, and
/// made unique among the names already taken.
fn unique_name(server: &str, tool: &str, taken: &mut HashSet<String>) -> String {
    let clean = |text: &str| -> String {
        text.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    let base: String = format!("{}__{}", clean(server), clean(tool))
        .chars()
        .take(MAX_TOOL_NAME)
        .collect();
    let mut name = base.clone();
    let mut suffix = 2;
    while !taken.insert(name.clone()) {
        let tail = format!("_{suffix}");
        name = base
            .chars()
            .take(MAX_TOOL_NAME - tail.len())
            .collect::<String>()
            + &tail;
        suffix += 1;
    }
    name
}

/// Describe an MCP tool to the model.
fn agent_tool(name: String, definition: &ToolDefinition) -> Tool {
    let description = definition.description.clone().unwrap_or_default();
    // MCP input schemas are free-form JSON Schema. Those that fit the agent's
    // parameter model are used as they are; for the others the schema is put
    // in the description, so the model still knows what to send.
    match serde_json::from_value::<ToolParameters>(definition.input_schema.clone()) {
        Ok(parameters) => Tool::new(name, description).with_parameters(parameters),
        Err(_) => Tool::new(
            name,
            format!(
                "{description}\nArguments (JSON Schema): {}",
                definition.input_schema
            ),
        ),
    }
}

/// What the model sees of a tool result. A tool that reports a failure
/// becomes an error, which the agent passes back to the model as such.
fn model_view(tool: &str, result: ToolResult) -> Result<serde_json::Value, AgentError> {
    let mut text = result.text();
    if text.chars().count() > MAX_RESULT_CHARS {
        text = text.chars().take(MAX_RESULT_CHARS).collect::<String>() + "\n[truncated]";
    }
    if result.is_error {
        return Err(AgentError::tool(tool, text));
    }
    let other_items = result.content.len()
        - result
            .content
            .iter()
            .filter(|item| {
                matches!(
                    item,
                    dasein_agentic_mcp::client::ContentItem::Text { .. }
                        | dasein_agentic_mcp::client::ContentItem::Resource { .. }
                )
            })
            .count();
    let mut view = serde_json::json!({ "text": text });
    if let Some(structured) = result.structured_content {
        view["structured"] = structured;
    }
    if other_items > 0 {
        // Images, audio and links are not passed to the model as text.
        view["omitted_non_text_items"] = other_items.into();
    }
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_safe_unique_and_bounded() {
        let mut taken = HashSet::new();
        assert_eq!(
            unique_name("docs", "query-docs", &mut taken),
            "docs__query-docs"
        );
        assert_eq!(
            unique_name("my.server", "a/b c", &mut taken),
            "my_server__a_b_c"
        );
        // Two remote names reducing to the same one stay distinct.
        assert_eq!(
            unique_name("my_server", "a b c", &mut taken),
            "my_server__a_b_c_2"
        );
        let long = unique_name("server", &"x".repeat(200), &mut taken);
        assert_eq!(long.len(), MAX_TOOL_NAME);
        let again = unique_name("server", &"x".repeat(200), &mut taken);
        assert_eq!(again.len(), MAX_TOOL_NAME);
        assert_ne!(long, again);
    }

    #[test]
    fn schemas_outside_the_parameter_model_go_to_the_description() {
        let simple = ToolDefinition {
            name: "t".into(),
            description: Some("Does a thing".into()),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "q": { "type": "string", "description": "Query" } },
                "required": ["q"],
            }),
        };
        let tool = agent_tool("s__t".into(), &simple);
        assert_eq!(tool.description, "Does a thing");
        assert_eq!(tool.parameters.required, ["q"]);

        // A property without a single `type` does not fit ToolParam.
        let complex = ToolDefinition {
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "v": { "anyOf": [{ "type": "string" }, { "type": "null" }] } },
            }),
            ..simple
        };
        let tool = agent_tool("s__t".into(), &complex);
        assert!(tool.description.contains("anyOf"), "{}", tool.description);
        assert!(tool.parameters.properties.is_empty());
    }
}
