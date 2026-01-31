//! Executor - Worker that processes tasks.
//!
//! # Quick Start
//!
//! ```rust
//! let executor = Executor::new("exe-001", "sup-001", bus)
//!     .llm(LLMConfig::gemini("gemini-2.0-flash"))
//!     .start().await?;
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

use agentic_llm::{
    AnthropicAdapter, GeminiAdapter, LLMAdapter, LLMError, LLMMessage, OpenAIAdapter,
};

use super::config::{Capability, ExecutorConfig, LLMConfig, SandboxConfig};

/// Current state of an executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExecutorState {
    /// Ready for tasks
    Idle,
    /// Processing a task
    Busy {
        task_id: String,
        started_at: DateTime<Utc>,
    },
    /// Lent to another supervisor
    Borrowed {
        to_supervisor: String,
        lease_id: String,
        expires_at: DateTime<Utc>,
    },
    /// Stopped
    Stopped,
}

/// An executor that processes tasks.
#[derive(Debug)]
pub struct Executor {
    /// Unique identifier
    pub id: String,
    /// Owner supervisor
    pub owner: String,
    /// Current controlling supervisor (may differ if borrowed)
    pub current_supervisor: String,
    /// Current state
    state: Arc<RwLock<ExecutorState>>,
    /// LLM configuration
    llm_config: LLMConfig,
    /// Capabilities
    capabilities: HashSet<Capability>,
    /// Metrics
    tasks_completed: Arc<RwLock<u64>>,
    tasks_failed: Arc<RwLock<u64>>,
}

impl Executor {
    /// Create a new executor with minimal config.
    ///
    /// # Example
    ///
    /// ```rust
    /// let exe = Executor::new("exe-001", "sup-001")
    ///     .llm(LLMConfig::gemini("gemini-2.0-flash"))
    ///     .build();
    /// ```
    pub fn new(id: impl Into<String>, owner: impl Into<String>) -> ExecutorBuilder {
        ExecutorBuilder::new(id.into(), owner.into())
    }

    /// Create from full config.
    pub fn from_config(config: ExecutorConfig) -> Self {
        Self {
            id: config.id,
            owner: config.owner_supervisor.clone(),
            current_supervisor: config.owner_supervisor,
            state: Arc::new(RwLock::new(ExecutorState::Idle)),
            llm_config: config.llm,
            capabilities: config.capabilities,
            tasks_completed: Arc::new(RwLock::new(0)),
            tasks_failed: Arc::new(RwLock::new(0)),
        }
    }

    /// Get current state.
    pub async fn state(&self) -> ExecutorState {
        self.state.read().await.clone()
    }

    /// Check if idle.
    pub async fn is_idle(&self) -> bool {
        matches!(*self.state.read().await, ExecutorState::Idle)
    }

    /// Check if borrowed.
    pub async fn is_borrowed(&self) -> bool {
        matches!(*self.state.read().await, ExecutorState::Borrowed { .. })
    }

    /// Set state to busy.
    pub async fn set_busy(&self, task_id: String) {
        *self.state.write().await = ExecutorState::Busy {
            task_id,
            started_at: Utc::now(),
        };
    }

    /// Set state to idle.
    pub async fn set_idle(&self) {
        *self.state.write().await = ExecutorState::Idle;
    }

    /// Set state to borrowed.
    pub async fn set_borrowed(
        &self,
        to_supervisor: String,
        lease_id: String,
        expires_at: DateTime<Utc>,
    ) {
        // Note: current_supervisor tracking would need interior mutability
        // For now, we just update the state
        *self.state.write().await = ExecutorState::Borrowed {
            to_supervisor,
            lease_id,
            expires_at,
        };
    }

    /// Return to owner.
    pub async fn return_to_owner(&mut self) {
        self.current_supervisor = self.owner.clone();
        *self.state.write().await = ExecutorState::Idle;
    }

