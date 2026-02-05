//! Gateway Core - Core types and traits for Agentic Gateway
//!
//! This crate provides the foundational types used across the gateway:
//! - LLM request/response types (OpenAI-compatible)
//! - Sandbox execution types
//! - Error types
//! - Configuration types

pub mod error;
pub mod llm;
pub mod sandbox;
pub mod config;

pub use error::{GatewayError, GatewayResult};
