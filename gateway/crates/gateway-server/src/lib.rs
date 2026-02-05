//! Gateway Server - HTTP API for Agentic Gateway
//!
//! This crate provides:
//! - OpenAI-compatible LLM API endpoints
//! - Sandbox execution API
//! - Health and metrics endpoints
//! - Authentication middleware
//! - Admin dashboard

pub mod api;
pub mod dashboard;
pub mod middleware;
pub mod state;

pub use api::create_router;
pub use dashboard::create_dashboard_router;
pub use state::AppState;
