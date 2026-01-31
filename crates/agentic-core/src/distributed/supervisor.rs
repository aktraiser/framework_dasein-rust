//! Supervisor - Orchestrates executors and handles task routing.
//!
//! # Quick Start
//!
//! ```rust
//! // Create a supervisor with 4 executors in one line
//! let sup = Supervisor::new("my-sup")
//!     .domain("code")
//!     .executors(4)
//!     .llm_gemini("gemini-2.0-flash")
//!     .build();
//!
//! // Or even simpler
//! let sup = Supervisor::quick("my-sup", 4);
//! ```

use std::collections::HashSet;
use std::sync::Arc;

use super::allocation::{AllocationGrant, AllocationManager, AllocationRequest};
use super::config::{Capability, LLMConfig, SandboxConfig, SupervisorConfig};
use super::executor::Executor;
use super::pool::ExecutorPool;
use super::validator::{ValidationRule, Validator};

/// Handle to interact with a supervisor.
#[derive(Debug, Clone)]
pub struct SupervisorHandle {
    pub id: String,
    pub domain: String,
}

/// Supervisor that orchestrates executors.
#[derive(Debug)]
pub struct Supervisor {
    /// Unique identifier
    pub id: String,
    /// Domain (e.g., "code", "data", "infra")
    pub domain: String,
    /// Executor pool
    pool: ExecutorPool,
    /// Validators (multiple for scale)
    validators: Vec<Validator>,
    /// Allocation manager
    allocations: AllocationManager,
    /// Allow lending executors
    allow_lending: bool,
    /// Allow borrowing executors
    allow_borrowing: bool,
    /// Config
    _config: SupervisorConfig,
}

impl Supervisor {
    // ========== CRÉATION SIMPLE ==========

    /// Create a new supervisor builder.
    ///
    /// # Example
    ///
    /// ```rust
    /// let sup = Supervisor::new("sup-code")
    ///     .domain("code")
    ///     .executors(4)
    ///     .llm_gemini("gemini-2.0-flash")
    ///     .build();
    /// ```
    pub fn new(id: impl Into<String>) -> SupervisorBuilder {
        SupervisorBuilder::new(id.into())
    }

    /// Quick creation with minimal config.
    ///
    /// # Example
    ///
    /// ```rust
    /// let sup = Supervisor::quick("sup-code", 4);
    /// ```
    pub fn quick(id: impl Into<String>, executors: usize) -> SupervisorBuilder {
        SupervisorBuilder::new(id.into()).executors(executors)
    }

    /// Create from full config.
    pub async fn from_config(config: SupervisorConfig) -> Self {
        let pool = ExecutorPool::new(&config.id, config.executor_count)
            .llm(config.llm.clone())
            .sandbox(config.sandbox.clone())
            .build_and_init()
            .await;

        // Create multiple validators
        let validators: Vec<Validator> = (0..config.validator_count)
            .map(|i| {
                Validator::new(format!("val-{}-{:03}", config.id, i), &config.id)
                    .default_rules()
                    .build()
            })
            .collect();

        let allocations = AllocationManager::new(&config.id)
            .max_lend(config.max_borrowed)
            .max_borrow(config.max_borrowed);

        Self {
            id: config.id.clone(),
            domain: config.domain.clone(),
            pool,
            validators,
            allocations,
            allow_lending: config.allow_lending,
            allow_borrowing: config.allow_borrowing,
            _config: config,
        }
    }

    /// Get a handle for external references.
    pub fn handle(&self) -> SupervisorHandle {
        SupervisorHandle {
            id: self.id.clone(),
            domain: self.domain.clone(),
        }
    }

    // ========== POOL MANAGEMENT ==========

    /// Get an idle executor.
    pub async fn get_executor(&self) -> Option<Arc<Executor>> {
        self.pool.get_idle().await
    }

    /// Get N idle executors.
    pub async fn get_executors(&self, n: usize) -> Vec<Arc<Executor>> {
        self.pool.get_n_idle(n).await
    }

    /// Get pool size.
    pub async fn pool_size(&self) -> usize {
        self.pool.size().await
    }

    /// Get idle count.
    pub async fn idle_count(&self) -> usize {
        self.pool.idle_count().await
    }

    /// Get utilization.
    pub async fn utilization(&self) -> f32 {
        self.pool.utilization().await
    }

    /// Scale up the pool.
    pub async fn scale_up(&self, n: usize) {
        self.pool.scale_up(n).await;
    }

    /// Scale down the pool.
    pub async fn scale_down(&self, n: usize) {
        self.pool.scale_down(n).await;
    }

    // ========== ALLOCATION ==========

