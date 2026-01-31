//! Sandbox pooling for fast allocation

use crate::firecracker::{FirecrackerConfig, FirecrackerInstance, FirecrackerManager, FirecrackerResult};
use gateway_core::sandbox::{CreateSandboxRequest, Runtime, SandboxId, SandboxStatus};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, info, warn};

/// Pool configuration
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Minimum number of ready sandboxes to maintain
    pub min_ready: usize,
    /// Maximum total sandboxes
    pub max_total: usize,
    /// Idle timeout before recycling (seconds)
    pub idle_timeout_secs: u64,
    /// Warmup sandboxes for each runtime
    pub warmup_runtimes: Vec<Runtime>,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_ready: 2,
            max_total: 10,
            idle_timeout_secs: 300,
            warmup_runtimes: vec![Runtime::Python, Runtime::Node],
        }
    }
}

/// Sandbox pool for fast allocation
pub struct SandboxPool {
    manager: Arc<FirecrackerManager>,
    config: PoolConfig,
    /// Available (ready) sandboxes by runtime
    available: RwLock<HashMap<Runtime, Vec<FirecrackerInstance>>>,
    /// In-use sandboxes by ID
    in_use: RwLock<HashMap<String, FirecrackerInstance>>,
    /// Lock for pool operations
    pool_lock: Mutex<()>,
}

impl SandboxPool {
    pub fn new(firecracker_config: FirecrackerConfig, pool_config: PoolConfig) -> Self {
        Self {
            manager: Arc::new(FirecrackerManager::new(firecracker_config)),
            config: pool_config,
            available: RwLock::new(HashMap::new()),
            in_use: RwLock::new(HashMap::new()),
            pool_lock: Mutex::new(()),
        }
    }

    /// Initialize the pool with warm sandboxes
    pub async fn initialize(&self) -> FirecrackerResult<()> {
        info!(
            min_ready = self.config.min_ready,
            runtimes = ?self.config.warmup_runtimes,
            "Initializing sandbox pool"
        );

        for runtime in &self.config.warmup_runtimes {
            for _ in 0..self.config.min_ready {
                match self.create_warm_sandbox(runtime.clone()).await {
                    Ok(_) => {}
                    Err(e) => {
                        warn!(runtime = ?runtime, error = %e, "Failed to create warm sandbox");
                    }
                }
            }
        }

        Ok(())
    }

    /// Acquire a sandbox from the pool or create a new one
    pub async fn acquire(&self, request: CreateSandboxRequest) -> FirecrackerResult<SandboxId> {
        let _lock = self.pool_lock.lock().await;
        let runtime = request.runtime.clone();

        // Try to get from available pool first
        {
            let mut available = self.available.write().await;
            if let Some(instances) = available.get_mut(&runtime) {
                if let Some(instance) = instances.pop() {
                    let id = instance.id.clone();
                    info!(sandbox_id = %id.as_str(), "Acquired sandbox from pool");

                    let mut in_use = self.in_use.write().await;
                    in_use.insert(id.as_str().to_string(), instance);

                    // Trigger background replenishment
                    drop(available);
                    self.maybe_replenish(&runtime).await;

                    return Ok(id);
                }
            }
        }

        // Check if we can create a new one
        let total = self.total_count().await;
        if total >= self.config.max_total {
            return Err(crate::firecracker::FirecrackerError::ApiError(
                "Pool exhausted".to_string(),
            ));
        }

        // Create a new sandbox
        let instance = self.manager.create(request).await?;
        let id = instance.id.clone();

        let mut in_use = self.in_use.write().await;
        in_use.insert(id.as_str().to_string(), instance);

        Ok(id)
    }

    /// Release a sandbox back to the pool or destroy it
    pub async fn release(&self, id: &SandboxId) -> FirecrackerResult<()> {
        let _lock = self.pool_lock.lock().await;

        let mut in_use = self.in_use.write().await;
        if let Some(mut instance) = in_use.remove(id.as_str()) {
            let runtime = instance.runtime.clone();

            // Check if we should recycle or destroy
            let available_count = {
                let available = self.available.read().await;
                available
                    .get(&runtime)
                    .map(|v: &Vec<FirecrackerInstance>| v.len())
                    .unwrap_or(0)
            };

            if available_count < self.config.min_ready && instance.status == SandboxStatus::Ready {
                // Return to pool
                debug!(sandbox_id = %id.as_str(), "Returning sandbox to pool");
                let mut available = self.available.write().await;
                available
                    .entry(runtime)
                    .or_default()
                    .push(instance);
            } else {
                // Destroy
                debug!(sandbox_id = %id.as_str(), "Destroying sandbox");
                instance.stop().await?;
            }
        }

        Ok(())
    }

    /// Get a sandbox by ID
    pub async fn get(&self, _id: &SandboxId) -> Option<()> {
        // Note: This is a simplified version. In production, we'd need
        // interior mutability or a different approach
        None
    }

    async fn create_warm_sandbox(&self, runtime: Runtime) -> FirecrackerResult<()> {
        let request = CreateSandboxRequest {
            runtime: runtime.clone(),
            packages: vec![],
            timeout_seconds: None,
            memory_mb: None,
        };

        let instance = self.manager.create(request).await?;
        let id = instance.id.clone();

        let mut available = self.available.write().await;
        available.entry(runtime).or_default().push(instance);

        debug!(sandbox_id = %id.as_str(), "Created warm sandbox");
        Ok(())
    }

    async fn maybe_replenish(&self, runtime: &Runtime) {
        let count = {
            let available = self.available.read().await;
            available
                .get(runtime)
                .map(|v: &Vec<FirecrackerInstance>| v.len())
                .unwrap_or(0)
        };

        if count < self.config.min_ready {
            let runtime = runtime.clone();
            let manager = self.manager.clone();
            let min_ready = self.config.min_ready;

            // Spawn background task to replenish
            tokio::spawn(async move {
                let needed = min_ready - count;
                for _ in 0..needed {
                    let request = CreateSandboxRequest {
                        runtime: runtime.clone(),
                        packages: vec![],
                        timeout_seconds: None,
                        memory_mb: None,
                    };
                    if let Err(e) = manager.create(request).await {
                        warn!(error = %e, "Failed to replenish pool");
                    }
                }
            });
        }
    }

    async fn total_count(&self) -> usize {
        let available = self.available.read().await;
        let in_use = self.in_use.read().await;

        let available_count: usize = available
            .values()
            .map(|v: &Vec<FirecrackerInstance>| v.len())
            .sum();
        available_count + in_use.len()
    }

    /// Get pool statistics
    pub async fn stats(&self) -> PoolStats {
        let available = self.available.read().await;
        let in_use = self.in_use.read().await;

        PoolStats {
            available: available
                .values()
                .map(|v: &Vec<FirecrackerInstance>| v.len())
                .sum(),
            in_use: in_use.len(),
            max_total: self.config.max_total,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PoolStats {
    pub available: usize,
    pub in_use: usize,
    pub max_total: usize,
}
