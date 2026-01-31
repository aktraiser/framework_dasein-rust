//! MCP error types.

use thiserror::Error;

/// Errors that can occur with MCP operations.
#[derive(Error, Debug)]
pub enum MCPError {
    /// Transport error
    #[error("Transport error: {0}")]
    TransportError(String),

    /// Connection failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Server not found
    #[error("Server not found: {0}")]
    ServerNotFound(String),

    /// Tool discovery failed
    #[error("Discovery failed: {0}")]
    DiscoveryFailed(String),

    /// Tool call failed
    #[error("Call failed: {0}")]
    CallFailed(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Timeout
    #[error("Request timed out")]
    Timeout,
}
