//! Graph Refactor Example - Code Refactoring with Validation
//!
//! Tests the graph architecture with a refactoring task:
//! - Input: Monolithic Rust code
//! - Output: Refactored code (organized sections in single file)
//! - Validation: Uses framework's SandboxPipelineValidator
//!
//! Graph structure:
//! ```
//! Input → Analyzer → Generator → Validator → [Assembler | RetryHandler]
//!                                                  ↑___________|
//! ```
//!
//! Environment variables:
//! - GEMINI_API_KEY: For code generation (required)
//!
//! Run with: cargo run --example graph_refactor

use async_trait::async_trait;
use dasein_agentic_core::distributed::graph::{
    Executor as GraphExecutor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind,
    ExecutorRegistry, InMemoryPersistentBackend, PersistentCheckpointBackend, TaskId, Workflow,
    WorkflowBuilder, WorkflowConfig,
};
use dasein_agentic_core::distributed::{
    CodeAssembler, Executor as LLMExecutor, SandboxPipelineValidator, SharedValidatorPipeline,
    ValidatorInput, ValidatorPipeline,
};
use dasein_agentic_sandbox::ProcessSandbox;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;

// ============================================================================
// MESSAGE TYPES
// ============================================================================

/// Artifact that flows through the workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RefactorArtifact {
    /// Original monolithic code
    original_code: String,
    /// Refactored code (single file, organized sections)
    refactored_code: String,
    /// Current stage
    stage: String,
    /// Validation result
    validation_passed: Option<bool>,
    /// Errors if any
    errors: Vec<String>,
}

impl RefactorArtifact {
    fn new(original_code: impl Into<String>) -> Self {
        Self {
            original_code: original_code.into(),
            refactored_code: String::new(),
            stage: "initial".into(),
            validation_passed: None,
            errors: vec![],
        }
    }

    fn with_refactored(mut self, code: impl Into<String>) -> Self {
        self.refactored_code = code.into();
        self.stage = "refactored".into();
        self
    }

    fn with_validation(mut self, passed: bool, errors: Vec<String>) -> Self {
        self.validation_passed = Some(passed);
        self.errors = errors;
        self.stage = if passed {
            "validated".into()
        } else {
            "failed".into()
        };
        self
    }
}

// ============================================================================
// SHARED RESOURCES
// ============================================================================

/// Resources that need Mutex (LLM executor has internal state)
struct LLMResources {
    executor: LLMExecutor,
    assembler: CodeAssembler,
}

// ValidatorPipeline is shared via SharedValidatorPipeline (Arc) - no Mutex needed

// ============================================================================
// EXECUTOR 1: CODE ANALYZER
// ============================================================================

struct CodeAnalyzerExecutor {
    id: ExecutorId,
    resources: Arc<Mutex<LLMResources>>,
}

impl CodeAnalyzerExecutor {
    fn new(resources: Arc<Mutex<LLMResources>>) -> Self {
        Self {
            id: ExecutorId::new("code-analyzer"),
            resources,
        }
    }
}

#[async_trait]
impl GraphExecutor for CodeAnalyzerExecutor {
    type Input = String;
    type Message = RefactorArtifact;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        println!("[Analyzer] Analyzing code ({} bytes)...", input.len());

        let system = r#"You are a Rust code analyzer. Analyze the code structure and identify:
1. All struct/enum definitions
2. All impl blocks
3. All functions
4. All tests

Return a brief summary of what you found and suggest how to better organize the code.
Keep the response concise (max 200 words)."#;

        let prompt = format!("Analyze this Rust code:\n\n```rust\n{}\n```", input);

        let resources = self.resources.lock().await;
        let result = resources
            .executor
            .execute(system, &prompt)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), format!("LLM error: {}", e)))?;
        drop(resources);

        println!(
            "[Analyzer] Analysis: {}",
            result.content.lines().next().unwrap_or("")
        );

        let artifact = RefactorArtifact::new(&input);
        ctx.send_message(artifact).await?;
        ctx.yield_output("Analysis complete".into()).await?;

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 2: CODE GENERATOR (Refactoring)
// ============================================================================

struct CodeGeneratorExecutor {
    id: ExecutorId,
    resources: Arc<Mutex<LLMResources>>,
}

impl CodeGeneratorExecutor {
    fn new(resources: Arc<Mutex<LLMResources>>) -> Self {
        Self {
            id: ExecutorId::new("code-generator"),
            resources,
        }
    }
}

