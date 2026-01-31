//! Executor pool management.
//!
//! # Quick Start
//!
//! ```rust
//! let pool = ExecutorPool::new("sup-001", 4)
//!     .llm(LLMConfig::gemini("gemini-2.0-flash"))
//!     .build();
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::config::{LLMConfig, SandboxConfig};
use super::executor::{generate_executor_id, Executor};

/// Pool configuration.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Minimum idle executors
    pub min_idle: usize,
    /// Maximum pool size
    pub max_size: usize,
    /// Warmup on start
    pub warmup_on_start: bool,
    /// Scale up threshold (0.0-1.0)
    pub scale_up_threshold: f32,
    /// Scale down threshold (0.0-1.0)
    pub scale_down_threshold: f32,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            min_idle: 2,
            max_size: 10,
            warmup_on_start: true,
            scale_up_threshold: 0.8,
            scale_down_threshold: 0.2,
        }
    }
}

/// Pool of executors for a supervisor.
#[derive(Debug)]
pub struct ExecutorPool {
    /// Owner supervisor
    supervisor_id: String,
    /// All executors
    executors: Arc<RwLock<HashMap<String, Arc<Executor>>>>,
    /// LLM config for new executors
    llm_config: LLMConfig,
    /// Sandbox config for new executors
    sandbox_config: SandboxConfig,
    /// Pool config
    config: PoolConfig,
    /// Next executor index
    next_index: Arc<RwLock<usize>>,
}

impl ExecutorPool {
    /// Create a new pool.
    ///
    /// # Example
    ///
    /// ```rust
    /// let pool = ExecutorPool::new("sup-001", 4)
    ///     .llm_gemini("gemini-2.0-flash")
    ///     .build();
    /// ```
    pub fn new(supervisor_id: impl Into<String>, initial_size: usize) -> ExecutorPoolBuilder {
        ExecutorPoolBuilder::new(supervisor_id.into(), initial_size)
    }

    /// Get an idle executor.
    pub async fn get_idle(&self) -> Option<Arc<Executor>> {
        let executors = self.executors.read().await;
        for exe in executors.values() {
            if exe.is_idle().await {
                return Some(Arc::clone(exe));
            }
        }
        None
    }

    /// Get all idle executors.
    pub async fn get_all_idle(&self) -> Vec<Arc<Executor>> {
        let executors = self.executors.read().await;
        let mut idle = Vec::new();
        for exe in executors.values() {
            if exe.is_idle().await {
                idle.push(Arc::clone(exe));
            }
        }
        idle
    }

    /// Get N idle executors.
    pub async fn get_n_idle(&self, n: usize) -> Vec<Arc<Executor>> {
        let executors = self.executors.read().await;
        let mut idle = Vec::new();
        for exe in executors.values() {
            if idle.len() >= n {
                break;
            }
            if exe.is_idle().await {
                idle.push(Arc::clone(exe));
            }
        }
        idle
    }

    /// Get executor by ID.
    pub async fn get(&self, id: &str) -> Option<Arc<Executor>> {
        self.executors.read().await.get(id).cloned()
    }

    /// Get pool size.
    pub async fn size(&self) -> usize {
        self.executors.read().await.len()
    }

    /// Get idle count.
    pub async fn idle_count(&self) -> usize {
        let executors = self.executors.read().await;
        let mut count = 0;
        for exe in executors.values() {
            if exe.is_idle().await {
                count += 1;
            }
        }
        count
    }

    /// Get busy count.
    pub async fn busy_count(&self) -> usize {
        self.size().await - self.idle_count().await
    }

    /// Get utilization (0.0-1.0).
    pub async fn utilization(&self) -> f32 {
        let size = self.size().await;
        if size == 0 {
            return 0.0;
        }
        self.busy_count().await as f32 / size as f32
    }

    /// Add a new executor to the pool.
    pub async fn add_executor(&self) -> Arc<Executor> {
        let mut next_index = self.next_index.write().await;
        let id = generate_executor_id(&self.supervisor_id, *next_index);
        *next_index += 1;

        let executor = Arc::new(
            Executor::new(&id, &self.supervisor_id)
                .llm(self.llm_config.clone())
                .sandbox(self.sandbox_config.clone())
                .build(),
        );

        self.executors
            .write()
            .await
            .insert(id, Arc::clone(&executor));
        executor
    }

    /// Remove an idle executor.
    pub async fn remove_idle(&self) -> Option<String> {
        let mut executors = self.executors.write().await;

        // Find an idle executor
        let idle_id = {
            let mut found = None;
            for (id, exe) in executors.iter() {
                if exe.is_idle().await {
                    found = Some(id.clone());
                    break;
                }
            }
            found
        };

        if let Some(id) = idle_id {
            executors.remove(&id);
            Some(id)
        } else {
            None
        }
    }