    /// Check if can lend executors.
    pub async fn can_lend(&self, count: usize) -> bool {
        if !self.allow_lending {
            return false;
        }
        let idle = self.idle_count().await;
        idle > count && self.allocations.can_lend(count).await
    }

    /// Check if can borrow executors.
    pub async fn can_borrow(&self, count: usize) -> bool {
        if !self.allow_borrowing {
            return false;
        }
        self.allocations.can_borrow(count).await
    }

    /// Lend executors to another supervisor.
    pub async fn lend_executors(
        &self,
        to_supervisor: &str,
        count: usize,
        duration_secs: i64,
    ) -> Option<AllocationGrant> {
        if !self.can_lend(count).await {
            return None;
        }

        let executors = self.pool.get_n_idle(count).await;
        if executors.len() < count {
            return None;
        }

        let executor_ids: Vec<String> = executors.iter().map(|e| e.id.clone()).collect();

        let grant = AllocationGrant::new(&self.id, to_supervisor, executor_ids)
            .duration_secs(duration_secs);

        // Mark executors as borrowed
        for exe in &executors {
            exe.set_borrowed(
                to_supervisor.to_string(),
                grant.lease_id.clone(),
                grant.expires_at,
            )
            .await;
        }

        self.allocations.record_lent(grant.clone()).await;

        Some(grant)
    }

    /// Receive borrowed executors.
    pub async fn receive_borrowed(&self, grant: AllocationGrant) {
        self.allocations.record_borrowed(grant).await;
    }

    /// Return borrowed executors.
    pub async fn return_executors(&self, lease_id: &str) {
        self.allocations.remove_grant(lease_id).await;
    }

    /// Get borrowed executor count.
    pub async fn borrowed_count(&self) -> usize {
        self.allocations.borrowed_count().await
    }

    /// Get lent executor count.
    pub async fn lent_count(&self) -> usize {
        self.allocations.lent_count().await
    }

    /// Effective pool size (own + borrowed - lent).
    pub async fn effective_pool_size(&self) -> usize {
        let own = self.pool_size().await;
        let borrowed = self.borrowed_count().await;
        let lent = self.lent_count().await;
        own + borrowed - lent
    }

    // ========== VALIDATION ==========

    /// Get validator count.
    pub fn validator_count(&self) -> usize {
        self.validators.len()
    }

    /// Get a validator by index (round-robin).
    pub fn get_validator(&self, index: usize) -> Option<&Validator> {
        if self.validators.is_empty() {
            None
        } else {
            Some(&self.validators[index % self.validators.len()])
        }
    }

    /// Validate output using first available validator.
    pub fn validate(&self, output: &str, attempt: u32) -> super::validator::ValidationResult {
        self.validators
            .first()
            .map(|v| v.validate(output, attempt))
            .unwrap_or_else(|| super::validator::ValidationResult {
                passed: true,
                rule_results: vec![],
                score: 100,
                feedback: None,
                action: super::validator::ValidationAction::Accept,
            })
    }

    /// Validate with specific validator.
    pub fn validate_with(
        &self,
        validator_index: usize,
        output: &str,
        attempt: u32,
    ) -> super::validator::ValidationResult {
        self.get_validator(validator_index)
            .map(|v| v.validate(output, attempt))
            .unwrap_or_else(|| self.validate(output, attempt))
    }

    // ========== HELPERS ==========

    /// Create an allocation request.
    pub fn create_help_request(&self, executors_needed: usize) -> AllocationRequest {
        AllocationRequest::new(&self.id, executors_needed)
    }

    /// Get all executor IDs (own pool only).
    pub async fn executor_ids(&self) -> Vec<String> {
        self.pool.executor_ids().await
    }

    /// Cleanup expired allocations.
    pub async fn cleanup_allocations(&self) -> Vec<String> {
        self.allocations.cleanup_expired().await
    }
}

/// Builder for Supervisor.
pub struct SupervisorBuilder {
    id: String,
    domain: String,
    executor_count: usize,
    validator_count: usize,
    llm: Option<LLMConfig>,
    sandbox: Option<SandboxConfig>,
    validation_rules: Vec<ValidationRule>,
    allow_lending: bool,
    allow_borrowing: bool,
    max_borrowed: usize,
    capabilities: HashSet<Capability>,
}

impl SupervisorBuilder {
    fn new(id: String) -> Self {
        Self {
            id,
            domain: "default".into(),
            executor_count: 2,
            validator_count: 1,
            llm: None,
            sandbox: None,
            validation_rules: Vec::new(),
            allow_lending: true,
            allow_borrowing: true,
            max_borrowed: 4,
            capabilities: HashSet::from([Capability::CodeGeneration]),
        }
    }