#[async_trait]
impl GraphExecutor for CodeGeneratorExecutor {
    type Input = RefactorArtifact;
    type Message = RefactorArtifact;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        println!("[Generator] Refactoring code...");

        // Build prompt with error context if this is a retry
        let error_context = if !input.errors.is_empty() {
            format!(
                "\n\n=== PREVIOUS ERRORS - FIX THESE ===\n{}\n",
                input.errors.join("\n")
            )
        } else {
            String::new()
        };

        let system = r#"You are a Rust refactoring expert. Reorganize the code with:
1. Clear section comments (// ===== TYPES ===== etc)
2. Types/structs at the top
3. Implementations in the middle
4. Tests at the bottom in #[cfg(test)] mod tests { }

IMPORTANT:
- Return ONLY valid, compilable Rust code
- NO markdown, NO explanations
- Keep ALL functionality intact
- Keep ALL tests working
- Use proper imports at the top"#;

        let prompt = format!(
            "Refactor this Rust code into well-organized sections:\n\n```rust\n{}\n```{}",
            input.original_code, error_context
        );

        let resources = self.resources.lock().await;
        let result = resources
            .executor
            .execute(system, &prompt)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), format!("LLM error: {}", e)))?;

        let code = resources.assembler.clean_for_validation(&result.content);
        drop(resources);

        println!("[Generator] Generated {} bytes", code.len());

        let artifact = input.with_refactored(&code);
        ctx.send_message(artifact).await?;
        ctx.yield_output(format!("Generated {} bytes", code.len()))
            .await?;

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 3: CODE VALIDATOR
// Uses SharedValidatorPipeline (Arc) - no Mutex needed because validate(&self)
// ============================================================================

struct CodeValidatorExecutor {
    id: ExecutorId,
    /// SharedValidatorPipeline = Arc<ValidatorPipeline>
    /// No Mutex needed - pipeline.validate() takes &self (immutable)
    pipeline: SharedValidatorPipeline,
}

impl CodeValidatorExecutor {
    fn new(pipeline: SharedValidatorPipeline) -> Self {
        Self {
            id: ExecutorId::new("code-validator"),
            pipeline,
        }
    }
}

#[async_trait]
impl GraphExecutor for CodeValidatorExecutor {
    type Input = RefactorArtifact;
    type Message = RefactorArtifact;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Validator
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        println!("[Validator] Validating refactored code...");

        // No lock needed - pipeline is Arc<ValidatorPipeline> and validate takes &self
        let validator_input = ValidatorInput::new(&input.refactored_code, "rust")
            .with_task("Refactored code validation");
        let result = self.pipeline.validate(validator_input).await;

        let errors: Vec<String> = result
            .results
            .iter()
            .flat_map(|r| r.errors.clone())
            .collect();

        if result.passed {
            println!("[Validator] ✓ Validation PASSED");

            let artifact = input.with_validation(true, vec![]);
            ctx.send_message(artifact).await?;
            ctx.yield_output("Validation: PASSED".into()).await?;
        } else {
            println!("[Validator] ✗ Validation FAILED ({} errors)", errors.len());
            for err in errors.iter().take(3) {
                println!("  - {}", err.lines().next().unwrap_or(""));
            }

            let artifact = input.with_validation(false, errors);
            ctx.send_message(artifact).await?;
            ctx.yield_output("Validation: FAILED".into()).await?;
        }

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 4: RETRY HANDLER
// ============================================================================

struct RetryHandlerExecutor {
    id: ExecutorId,
}

impl RetryHandlerExecutor {
    fn new() -> Self {
        Self {
            id: ExecutorId::new("retry-handler"),
        }
    }
}

#[async_trait]
impl GraphExecutor for RetryHandlerExecutor {
    type Input = RefactorArtifact;
    type Message = RefactorArtifact;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        println!(
            "[RetryHandler] Preparing retry with {} errors...",
            input.errors.len()
        );

        // Keep original code and errors for retry
        let mut artifact = RefactorArtifact::new(&input.original_code);
        artifact.errors = input.errors;
        artifact.stage = "retry".into();

        ctx.send_message(artifact).await?;
        ctx.yield_output("Retry prepared".into()).await?;

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 5: ASSEMBLER
// ============================================================================

struct AssemblerExecutor {
    id: ExecutorId,
}

impl AssemblerExecutor {
    fn new() -> Self {
        Self {
            id: ExecutorId::new("assembler"),
        }
    }
}

#[async_trait]
impl GraphExecutor for AssemblerExecutor {
    type Input = RefactorArtifact;
    type Message = RefactorArtifact;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        println!("[Assembler] Assembling final output...");

