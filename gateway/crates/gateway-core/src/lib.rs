//! Gateway Core - Core types and traits for Agentic Gateway
//!
//! This crate provides the foundational types used across the gateway:
//! - LLM request/response types (OpenAI-compatible)
//! - Sandbox execution types
//! - Error types
//! - Configuration types
//! - Storage (SQLite persistence)

pub mod config;
pub mod error;
pub mod llm;
pub mod sandbox;
pub mod storage;

pub use error::{GatewayError, GatewayResult};
pub use storage::{Storage, StorageError, StorageResult};