    /// Scale up by n executors.
    pub async fn scale_up(&self, n: usize) {
        for _ in 0..n {
            if self.size().await >= self.config.max_size {
                break;
            }
            self.add_executor().await;
        }
    }

    /// Scale down by n executors.
    pub async fn scale_down(&self, n: usize) {
        for _ in 0..n {
            if self.size().await <= self.config.min_idle {
                break;
            }
            self.remove_idle().await;
        }
    }

    /// Auto-scale based on utilization.
    pub async fn auto_scale(&self) {
        let util = self.utilization().await;
        let size = self.size().await;

        if util > self.config.scale_up_threshold && size < self.config.max_size {
            // Scale up
            let to_add = ((size as f32 * 0.5) as usize).max(1);
            self.scale_up(to_add).await;
        } else if util < self.config.scale_down_threshold && size > self.config.min_idle {
            // Scale down
            self.scale_down(1).await;
        }
    }

    /// Get all executor IDs.
    pub async fn executor_ids(&self) -> Vec<String> {
        self.executors.read().await.keys().cloned().collect()
    }
}

/// Builder for ExecutorPool.
pub struct ExecutorPoolBuilder {
    supervisor_id: String,
    initial_size: usize,
    llm: Option<LLMConfig>,
    sandbox: Option<SandboxConfig>,
    config: PoolConfig,
}

impl ExecutorPoolBuilder {
    fn new(supervisor_id: String, initial_size: usize) -> Self {
        Self {
            supervisor_id,
            initial_size,
            llm: None,
            sandbox: None,
            config: PoolConfig::default(),
        }
    }

    /// Set LLM config.
    pub fn llm(mut self, config: LLMConfig) -> Self {
        self.llm = Some(config);
        self
    }

    /// Use Gemini (shortcut).
    pub fn llm_gemini(self, model: &str) -> Self {
        self.llm(LLMConfig::gemini(model))
    }

    /// Set sandbox config.
    pub fn sandbox(mut self, config: SandboxConfig) -> Self {
        self.sandbox = Some(config);
        self
    }

    /// Use process sandbox.
    pub fn sandbox_process(self) -> Self {
        self.sandbox(SandboxConfig::process())
    }

    /// Set pool config.
    pub fn pool_config(mut self, config: PoolConfig) -> Self {
        self.config = config;
        self
    }

    /// Set min idle.
    pub fn min_idle(mut self, n: usize) -> Self {
        self.config.min_idle = n;
        self
    }

    /// Set max size.
    pub fn max_size(mut self, n: usize) -> Self {
        self.config.max_size = n;
        self
    }

    /// Build the pool.
    pub fn build(self) -> ExecutorPool {
        let llm_config = self
            .llm
            .unwrap_or_else(|| LLMConfig::gemini("gemini-2.0-flash"));
        let sandbox_config = self.sandbox.unwrap_or_else(SandboxConfig::process);

        let pool = ExecutorPool {
            supervisor_id: self.supervisor_id.clone(),
            executors: Arc::new(RwLock::new(HashMap::new())),
            llm_config: llm_config.clone(),
            sandbox_config: sandbox_config.clone(),
            config: self.config,
            next_index: Arc::new(RwLock::new(0)),
        };

        // Create initial executors synchronously (IDs only)
        // Actual creation happens in an async context
        pool
    }

    /// Build and initialize the pool (async).
    pub async fn build_and_init(self) -> ExecutorPool {
        let initial_size = self.initial_size;
        let pool = self.build();

        // Create initial executors
        for _ in 0..pool.config.min_idle.max(initial_size) {
            pool.add_executor().await;
        }

        pool
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pool_creation() {
        let pool = ExecutorPool::new("sup-001", 4)
            .llm_gemini("gemini-2.0-flash")
            .build_and_init()
            .await;

        assert_eq!(pool.size().await, 4);
        assert_eq!(pool.idle_count().await, 4);
    }

    #[tokio::test]
    async fn test_pool_get_idle() {
        let pool = ExecutorPool::new("sup-001", 2).build_and_init().await;

        let exe = pool.get_idle().await;
        assert!(exe.is_some());
    }

    #[tokio::test]
    async fn test_pool_scale() {
        let pool = ExecutorPool::new("sup-001", 2)
            .max_size(10)
            .build_and_init()
            .await;

        assert_eq!(pool.size().await, 2);

        pool.scale_up(3).await;
        assert_eq!(pool.size().await, 5);

        pool.scale_down(2).await;
        assert_eq!(pool.size().await, 3);
    }
}
