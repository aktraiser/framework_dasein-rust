//! Gateway Server - HTTP API for Agentic Gateway
//!
//! This crate provides:
//! - OpenAI-compatible LLM API endpoints
//! - Sandbox execution API
//! - Health and metrics endpoints
//! - Authentication middleware

pub mod api;
pub mod middleware;
pub mod state;

pub use api::create_router;
pub use state::AppState;