        let output = format!(
            "// ============================================================================\n\
             // REFACTORED CODE - Validated ✓\n\
             // Original: {} bytes → Refactored: {} bytes\n\
             // ============================================================================\n\n\
             {}",
            input.original_code.len(),
            input.refactored_code.len(),
            input.refactored_code
        );

        ctx.yield_output(output).await?;

        Ok(())
    }
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();

    println!("\n{}", "=".repeat(70));
    println!("  GRAPH REFACTOR DEMO");
    println!("  Monolithic Code → Refactored + Validated");
    println!("  Framework Features: Persistence + SharedValidatorPipeline");
    println!("{}\n", "=".repeat(70));

    // === Setup LLM (needs Mutex - may have internal state) ===
    let model = std::env::var("MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_string());
    let llm_executor = LLMExecutor::new("refactor-llm", "supervisor")
        .llm_gemini(&model)
        .build();
    let llm_resources = Arc::new(Mutex::new(LLMResources {
        executor: llm_executor,
        assembler: CodeAssembler::new(),
    }));
    println!("✓ LLM: {} (Arc<Mutex>)", model);

    // === Setup Validator Pipeline (stateless - Arc only, no Mutex) ===
    let sandbox = ProcessSandbox::new().with_timeout(120_000);
    let sandbox_validator = SandboxPipelineValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/refactor-validation"))
        .run_tests(true);
    // Use framework's into_shared() - returns SharedValidatorPipeline = Arc<ValidatorPipeline>
    let pipeline = ValidatorPipeline::new()
        .add(sandbox_validator)
        .into_shared();
    println!("✓ Validator: SharedValidatorPipeline (Arc, no Mutex)");

    // === Build Workflow Graph ===
    println!("\n[1/3] Building workflow...\n");

    let definition = WorkflowBuilder::<RefactorArtifact>::new("refactor-pipeline")
        .name("Code Refactoring Pipeline")
        .set_start("code-analyzer")
        .add_executor_id("code-generator")
        .add_executor_id("code-validator")
        .add_executor_id("retry-handler")
        .add_executor_id("assembler")
        // Analyzer → Generator
        .add_direct_edge("code-analyzer", "code-generator")
        // Generator → Validator
        .add_direct_edge("code-generator", "code-validator")
        // Validator → Assembler (on success)
        .add_conditional_edge(
            "code-validator",
            "assembler",
            |artifact: &RefactorArtifact| artifact.validation_passed == Some(true),
            "on_success",
        )
        // Validator → RetryHandler (on failure)
        .add_conditional_edge(
            "code-validator",
            "retry-handler",
            |artifact: &RefactorArtifact| artifact.validation_passed == Some(false),
            "on_failure",
        )
        // RetryHandler → Generator (loop)
        .add_direct_edge("retry-handler", "code-generator")
        .build()?;

    println!("  Workflow: {}", definition.id.as_str());
    println!("  Edges: {}", definition.edges.len());

    // === Register Executors ===
    println!("\n[2/3] Registering executors...\n");

    let mut registry: ExecutorRegistry<RefactorArtifact, String> = ExecutorRegistry::new();
    registry.register(CodeAnalyzerExecutor::new(llm_resources.clone()));
    registry.register(CodeGeneratorExecutor::new(llm_resources.clone()));
    registry.register(CodeValidatorExecutor::new(pipeline.clone()));
    registry.register(RetryHandlerExecutor::new());
    registry.register(AssemblerExecutor::new());

    println!("  ✓ code-analyzer (Worker)");
    println!("  ✓ code-generator (Worker)");
    println!("  ✓ code-validator (Validator) - uses framework");
    println!("  ✓ retry-handler (Worker)");
    println!("  ✓ assembler (Worker)");

    // === HARDCORE TEST ===
    // Complex async code with generics, traits, error handling, and async tests
    // This will stress the LLM and likely require multiple retry cycles
    let monolithic_code = r#"
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use thiserror::Error;
use async_trait::async_trait;

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("Key not found: {0}")]
    NotFound(String),
    #[error("Cache is full (max: {0})")]
    Full(usize),
    #[error("TTL expired for key: {0}")]
    Expired(String),
    #[error("Lock poisoned")]
    LockError,
}

#[async_trait]
pub trait CacheBackend<V: Clone + Send + Sync>: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<V>, CacheError>;
    async fn set(&self, key: &str, value: V, ttl: Option<Duration>) -> Result<(), CacheError>;
    async fn delete(&self, key: &str) -> Result<bool, CacheError>;
    async fn clear(&self) -> Result<usize, CacheError>;
}

