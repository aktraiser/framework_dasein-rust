//! Bus error types.

use thiserror::Error;

/// Errors that can occur with the message bus.
#[derive(Error, Debug)]
pub enum BusError {
    /// Connection failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// Disconnection failed
    #[error("Disconnect failed: {0}")]
    DisconnectFailed(String),

    /// Publish failed
    #[error("Publish failed: {0}")]
    PublishFailed(String),

    /// Subscribe failed
    #[error("Subscribe failed: {0}")]
    SubscribeFailed(String),

    /// Request failed
    #[error("Request failed: {0}")]
    RequestFailed(String),

    /// Timeout
    #[error("Request timed out")]
    Timeout,

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Deserialization error
    #[error("Deserialization error: {0}")]
    DeserializationError(String),
}
