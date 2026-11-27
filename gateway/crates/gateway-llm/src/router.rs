//! Request routing with load balancing and failover

use crate::provider::{Provider, ProviderError, ProviderRegistry, ProviderResult};
use gateway_core::llm::{
    ChatCompletionRequest, ChatCompletionResponse, EmbeddingRequest, EmbeddingResponse,
};
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
        self.chat_completion_with_provider(request, fallbacks)
            .await
            .map(|(response, _)| response)
    }

    /// Route a chat completion request and report which provider answered,
    /// which differs from the model's own provider after a failover.
    pub async fn chat_completion_with_provider(
        &self,
        request: ChatCompletionRequest,
        fallbacks: &[String],
    ) -> ProviderResult<(ChatCompletionResponse, String)> {
        if !matches!(self.config.strategy, RoutingStrategy::Failover) {
            return Err(ProviderError::Unavailable(
                "Only failover routing is implemented".into(),
            ));
        }
        let model = &request.model;

        // Find primary provider for this model
        let primary = self
            .registry
            .find_for_model(model)
            .ok_or_else(|| ProviderError::ModelNotSupported(model.clone()))?;

        // Try primary provider
        match self.try_provider(&primary, request.clone()).await {
            Ok(response) => return Ok((response, primary.name().to_string())),
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
                        return Ok((response, fallback_name.clone()));
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

    /// Name of the provider a model is routed to, if any.
    pub fn provider_name(&self, model: &str) -> Option<String> {
        self.registry
            .find_for_model(model)
            .map(|provider| provider.name().to_string())
    }

    /// Embeddings use the configured model provider.
    pub async fn embeddings(&self, request: EmbeddingRequest) -> ProviderResult<EmbeddingResponse> {
        let provider = self
            .registry
            .find_for_model(&request.model)
            .ok_or_else(|| ProviderError::ModelNotSupported(request.model.clone()))?;
        provider.embeddings(request).await
    }

    /// Open a stream; never replay an already opened response.
    pub async fn chat_completion_stream(
        &self,
        request: ChatCompletionRequest,
    ) -> ProviderResult<crate::provider::CompletionStream> {
        if !matches!(self.config.strategy, RoutingStrategy::Failover) {
            return Err(ProviderError::Unavailable(
                "Only failover routing is implemented".into(),
            ));
        }
        let provider = self
            .registry
            .find_for_model(&request.model)
            .ok_or_else(|| ProviderError::ModelNotSupported(request.model.clone()))?;
        provider.chat_completion_stream(request).await
    }

    async fn try_provider(
        &self,
        provider: &Arc<dyn Provider>,
        request: ChatCompletionRequest,
    ) -> ProviderResult<ChatCompletionResponse> {
        /// Longest wait honoured when a provider asks us to slow down.
        const MAX_BACKOFF_MS: u64 = 10_000;
        let mut attempt = 0;
        loop {
            let error = match provider.chat_completion(request.clone()).await {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            // A request the provider rejected would be rejected again.
            if !error.is_retriable() || attempt >= self.config.max_retries {
                return Err(error);
            }
            attempt += 1;
            let mut delay = self.config.retry_delay_ms.saturating_mul(attempt as u64);
            if let ProviderError::RateLimited { retry_after_ms } = &error {
                delay = delay.max(*retry_after_ms);
            }
            tokio::time::sleep(tokio::time::Duration::from_millis(
                delay.min(MAX_BACKOFF_MS),
            ))
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_core::llm::{EmbeddingRequest, EmbeddingResponse};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Fails with the given status until `succeed_after` calls were made.
    struct Flaky {
        calls: Arc<AtomicUsize>,
        status: u16,
        succeed_after: usize,
    }

    #[async_trait::async_trait]
    impl Provider for Flaky {
        fn name(&self) -> &str {
            "flaky"
        }
        fn supported_models(&self) -> &[&str] {
            &["m"]
        }
        async fn chat_completion(
            &self,
            request: ChatCompletionRequest,
        ) -> ProviderResult<ChatCompletionResponse> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call < self.succeed_after {
                return Err(match self.status {
                    429 => ProviderError::RateLimited { retry_after_ms: 1 },
                    status => ProviderError::Api {
                        status,
                        message: "no".into(),
                    },
                });
            }
            Ok(ChatCompletionResponse {
                id: "1".into(),
                object: "chat.completion".into(),
                created: 0,
                model: request.model,
                choices: vec![],
                usage: None,
            })
        }
        async fn chat_completion_stream(
            &self,
            _: ChatCompletionRequest,
        ) -> ProviderResult<crate::provider::CompletionStream> {
            Err(ProviderError::Unavailable("no stream".into()))
        }
        async fn embeddings(&self, _: EmbeddingRequest) -> ProviderResult<EmbeddingResponse> {
            Err(ProviderError::Unavailable("no embeddings".into()))
        }
    }

    async fn run(status: u16, succeed_after: usize) -> (bool, usize) {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut registry = ProviderRegistry::new();
        registry.register(Arc::new(Flaky {
            calls: calls.clone(),
            status,
            succeed_after,
        }));
        let router = Router::new(
            Arc::new(registry),
            RouterConfig {
                retry_delay_ms: 1,
                ..RouterConfig::default()
            },
        );
        let request = serde_json::from_value(
            serde_json::json!({ "model": "m", "messages": [{ "role": "user", "content": "hi" }] }),
        )
        .unwrap();
        let ok = router.chat_completion(request, &[]).await.is_ok();
        (ok, calls.load(Ordering::SeqCst))
    }

    #[tokio::test]
    async fn rejected_request_is_not_retried() {
        // 400: the provider refused this request; sending it again is waste.
        assert_eq!(run(400, usize::MAX).await, (false, 1));
        assert_eq!(run(404, usize::MAX).await, (false, 1));
    }

    #[tokio::test]
    async fn server_errors_and_rate_limits_are_retried() {
        assert_eq!(run(503, 2).await, (true, 3));
        assert_eq!(run(429, 1).await, (true, 2));
        // Gives up after the configured retries: 1 attempt + 3 retries.
        assert_eq!(run(500, usize::MAX).await, (false, 4));
    }
}