struct CacheEntry<V> {
    value: V,
    expires_at: Option<std::time::Instant>,
}

impl<V> CacheEntry<V> {
    fn new(value: V, ttl: Option<Duration>) -> Self {
        Self {
            value,
            expires_at: ttl.map(|d| std::time::Instant::now() + d),
        }
    }

    fn is_expired(&self) -> bool {
        self.expires_at.map(|e| e < std::time::Instant::now()).unwrap_or(false)
    }
}

pub struct InMemoryCache<V: Clone + Send + Sync> {
    data: Arc<RwLock<HashMap<String, CacheEntry<V>>>>,
    max_size: usize,
}

impl<V: Clone + Send + Sync> InMemoryCache<V> {
    pub fn new(max_size: usize) -> Self {
        Self {
            data: Arc::new(RwLock::new(HashMap::new())),
            max_size,
        }
    }

    async fn cleanup_expired(&self) {
        let mut data = self.data.write().await;
        data.retain(|_, entry| !entry.is_expired());
    }
}

#[async_trait]
impl<V: Clone + Send + Sync + 'static> CacheBackend<V> for InMemoryCache<V> {
    async fn get(&self, key: &str) -> Result<Option<V>, CacheError> {
        let data = self.data.read().await;
        match data.get(key) {
            Some(entry) if !entry.is_expired() => Ok(Some(entry.value.clone())),
            Some(_) => Err(CacheError::Expired(key.to_string())),
            None => Ok(None),
        }
    }

    async fn set(&self, key: &str, value: V, ttl: Option<Duration>) -> Result<(), CacheError> {
        self.cleanup_expired().await;
        let mut data = self.data.write().await;
        if data.len() >= self.max_size && !data.contains_key(key) {
            return Err(CacheError::Full(self.max_size));
        }
        data.insert(key.to_string(), CacheEntry::new(value, ttl));
        Ok(())
    }

    async fn delete(&self, key: &str) -> Result<bool, CacheError> {
        let mut data = self.data.write().await;
        Ok(data.remove(key).is_some())
    }

    async fn clear(&self) -> Result<usize, CacheError> {
        let mut data = self.data.write().await;
        let count = data.len();
        data.clear();
        Ok(count)
    }
}

pub struct CacheManager<V: Clone + Send + Sync> {
    backend: Arc<dyn CacheBackend<V>>,
    stats: Arc<RwLock<CacheStats>>,
}

#[derive(Default, Clone)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub sets: u64,
    pub deletes: u64,
}

impl CacheStats {
    pub fn hit_rate(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 { 0.0 } else { self.hits as f64 / total as f64 }
    }
}

