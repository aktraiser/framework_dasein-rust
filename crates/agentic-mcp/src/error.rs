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

    /// The server answered with a JSON-RPC error.
    #[error("Server error {code}: {message}{}", .data.as_ref().map(|data| format!(" ({data})")).unwrap_or_default())]
    Rpc {
        /// JSON-RPC error code.
        code: i64,
        /// Message from the server.
        message: String,
        /// Additional details from the server, if any.
        data: Option<serde_json::Value>,
    },

    /// The server answered with an HTTP error status.
    #[error("HTTP {status}: {body}")]
    Http {
        /// HTTP status code.
        status: u16,
        /// Start of the response body.
        body: String,
    },

    /// Timeout
    #[error("Request timed out")]
    Timeout,
}