    /// Increment completed tasks.
    pub async fn increment_completed(&self) {
        *self.tasks_completed.write().await += 1;
    }

    /// Increment failed tasks.
    pub async fn increment_failed(&self) {
        *self.tasks_failed.write().await += 1;
    }

    /// Get capabilities.
    pub fn capabilities(&self) -> &HashSet<Capability> {
        &self.capabilities
    }

    /// Check if has capability.
    pub fn has_capability(&self, cap: Capability) -> bool {
        self.capabilities.contains(&cap)
    }

    /// Get LLM config.
    pub fn llm_config(&self) -> &LLMConfig {
        &self.llm_config
    }

    /// Execute a task with the LLM.
    ///
    /// This makes a real API call to the configured LLM provider.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// let result = executor.execute(
    ///     "You are a Rust expert.",
    ///     "Write a function to check if a number is prime."
    /// ).await?;
    /// ```
    pub async fn execute(
        &self,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<ExecutionResult, ExecutionError> {
        let task_id = format!("task-{}", uuid::Uuid::new_v4());

        // Mark as busy
        self.set_busy(task_id.clone()).await;

        let start = std::time::Instant::now();

        // Create LLM adapter based on config
        let result = match self.llm_config.provider.as_str() {
            "gemini" => {
                let adapter = GeminiAdapter::new(&self.llm_config.api_key, &self.llm_config.model)
                    .with_temperature(self.llm_config.temperature)
                    .with_max_tokens(self.llm_config.max_tokens);

                let messages = vec![
                    LLMMessage::system(system_prompt),
                    LLMMessage::user(user_prompt),
                ];

                adapter.generate(&messages).await
            }
            "openai" => {
                let adapter = OpenAIAdapter::new(&self.llm_config.api_key, &self.llm_config.model)
                    .with_temperature(self.llm_config.temperature)
                    .with_max_tokens(self.llm_config.max_tokens);

                let messages = vec![
                    LLMMessage::system(system_prompt),
                    LLMMessage::user(user_prompt),
                ];

                adapter.generate(&messages).await
            }
            "anthropic" => {
                let adapter =
                    AnthropicAdapter::new(&self.llm_config.api_key, &self.llm_config.model)
                        .with_temperature(self.llm_config.temperature)
                        .with_max_tokens(self.llm_config.max_tokens);

                let messages = vec![
                    LLMMessage::system(system_prompt),
                    LLMMessage::user(user_prompt),
                ];

                adapter.generate(&messages).await
            }
            other => {
                self.set_idle().await;
                return Err(ExecutionError::UnsupportedProvider(other.to_string()));
            }
        };

        let duration = start.elapsed();

        match result {
            Ok(response) => {
                self.increment_completed().await;
                self.set_idle().await;

                // Check if response was truncated
                let truncated = response.finish_reason == agentic_llm::FinishReason::Length;
                if truncated {
                    tracing::warn!(
                        "LLM output truncated (hit max_tokens limit). Model: {}, tokens: {}",
                        response.model,
                        response.tokens_used.total
                    );
                }

                Ok(ExecutionResult {
                    task_id,
                    executor_id: self.id.clone(),
                    content: response.content,
                    tokens_used: response.tokens_used.total,
                    duration_ms: duration.as_millis() as u64,
                    model: response.model,
                    truncated,
                })
            }
            Err(e) => {
                self.increment_failed().await;
                self.set_idle().await;

                Err(ExecutionError::LLMError(e))
            }
        }
    }
}

/// Result of an execution.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Task ID
    pub task_id: String,
    /// Executor that processed it
    pub executor_id: String,
    /// Generated content
    pub content: String,
    /// Tokens used
    pub tokens_used: u32,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Model used
    pub model: String,
    /// Whether output was truncated (hit max tokens)
    pub truncated: bool,
}

