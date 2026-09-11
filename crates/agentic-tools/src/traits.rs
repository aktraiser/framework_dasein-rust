//! Core traits for tool execution.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ToolError;
use dasein_agentic_llm::Tool;

/// Result of executing a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Text output for the LLM.
    pub text: String,

    /// Structured data (for rich display like embeds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,

    /// Whether the tool execution was successful.
    #[serde(default)]
    pub success: bool,
}

impl ToolResult {
    /// Create a successful result with text.
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            data: None,
            success: true,
        }
    }

    /// Create a successful result with text and structured data.
    pub fn ok_with_data(text: impl Into<String>, data: Value) -> Self {
        Self {
            text: text.into(),
            data: Some(data),
            success: true,
        }
    }

    /// Create an error result.
    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            data: None,
            success: false,
        }
    }
}

/// Trait for tool executors.
///
/// Implement this trait to create a new tool that can be called by LLMs.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Get the tool name (must be unique).
    fn name(&self) -> &str;

    /// Get the tool definition for the LLM.
    fn definition(&self) -> Tool;

    /// Execute the tool with the given arguments.
    async fn execute(&self, arguments: &Value) -> Result<ToolResult, ToolError>;
}
