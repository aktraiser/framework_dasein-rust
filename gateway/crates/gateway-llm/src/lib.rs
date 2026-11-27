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

/// HTTP client shared by all providers, so connections are reused across
/// requests. It gives up when a provider cannot be reached within 10 s or
/// stays silent for 2 minutes; a stream that keeps sending is never cut.
pub fn http_client() -> reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .read_timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default()
        })
        .clone()
}
