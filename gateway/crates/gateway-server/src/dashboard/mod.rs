//! Admin Dashboard module
//!
//! Provides a web-based admin interface for the gateway:
//! - Dashboard with usage statistics
//! - Provider management
//! - API key management
//! - Sandbox configuration

pub mod auth;
pub mod handlers;
pub mod routes;
pub mod templates;

pub use routes::create_dashboard_router;