impl ExecutionResult {
    /// Check if the response was truncated due to max tokens limit.
    pub fn is_truncated(&self) -> bool {
        self.truncated
    }
}

/// Execution errors.
#[derive(Debug, thiserror::Error)]
pub enum ExecutionError {
    #[error("LLM error: {0}")]
    LLMError(#[from] LLMError),

    #[error("Unsupported provider: {0}")]
    UnsupportedProvider(String),

    #[error("Executor not idle")]
    NotIdle,
}

/// Builder for creating executors.
pub struct ExecutorBuilder {
    id: String,
    owner: String,
    llm: Option<LLMConfig>,
    sandbox: Option<SandboxConfig>,
    capabilities: HashSet<Capability>,
}

impl ExecutorBuilder {
    fn new(id: String, owner: String) -> Self {
        Self {
            id,
            owner,
            llm: None,
            sandbox: None,
            capabilities: HashSet::from([Capability::CodeGeneration]),
        }
    }

    /// Set LLM configuration.
    pub fn llm(mut self, config: LLMConfig) -> Self {
        self.llm = Some(config);
        self
    }

    /// Use Gemini (shortcut).
    pub fn llm_gemini(self, model: &str) -> Self {
        self.llm(LLMConfig::gemini(model))
    }

    /// Use OpenAI (shortcut).
    pub fn llm_openai(self, model: &str) -> Self {
        self.llm(LLMConfig::openai(model))
    }

    /// Use Anthropic/Claude (shortcut).
    pub fn llm_anthropic(self, model: &str) -> Self {
        self.llm(LLMConfig::anthropic(model))
    }

    /// Set sandbox configuration.
    pub fn sandbox(mut self, config: SandboxConfig) -> Self {
        self.sandbox = Some(config);
        self
    }

    /// Use process sandbox (shortcut).
    pub fn sandbox_process(self) -> Self {
        self.sandbox(SandboxConfig::process())
    }

    /// Use docker sandbox (shortcut).
    pub fn sandbox_docker(self) -> Self {
        self.sandbox(SandboxConfig::docker())
    }

    /// No sandbox (generation only).
    pub fn no_sandbox(self) -> Self {
        self.sandbox(SandboxConfig::none())
    }

    /// Add capability.
    pub fn capability(mut self, cap: Capability) -> Self {
        self.capabilities.insert(cap);
        self
    }

    /// Set capabilities.
    pub fn capabilities(mut self, caps: impl IntoIterator<Item = Capability>) -> Self {
        self.capabilities = caps.into_iter().collect();
        self
    }

    /// Build the executor.
    pub fn build(self) -> Executor {
        Executor::from_config(ExecutorConfig {
            id: self.id,
            owner_supervisor: self.owner,
            llm: self
                .llm
                .unwrap_or_else(|| LLMConfig::gemini("gemini-2.0-flash")),
            sandbox: self.sandbox.unwrap_or_else(SandboxConfig::process),
            capabilities: self.capabilities,
        })
    }
}

/// Generate a unique executor ID.
pub fn generate_executor_id(supervisor_id: &str, index: usize) -> String {
    format!("exe-{}-{:03}", supervisor_id, index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_executor_builder() {
        let exe = Executor::new("exe-001", "sup-001")
            .llm_gemini("gemini-2.0-flash")
            .sandbox_process()
            .capability(Capability::CodeExecution)
            .build();

        assert_eq!(exe.id, "exe-001");
        assert_eq!(exe.owner, "sup-001");
        assert!(exe.has_capability(Capability::CodeGeneration));
        assert!(exe.has_capability(Capability::CodeExecution));
    }

    #[tokio::test]
    async fn test_executor_state() {
        let exe = Executor::new("exe-001", "sup-001").build();

        assert!(exe.is_idle().await);

        exe.set_busy("task-123".into()).await;
        assert!(!exe.is_idle().await);

        exe.set_idle().await;
        assert!(exe.is_idle().await);
    }
}