impl<V: Clone + Send + Sync + 'static> CacheManager<V> {
    pub fn new(backend: Arc<dyn CacheBackend<V>>) -> Self {
        Self {
            backend,
            stats: Arc::new(RwLock::new(CacheStats::default())),
        }
    }

    pub async fn get(&self, key: &str) -> Result<Option<V>, CacheError> {
        let result = self.backend.get(key).await?;
        let mut stats = self.stats.write().await;
        if result.is_some() {
            stats.hits += 1;
        } else {
            stats.misses += 1;
        }
        Ok(result)
    }

    pub async fn set(&self, key: &str, value: V, ttl: Option<Duration>) -> Result<(), CacheError> {
        self.backend.set(key, value, ttl).await?;
        self.stats.write().await.sets += 1;
        Ok(())
    }

    pub async fn get_or_insert<F, Fut>(&self, key: &str, ttl: Option<Duration>, f: F) -> Result<V, CacheError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = V>,
    {
        if let Some(v) = self.get(key).await? {
            return Ok(v);
        }
        let value = f().await;
        self.set(key, value.clone(), ttl).await?;
        Ok(value)
    }

    pub async fn stats(&self) -> CacheStats {
        self.stats.read().await.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_cache_operations() {
        let cache: InMemoryCache<String> = InMemoryCache::new(100);
        cache.set("key1", "value1".to_string(), None).await.unwrap();
        let result = cache.get("key1").await.unwrap();
        assert_eq!(result, Some("value1".to_string()));
    }

    #[tokio::test]
    async fn test_cache_miss() {
        let cache: InMemoryCache<i32> = InMemoryCache::new(100);
        let result = cache.get("nonexistent").await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_cache_delete() {
        let cache: InMemoryCache<String> = InMemoryCache::new(100);
        cache.set("key", "value".to_string(), None).await.unwrap();
        let deleted = cache.delete("key").await.unwrap();
        assert!(deleted);
        let result = cache.get("key").await.unwrap();
        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn test_cache_full() {
        let cache: InMemoryCache<i32> = InMemoryCache::new(2);
        cache.set("a", 1, None).await.unwrap();
        cache.set("b", 2, None).await.unwrap();
        let result = cache.set("c", 3, None).await;
        assert!(matches!(result, Err(CacheError::Full(2))));
    }

    #[tokio::test]
    async fn test_cache_manager_stats() {
        let backend = Arc::new(InMemoryCache::new(100));
        let manager: CacheManager<String> = CacheManager::new(backend);
        manager.set("key", "value".to_string(), None).await.unwrap();
        let _ = manager.get("key").await;
        let _ = manager.get("missing").await;
        let stats = manager.stats().await;
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.sets, 1);
        assert!((stats.hit_rate() - 0.5).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_get_or_insert() {
        let backend = Arc::new(InMemoryCache::new(100));
        let manager: CacheManager<i32> = CacheManager::new(backend);
        let value = manager.get_or_insert("computed", None, || async { 42 }).await.unwrap();
        assert_eq!(value, 42);
        let cached = manager.get("computed").await.unwrap();
        assert_eq!(cached, Some(42));
    }

    #[tokio::test]
    async fn test_ttl_expiration() {
        let cache: InMemoryCache<String> = InMemoryCache::new(100);
        cache.set("temp", "value".to_string(), Some(Duration::from_millis(50))).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let result = cache.get("temp").await;
        assert!(matches!(result, Err(CacheError::Expired(_))));
    }
}
"#;

    // === Run Workflow ===
    println!("\n[3/3] Running workflow...\n");
    println!("Input: {} bytes\n", monolithic_code.len());
    println!("{}", "-".repeat(70));

    // HARDCORE: Allow more supersteps for complex code with multiple retries
    let config = WorkflowConfig::new()
        .with_max_supersteps(25)  // More cycles for complex code
        .with_max_retries(8)      // More retries per executor
        .with_executor_timeout_ms(180_000) // 3 minutes for complex validation
        .with_checkpoint_interval(1); // Checkpoint every superstep

    // === Setup Persistence Backend ===
    let persistent_backend = Arc::new(InMemoryPersistentBackend::new());
    println!("✓ Persistence: InMemoryPersistentBackend (checkpoint every superstep)");

    let workflow = Workflow::with_config(definition, registry, config)
        .with_persistent_backend(persistent_backend.clone());

    // Use a fixed task ID so we can resume if needed
    let task_id = TaskId::new("refactor-hardcore-task");

    let start = Instant::now();
    // Use run_with_resume - will resume from checkpoint if exists, or start fresh
    let result = workflow
        .run_with_resume(task_id.clone(), None, monolithic_code.to_string())
        .await?;
    let duration = start.elapsed();

    println!("{}", "-".repeat(70));

    // === Results ===
    println!("\n{}", "=".repeat(70));
    println!("  RESULT");
    println!("{}", "=".repeat(70));

    println!(
        "\n  Status: {}",
        if result.success {
            "SUCCESS ✓"
        } else {
            "FAILED ✗"
        }
    );
    println!("  Supersteps: {}", result.superstep_count);
    println!("  Duration: {}ms", duration.as_millis());

    if let Some(final_output) = result.outputs.last() {
        if final_output.starts_with("//") {
            println!("\n  --- Final Code (first 30 lines) ---");
            for line in final_output.lines().take(30) {
                println!("  {}", line);
            }
            let total_lines = final_output.lines().count();
            if total_lines > 30 {
                println!("  ... ({} more lines)", total_lines - 30);
            }
        }
    }

    if let Some(error) = &result.error {
        println!("\n  Error: {}", error);
    }

    // === Checkpoint Stats ===
    let checkpoints = persistent_backend
        .list_metadata(&workflow.id().clone(), Some(&task_id))
        .await
        .unwrap_or_default();
    println!("\n  Checkpoints saved: {}", checkpoints.len());
    if !checkpoints.is_empty() {
        println!(
            "  Latest checkpoint: superstep {}",
            checkpoints[0].superstep
        );
    }

    println!("\n{}\n", "=".repeat(70));

    Ok(())
}
