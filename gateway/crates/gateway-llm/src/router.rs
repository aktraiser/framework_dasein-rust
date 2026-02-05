//! Request routing with load balancing and failover

use crate::provider::{Provider, ProviderError, ProviderRegistry, ProviderResult};
use gateway_core::llm::{ChatCompletionRequest, ChatCompletionResponse};
use std::sync::Arc;
use tracing::{info, warn};

/// Routing strategy
#[derive(Debug, Clone, Default)]
pub enum RoutingStrategy {
    /// Use primary provider, fallback on failure
    #[default]
    Failover,
    /// Round-robin across healthy providers
    RoundRobin,
    /// Route based on cost optimization
    CostOptimized,
    /// Route based on latency
    LatencyOptimized,
}

/// Router configuration
#[derive(Debug, Clone)]
pub struct RouterConfig {
    pub strategy: RoutingStrategy,
    pub max_retries: usize,
    pub retry_delay_ms: u64,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            strategy: RoutingStrategy::Failover,
            max_retries: 3,
            retry_delay_ms: 100,
        }
    }
}

/// LLM request router
pub struct Router {
    registry: Arc<ProviderRegistry>,
    config: RouterConfig,
}

impl Router {
    pub fn new(registry: Arc<ProviderRegistry>, config: RouterConfig) -> Self {
        Self { registry, config }
    }

    /// Route a chat completion request
    pub async fn chat_completion(
        &self,
        request: ChatCompletionRequest,
        fallbacks: &[String],
    ) -> ProviderResult<ChatCompletionResponse> {
        let model = &request.model;

        // Find primary provider for this model
        let primary = self
            .registry
            .find_for_model(model)
            .ok_or_else(|| ProviderError::ModelNotSupported(model.clone()))?;

        // Try primary provider
        match self.try_provider(&primary, request.clone()).await {
            Ok(response) => return Ok(response),
            Err(e) => {
                warn!(
                    provider = primary.name(),
                    error = %e,
                    "Primary provider failed, trying fallbacks"
                );
            }
        }

        // Try fallback providers
        for fallback_name in fallbacks {
            if let Some(provider) = self.registry.get(fallback_name) {
                match self.try_provider(&provider, request.clone()).await {
                    Ok(response) => {
                        info!(provider = fallback_name, "Fallback provider succeeded");
                        return Ok(response);
                    }
                    Err(e) => {
                        warn!(
                            provider = fallback_name,
                            error = %e,
                            "Fallback provider failed"
                        );
                    }
                }
            }
        }

        Err(ProviderError::Unavailable(
            "All providers failed".to_string(),
        ))
    }

    async fn try_provider(
        &self,
        provider: &Arc<dyn Provider>,
        request: ChatCompletionRequest,
    ) -> ProviderResult<ChatCompletionResponse> {
        let mut last_error = None;

        for attempt in 0..=self.config.max_retries {
            if attempt > 0 {
                tokio::time::sleep(tokio::time::Duration::from_millis(
                    self.config.retry_delay_ms * attempt as u64,
                ))
                .await;
            }

            match provider.chat_completion(request.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    // Don't retry on certain errors
                    if matches!(
                        e,
                        ProviderError::AuthFailed | ProviderError::ModelNotSupported(_)
                    ) {
                        return Err(e);
                    }
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| ProviderError::Unavailable("Unknown error".to_string())))
    }
}
