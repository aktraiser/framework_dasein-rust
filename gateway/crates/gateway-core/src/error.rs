//! Gateway error types

use thiserror::Error;

/// Main gateway error type
#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Authentication error: {0}")]
    Auth(String),

    #[error("Rate limit exceeded: {0}")]
    RateLimit(String),

    #[error("Sandbox error: {0}")]
    Sandbox(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Result type alias for gateway operations
pub type GatewayResult<T> = Result<T, GatewayError>;
