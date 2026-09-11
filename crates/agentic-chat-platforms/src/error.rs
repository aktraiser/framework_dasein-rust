//! Chat platform error types.

use thiserror::Error;

/// Errors that can occur with chat platforms.
#[derive(Error, Debug)]
pub enum ChatError {
    /// Platform not initialized
    #[error("Platform not initialized")]
    NotInitialized,

    /// Platform not found
    #[error("Platform not found: {0}")]
    PlatformNotFound(String),

    /// Connection failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Send message failed
    #[error("Send failed: {0}")]
    SendFailed(String),

    /// Receive failed
    #[error("Receive failed: {0}")]
    ReceiveFailed(String),

    /// Invalid channel ID
    #[error("Invalid channel ID: {0}")]
    InvalidChannelId(String),

    /// Invalid user ID
    #[error("Invalid user ID: {0}")]
    InvalidUserId(String),

    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Rate limited
    #[error("Rate limited: retry after {0}ms")]
    RateLimited(u64),

    /// Platform-specific error
    #[error("Platform error: {0}")]
    PlatformError(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    /// Channel closed
    #[error("Channel closed")]
    ChannelClosed,
}

/// Result type for chat operations.
pub type ChatResult<T> = Result<T, ChatError>;
