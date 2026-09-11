//! Gateway LLM - Multi-provider LLM routing and load balancing
//!
//! This crate provides:
//! - Provider abstraction for multiple LLM backends
//! - Load balancing and failover
//! - Rate limiting per provider
//! - Streaming support

pub mod provider;
pub mod providers;
pub mod rate_limit;
pub mod router;

pub use provider::{Provider, ProviderRegistry};
pub use providers::{AnthropicProvider, OpenAIProvider};
pub use router::Router;
