//! Tool registry for managing and executing tools.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tracing::{debug, instrument};

use crate::error::ToolError;
use crate::traits::{ToolExecutor, ToolResult};
use dasein_agentic_llm::{Tool, ToolCall};

/// Registry for managing tools.
///
/// The registry holds all available tools and provides methods to:
/// - Register new tools
/// - Get tool definitions for LLMs
/// - Execute tool calls
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn ToolExecutor>>,
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool.
    pub fn register<T: ToolExecutor + 'static>(&mut self, tool: T) {
        let name = tool.name().to_string();
        debug!(tool = %name, "Registering tool");
        self.tools.insert(name, Arc::new(tool));
    }

    /// Check if a tool is registered.
    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Get the number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Get all tool definitions for LLM.
    pub fn get_tools(&self) -> Vec<Tool> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    /// Get tool names.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.keys().map(String::as_str).collect()
    }

    /// Execute a tool by name.
    #[instrument(skip(self, arguments), fields(tool = %name))]
    pub async fn execute(&self, name: &str, arguments: &Value) -> Result<ToolResult, ToolError> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| ToolError::NotFound(name.to_string()))?;

        debug!(?arguments, "Executing tool");
        tool.execute(arguments).await
    }

    /// Execute a tool call from an LLM response.
    pub async fn execute_call(&self, call: &ToolCall) -> Result<ToolResult, ToolError> {
        self.execute(&call.name, &call.arguments).await
    }

    /// Execute multiple tool calls and return results.
    pub async fn execute_calls(
        &self,
        calls: &[ToolCall],
    ) -> Vec<(String, Result<ToolResult, ToolError>)> {
        let mut results = Vec::with_capacity(calls.len());

        for call in calls {
            let result = self.execute_call(call).await;
            results.push((call.name.clone(), result));
        }

        results
    }

    /// Execute multiple tool calls and format results as text for LLM.
    pub async fn execute_calls_as_text(&self, calls: &[ToolCall]) -> String {
        let results = self.execute_calls(calls).await;
        let mut output = String::new();

        for (name, result) in results {
            match result {
                Ok(r) => output.push_str(&format!("[{}]: {}\n", name, r.text)),
                Err(e) => output.push_str(&format!("[{}]: Error: {}\n", name, e)),
            }
        }

        output
    }
}
