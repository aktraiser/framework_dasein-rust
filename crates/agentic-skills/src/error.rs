//! Skill system error types.

use thiserror::Error;

/// Errors that can occur in the skill system.
#[derive(Error, Debug)]
pub enum SkillError {
    /// Skill not found in registry
    #[error("Skill not found: {0}")]
    NotFound(String),

    /// Skill already exists
    #[error("Skill already exists: {0}")]
    AlreadyExists(String),

    /// Invalid skill definition
    #[error("Invalid skill: {0}")]
    InvalidSkill(String),

    /// Storage error (Redis/Qdrant)
    #[error("Storage error: {0}")]
    StorageError(String),

    /// Embedding generation failed
    #[error("Embedding error: {0}")]
    EmbeddingError(String),

    /// No matching skills found for query
    #[error("No matching skills found for query: {0}")]
    NoMatch(String),

    /// Serialization/deserialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),
}

impl From<serde_json::Error> for SkillError {
    fn from(err: serde_json::Error) -> Self {
        Self::SerializationError(err.to_string())
    }
}
