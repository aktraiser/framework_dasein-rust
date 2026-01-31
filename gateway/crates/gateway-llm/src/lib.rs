//! Gateway LLM - Multi-provider LLM routing and load balancing
//!
//! This crate provides:
//! - Provider abstraction for multiple LLM backends
//! - Load balancing and failover
//! - Rate limiting per provider
//! - Streaming support

pub mod provider;
pub mod providers;
pub mod router;
pub mod rate_limit;

pub use provider::{Provider, ProviderRegistry};
pub use providers::{OpenAIProvider, AnthropicProvider};
pub use router::Router;
