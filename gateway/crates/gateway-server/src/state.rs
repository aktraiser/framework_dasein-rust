//! Application state

use gateway_core::config::GatewayConfig;
use gateway_core::storage::Storage;
use gateway_llm::{router::RouterConfig, ProviderRegistry, Router};
use gateway_sandbox::{
    pool::PoolConfig, CodeExecutor, ExecutionBackend, FirecrackerOrchestrator, OrchestratorConfig,
    RemoteSandboxConfig, SandboxPool,
};
use std::sync::Arc;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub boxes: Arc<crate::boxes::Boxes>,
    pub config: Arc<GatewayConfig>,
    pub operator_api_key: Option<String>,
    pub llm_router: Arc<Router>,
    pub sandbox_pool: Arc<SandboxPool>,
    pub code_executor: Arc<CodeExecutor>,
    pub orchestrator: Option<Arc<FirecrackerOrchestrator>>,
    pub storage: Option<Arc<Storage>>,
}

impl AppState {
    pub fn new(config: GatewayConfig) -> Self {
        // Initialize LLM router
        let llm_router = Self::configured_router(&config);

        // Initialize sandbox pool
        let firecracker_config = gateway_sandbox::firecracker::FirecrackerConfig {
            binary_path: config.sandbox.firecracker_path.clone().into(),
            kernel_path: config
                .sandbox
                .kernel_path
                .clone()
                .unwrap_or_default()
                .into(),
            rootfs_path: config
                .sandbox
                .rootfs_path
                .clone()
                .unwrap_or_default()
                .into(),
            ..Default::default()
        };

        let pool_config = PoolConfig {
            min_ready: config.sandbox.pool.min_ready,
            max_total: config.sandbox.pool.max_total,
            idle_timeout_secs: config.sandbox.pool.idle_timeout_seconds,
            ..Default::default()
        };

        let sandbox_pool = Arc::new(SandboxPool::new(firecracker_config, pool_config));

        // Initialize code executor with remote backend if configured
        let execution_backend = if let Ok(url) = std::env::var("FIRECRACKER_REMOTE_URL") {
            tracing::info!(url = %url, "Using remote Firecracker backend");
            ExecutionBackend::Remote(RemoteSandboxConfig::new(url))
        } else {
            tracing::info!("Using local Firecracker backend (placeholder)");
            ExecutionBackend::Local
        };

        let code_executor = Arc::new(CodeExecutor::with_backend(
            sandbox_pool.clone(),
            execution_backend,
        ));

        // Initialize orchestrator if SSH credentials are configured
        let orchestrator = if std::env::var("FIRECRACKER_SSH_PASSWORD").is_ok() {
            tracing::info!("Firecracker orchestrator enabled (SSH management)");
            Some(Arc::new(FirecrackerOrchestrator::new(
                OrchestratorConfig::default(),
            )))
        } else {
            tracing::info!("Firecracker orchestrator disabled (no SSH password configured)");
            None
        };

        Self {
            boxes: Arc::new(crate::boxes::Boxes::default()),
            operator_api_key: std::env::var("GATEWAY_API_KEY")
                .ok()
                .filter(|k| !k.is_empty()),
            config: Arc::new(config),
            llm_router,
            sandbox_pool,
            code_executor,
            orchestrator,
            storage: None,
        }
    }

    fn configured_router(config: &GatewayConfig) -> Arc<Router> {
        let mut registry = ProviderRegistry::new();
        for (name, provider) in &config.llm.providers {
            if !provider.enabled {
                continue;
            }
            let Some(key) = provider.api_key.as_ref().filter(|k| !k.is_empty()) else {
                continue;
            };
            match name.as_str() {
                "openai" => {
                    let mut p = gateway_llm::providers::openai::OpenAIProvider::new(key);
                    if let Some(base) = &provider.api_base {
                        p = p.with_base_url(base);
                    }
                    registry.register(Arc::new(p));
                }
                "anthropic" => {
                    let mut p = gateway_llm::providers::anthropic::AnthropicProvider::new(key);
                    if let Some(base) = &provider.api_base {
                        p = p.with_base_url(base);
                    }
                    registry.register(Arc::new(p));
                }
                _ => tracing::warn!(provider = %name, "Unsupported provider configuration"),
            }
        }
        for (model, route) in &config.llm.models {
            registry.route_model(model.clone(), route.provider.clone());
        }
        let registry = Arc::new(registry);
        Arc::new(Router::new(registry, RouterConfig::default()))
    }

    /// Resolve current persisted provider settings for each request.
    pub async fn runtime_router(&self) -> Result<Arc<Router>, gateway_core::storage::StorageError> {
        let Some(storage) = &self.storage else {
            return Ok(self.llm_router.clone());
        };
        let mut config = (*self.config).clone();
        for provider in storage.list_providers().await? {
            let key = if provider.enabled {
                storage.decrypt_provider_key(&provider)?
            } else {
                None
            };
            config.llm.providers.insert(
                provider.name.clone(),
                gateway_core::config::ProviderConfig {
                    api_key: key,
                    api_base: provider.api_base,
                    enabled: provider.enabled,
                },
            );
        }
        Ok(Self::configured_router(&config))
    }

    /// Set the storage backend
    pub fn with_storage(mut self, storage: Storage) -> Self {
        self.storage = Some(Arc::new(storage));
        self
    }
}
