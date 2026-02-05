//! Rate limiting for LLM providers

use governor::{Quota, RateLimiter};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Rate limiter configuration
#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    /// Requests per minute
    pub requests_per_minute: Option<u32>,
    /// Tokens per minute (input + output)
    pub tokens_per_minute: Option<u32>,
    /// Tokens per day
    pub tokens_per_day: Option<u32>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_minute: Some(60),
            tokens_per_minute: None,
            tokens_per_day: None,
        }
    }
}

type KeyedRateLimiter = RateLimiter<
    String,
    governor::state::keyed::DashMapStateStore<String>,
    governor::clock::DefaultClock,
>;

/// Rate limiter manager
pub struct RateLimitManager {
    /// Request rate limiters by provider
    request_limiters: RwLock<HashMap<String, Arc<KeyedRateLimiter>>>,
    /// Token usage tracking
    token_usage: RwLock<HashMap<String, TokenUsage>>,
    /// Configuration per provider
    configs: RwLock<HashMap<String, RateLimitConfig>>,
}

#[derive(Debug)]
struct TokenUsage {
    minute_tokens: u64,
    day_tokens: u64,
    minute_reset: std::time::Instant,
    day_reset: std::time::Instant,
}

impl Default for TokenUsage {
    fn default() -> Self {
        let now = std::time::Instant::now();
        Self {
            minute_tokens: 0,
            day_tokens: 0,
            minute_reset: now,
            day_reset: now,
        }
    }
}

impl RateLimitManager {
    pub fn new() -> Self {
        Self {
            request_limiters: RwLock::new(HashMap::new()),
            token_usage: RwLock::new(HashMap::new()),
            configs: RwLock::new(HashMap::new()),
        }
    }

    /// Configure rate limits for a provider
    pub async fn configure(&self, provider: &str, config: RateLimitConfig) {
        let mut configs = self.configs.write().await;
        configs.insert(provider.to_string(), config.clone());

        if let Some(rpm) = config.requests_per_minute {
            if let Some(quota) = NonZeroU32::new(rpm) {
                let limiter = RateLimiter::keyed(Quota::per_minute(quota));
                let mut limiters = self.request_limiters.write().await;
                limiters.insert(provider.to_string(), Arc::new(limiter));
            }
        }
    }

    /// Check if a request is allowed
    pub async fn check_request(&self, provider: &str, api_key: &str) -> Result<(), RateLimitError> {
        let limiters = self.request_limiters.read().await;
        if let Some(limiter) = limiters.get(provider) {
            limiter
                .check_key(&api_key.to_string())
                .map_err(|_| RateLimitError::RequestsExceeded)?;
        }
        Ok(())
    }

    /// Record token usage
    pub async fn record_tokens(
        &self,
        provider: &str,
        _api_key: &str,
        tokens: u64,
    ) -> Result<(), RateLimitError> {
        let configs = self.configs.read().await;
        let config = configs.get(provider);

        let mut usage = self.token_usage.write().await;
        let entry = usage.entry(provider.to_string()).or_default();

        let now = std::time::Instant::now();

        // Reset minute counter if needed
        if now.duration_since(entry.minute_reset).as_secs() >= 60 {
            entry.minute_tokens = 0;
            entry.minute_reset = now;
        }

        // Reset day counter if needed
        if now.duration_since(entry.day_reset).as_secs() >= 86400 {
            entry.day_tokens = 0;
            entry.day_reset = now;
        }

        entry.minute_tokens += tokens;
        entry.day_tokens += tokens;

        // Check limits
        if let Some(cfg) = config {
            if let Some(tpm) = cfg.tokens_per_minute {
                if entry.minute_tokens > tpm as u64 {
                    return Err(RateLimitError::TokensPerMinuteExceeded);
                }
            }
            if let Some(tpd) = cfg.tokens_per_day {
                if entry.day_tokens > tpd as u64 {
                    return Err(RateLimitError::TokensPerDayExceeded);
                }
            }
        }

        Ok(())
    }
}

impl Default for RateLimitManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RateLimitError {
    #[error("Request rate limit exceeded")]
    RequestsExceeded,
    #[error("Tokens per minute limit exceeded")]
    TokensPerMinuteExceeded,
    #[error("Tokens per day limit exceeded")]
    TokensPerDayExceeded,
}
