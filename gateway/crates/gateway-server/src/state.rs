//! Application state

use gateway_core::config::GatewayConfig;
use gateway_llm::{ProviderRegistry, Router, router::RouterConfig};
use gateway_sandbox::{pool::PoolConfig, SandboxPool};
use std::sync::Arc;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<GatewayConfig>,
    pub llm_router: Arc<Router>,
    pub sandbox_pool: Arc<SandboxPool>,
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

        Self {
            config: Arc::new(config),
            llm_router,
            sandbox_pool,
        }
    }
}
