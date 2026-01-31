//! Application state

use gateway_core::config::GatewayConfig;
use gateway_core::storage::Storage;
use gateway_llm::{ProviderRegistry, Router, router::RouterConfig};
use gateway_sandbox::{
    pool::PoolConfig,
    CodeExecutor,
    ExecutionBackend,
    RemoteSandboxConfig,
    SandboxPool,
    FirecrackerOrchestrator,
    OrchestratorConfig,
};
use std::sync::Arc;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<GatewayConfig>,
    pub llm_router: Arc<Router>,
    pub sandbox_pool: Arc<SandboxPool>,
    pub code_executor: Arc<CodeExecutor>,
    pub orchestrator: Option<Arc<FirecrackerOrchestrator>>,
    pub storage: Option<Arc<Storage>>,
}

impl AppState {
    pub fn new(config: GatewayConfig) -> Self {
        // Initialize LLM router
        let registry = Arc::new(ProviderRegistry::new());
        let llm_router = Arc::new(Router::new(registry, RouterConfig::default()));

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
            Some(Arc::new(FirecrackerOrchestrator::new(OrchestratorConfig::default())))
        } else {
            tracing::info!("Firecracker orchestrator disabled (no SSH password configured)");
            None
        };

        Self {
            config: Arc::new(config),
            llm_router,
            sandbox_pool,
            code_executor,
            orchestrator,
            storage: None,
        }
    }

    /// Set the storage backend
    pub fn with_storage(mut self, storage: Storage) -> Self {
        self.storage = Some(Arc::new(storage));
        self
    }
}