    /// Set domain.
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = domain.into();
        self
    }

    /// Set number of executors.
    pub fn executors(mut self, count: usize) -> Self {
        self.executor_count = count;
        self
    }

    /// Set number of validators.
    pub fn validators(mut self, count: usize) -> Self {
        self.validator_count = count;
        self
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

    /// Use OpenAI (shortcut).
    pub fn llm_openai(self, model: &str) -> Self {
        self.llm(LLMConfig::openai(model))
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

    /// Use docker sandbox.
    pub fn sandbox_docker(self) -> Self {
        self.sandbox(SandboxConfig::docker())
    }

    /// Add validation rule.
    pub fn validation_rule(mut self, rule: ValidationRule) -> Self {
        self.validation_rules.push(rule);
        self
    }

    /// Allow/disallow lending.
    pub fn allow_lending(mut self, allow: bool) -> Self {
        self.allow_lending = allow;
        self
    }

    /// Allow/disallow borrowing.
    pub fn allow_borrowing(mut self, allow: bool) -> Self {
        self.allow_borrowing = allow;
        self
    }

    /// Set max borrowed executors.
    pub fn max_borrowed(mut self, n: usize) -> Self {
        self.max_borrowed = n;
        self
    }

    /// Add capability.
    pub fn capability(mut self, cap: Capability) -> Self {
        self.capabilities.insert(cap);
        self
    }

    /// Build the supervisor (sync, pool not initialized).
    pub fn build(self) -> SupervisorConfig {
        SupervisorConfig {
            id: self.id,
            domain: self.domain,
            executor_count: self.executor_count,
            validator_count: self.validator_count,
            llm: self
                .llm
                .unwrap_or_else(|| LLMConfig::gemini("gemini-2.0-flash")),
            sandbox: self.sandbox.unwrap_or_else(SandboxConfig::process),
            capabilities: self.capabilities,
            allow_lending: self.allow_lending,
            allow_borrowing: self.allow_borrowing,
            max_borrowed: self.max_borrowed,
            max_queue_depth: 100,
            task_timeout_ms: 60_000,
        }
    }

    /// Build and initialize the supervisor (async).
    pub async fn build_async(self) -> Supervisor {
        let config = self.build();
        Supervisor::from_config(config).await
    }
}

// ========== CRÉATION RAPIDE ==========

/// Create multiple supervisors quickly.
///
/// # Example
///
/// ```rust
/// let supervisors = create_supervisors! {
///     "sup-code" => { domain: "code", executors: 4 },
///     "sup-data" => { domain: "data", executors: 2 },
/// };
/// ```
#[macro_export]
macro_rules! create_supervisor {
    ($id:expr) => {
        Supervisor::new($id).build()
    };
    ($id:expr, $executors:expr) => {
        Supervisor::new($id).executors($executors).build()
    };
    ($id:expr, $domain:expr, $executors:expr) => {
        Supervisor::new($id)
            .domain($domain)
            .executors($executors)
            .build()
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supervisor_builder() {
        let config = Supervisor::new("sup-001")
            .domain("code")
            .executors(4)
            .llm_gemini("gemini-2.0-flash")
            .sandbox_process()
            .allow_lending(true)
            .build();

        assert_eq!(config.id, "sup-001");
        assert_eq!(config.domain, "code");
        assert_eq!(config.executor_count, 4);
    }

    #[test]
    fn test_quick_creation() {
        let config = Supervisor::quick("sup-001", 4).build();
        assert_eq!(config.executor_count, 4);
    }

    #[tokio::test]
    async fn test_supervisor_creation() {
        let sup = Supervisor::new("sup-001")
            .domain("code")
            .executors(2)
            .build_async()
            .await;

        assert_eq!(sup.id, "sup-001");
        assert_eq!(sup.pool_size().await, 2);
    }

    #[tokio::test]
    async fn test_supervisor_lending() {
        let sup1 = Supervisor::new("sup-001")
            .executors(4)
            .allow_lending(true)
            .build_async()
            .await;

        let sup2 = Supervisor::new("sup-002")
            .executors(2)
            .allow_borrowing(true)
            .build_async()
            .await;

        // sup1 lends 2 executors to sup2
        let grant = sup1.lend_executors("sup-002", 2, 60).await;
        assert!(grant.is_some());

        let grant = grant.unwrap();
        assert_eq!(grant.executor_ids.len(), 2);

        // sup2 receives
        sup2.receive_borrowed(grant).await;
        assert_eq!(sup2.borrowed_count().await, 2);
    }
}
