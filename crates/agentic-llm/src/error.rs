//! LLM error types.

use thiserror::Error;

/// Errors that can occur when interacting with LLM providers.
#[derive(Error, Debug)]
pub enum LLMError {
    /// API error from the provider
    #[error("API error: {0}")]
    ApiError(String),

    /// Network/connection error
    #[error("Connection error: {0}")]
    ConnectionError(String),

    /// Authentication error
    #[error("Authentication failed: {0}")]
    AuthenticationError(String),

    /// Rate limit exceeded
    #[error("Rate limit exceeded: {0}")]
    RateLimitError(String),

    /// Empty response from provider
    #[error("Empty response from LLM")]
    EmptyResponse,

    /// Invalid response format
    #[error("Invalid response format: {0}")]
    InvalidResponse(String),

    /// Timeout
    #[error("Request timed out")]
    Timeout,

    /// Model not found
    #[error("Model not found: {0}")]
    ModelNotFound(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),
}
