//! Error types for tools.

use thiserror::Error;

/// Errors that can occur during tool execution.
#[derive(Debug, Error)]
pub enum ToolError {
    /// Tool not found in registry.
    #[error("Tool not found: {0}")]
    NotFound(String),

    /// Missing required argument.
    #[error("Missing required argument: {0}")]
    MissingArgument(String),

    /// Invalid argument type or value.
    #[error("Invalid argument '{0}': {1}")]
    InvalidArgument(String, String),

    /// External service error (API, CLI, etc.).
    #[error("Service error: {0}")]
    ServiceError(String),

    /// Tool is not configured (missing API key, etc.).
    #[error("Tool not configured: {0}")]
    NotConfigured(String),

    /// Configuration error (missing env var, etc.).
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Command execution failed.
    #[error("Command failed: {0}")]
    CommandFailed(String),

    /// Serialization/deserialization error.
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

impl From<reqwest::Error> for ToolError {
    fn from(e: reqwest::Error) -> Self {
        Self::ServiceError(e.to_string())
    }
}

impl From<serde_json::Error> for ToolError {
    fn from(e: serde_json::Error) -> Self {
        Self::SerializationError(e.to_string())
    }
}

impl From<std::io::Error> for ToolError {
    fn from(e: std::io::Error) -> Self {
        Self::CommandFailed(e.to_string())
    }
}
