//! Tool definitions for function calling.
//!
//! Tools allow agents to invoke external functions/APIs. This module
//! defines the `Tool` struct that describes available tools to the LLM.
//!
//! # Note
//!
//! Full tool execution is planned for a later phase. This module provides
//! the type definitions needed for the Agent trait.

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// TOOL DEFINITION
// ============================================================================

/// A tool that an agent can use.
///
/// Tools are described to the LLM, which can then request to invoke them.
/// The agent runtime handles the actual execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tool {
    /// Unique name of the tool (used in LLM function calling).
    pub name: String,

    /// Human-readable description (shown to LLM).
    pub description: String,

    /// JSON Schema for the tool's parameters.
    pub parameters: ToolParameters,

    /// Whether this tool is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Tool {
    /// Create a new tool.
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters: ToolParameters::default(),
            enabled: true,
        }
    }

    /// Add a required parameter.
    pub fn with_param(mut self, name: impl Into<String>, param: ToolParam) -> Self {
        self.parameters.add_required(name, param);
        self
    }

    /// Add an optional parameter.
    pub fn with_optional_param(mut self, name: impl Into<String>, param: ToolParam) -> Self {
        self.parameters.add_optional(name, param);
        self
    }

    /// Set the parameters schema directly.
    pub fn with_parameters(mut self, params: ToolParameters) -> Self {
        self.parameters = params;
        self
    }

    /// Disable the tool.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Check if tool is enabled.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

// ============================================================================
// TOOL PARAMETERS
// ============================================================================

/// Parameters schema for a tool (JSON Schema subset).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolParameters {
    /// Type is always "object" for function parameters.
    #[serde(rename = "type", default = "default_object")]
    pub param_type: String,

    /// Property definitions.
    #[serde(default)]
    pub properties: std::collections::HashMap<String, ToolParam>,

    /// Required property names.
    #[serde(default)]
    pub required: Vec<String>,
}

fn default_object() -> String {
    "object".to_string()
}

impl ToolParameters {
    /// Create empty parameters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a required parameter.
    pub fn add_required(&mut self, name: impl Into<String>, param: ToolParam) {
        let name = name.into();
        self.required.push(name.clone());
        self.properties.insert(name, param);
    }

    /// Add an optional parameter.
    pub fn add_optional(&mut self, name: impl Into<String>, param: ToolParam) {
        self.properties.insert(name.into(), param);
    }
}

// ============================================================================
// TOOL PARAM
// ============================================================================

/// A single parameter definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolParam {
    /// JSON Schema type (string, number, boolean, array, object).
    #[serde(rename = "type")]
    pub param_type: String,

    /// Description of the parameter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Enum values (if type is string with fixed options).
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "enum")]
    pub enum_values: Option<Vec<String>>,

    /// Default value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,

    /// Items schema (for array type).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Box<ToolParam>>,
}

impl ToolParam {
    /// Create a string parameter.
    pub fn string(description: impl Into<String>) -> Self {
        Self {
            param_type: "string".into(),
            description: Some(description.into()),
            enum_values: None,
            default: None,
            items: None,
        }
    }

    /// Create a number parameter.
    pub fn number(description: impl Into<String>) -> Self {
        Self {
            param_type: "number".into(),
            description: Some(description.into()),
            enum_values: None,
            default: None,
            items: None,
        }
    }

    /// Create an integer parameter.
    pub fn integer(description: impl Into<String>) -> Self {
        Self {
            param_type: "integer".into(),
            description: Some(description.into()),
            enum_values: None,
            default: None,
            items: None,
        }
    }

    /// Create a boolean parameter.
    pub fn boolean(description: impl Into<String>) -> Self {
        Self {
            param_type: "boolean".into(),
            description: Some(description.into()),
            enum_values: None,
            default: None,
            items: None,
        }
    }

    /// Create a string enum parameter.
    pub fn string_enum(description: impl Into<String>, values: Vec<&str>) -> Self {
        Self {
            param_type: "string".into(),
            description: Some(description.into()),
            enum_values: Some(values.into_iter().map(String::from).collect()),
            default: None,
            items: None,
        }
    }

    /// Create an array parameter.
    pub fn array(description: impl Into<String>, items: ToolParam) -> Self {
        Self {
            param_type: "array".into(),
            description: Some(description.into()),
            enum_values: None,
            default: None,
            items: Some(Box::new(items)),
        }
    }

    /// Add a default value.
    pub fn with_default(mut self, value: Value) -> Self {
        self.default = Some(value);
        self
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_new() {
        let tool = Tool::new("search", "Search the web");
        assert_eq!(tool.name, "search");
        assert!(tool.is_enabled());
    }

    #[test]
    fn test_tool_with_params() {
        let tool = Tool::new("calculator", "Perform calculations")
            .with_param(
                "expression",
                ToolParam::string("Math expression to evaluate"),
            )
            .with_optional_param("precision", ToolParam::integer("Decimal precision"));

        assert_eq!(tool.parameters.properties.len(), 2);
        assert_eq!(tool.parameters.required.len(), 1);
        assert!(tool.parameters.required.contains(&"expression".to_string()));
    }

    #[test]
    fn test_tool_param_types() {
        let string_param = ToolParam::string("A string");
        assert_eq!(string_param.param_type, "string");

        let number_param = ToolParam::number("A number");
        assert_eq!(number_param.param_type, "number");

        let bool_param = ToolParam::boolean("A boolean");
        assert_eq!(bool_param.param_type, "boolean");
    }

    #[test]
    fn test_tool_param_enum() {
        let param = ToolParam::string_enum("Select one", vec!["a", "b", "c"]);
        assert_eq!(
            param.enum_values,
            Some(vec!["a".into(), "b".into(), "c".into()])
        );
    }

    #[test]
    fn test_tool_param_array() {
        let param = ToolParam::array("List of items", ToolParam::string("Item"));
        assert_eq!(param.param_type, "array");
        assert!(param.items.is_some());
    }

    #[test]
    fn test_tool_serialize() {
        let tool =
            Tool::new("test", "Test tool").with_param("query", ToolParam::string("Search query"));

        let json = serde_json::to_string_pretty(&tool).unwrap();
        assert!(json.contains("\"name\": \"test\""));
        assert!(json.contains("\"query\""));
    }
}
