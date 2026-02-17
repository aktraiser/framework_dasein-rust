//! Graph Refactor Example - Python HARDCORE Mode
//!
//! Tests the graph architecture with complex Python async code:
//! - asyncio, dataclasses, typing, pytest
//! - Validation: Uses framework's SandboxPipelineValidator
//!
//! Run with: cargo run --example graph_refactor_python

use async_trait::async_trait;
use dasein_agentic_core::distributed::graph::{
    Executor as GraphExecutor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind,
    ExecutorRegistry, Workflow, WorkflowBuilder, WorkflowConfig,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RefactorArtifact {
    original_code: String,
    refactored_code: String,
    stage: String,
    validation_passed: Option<bool>,
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

struct LLMResources {
    executor: LLMExecutor,
    assembler: CodeAssembler,
}

// ============================================================================
// EXECUTOR 1: CODE ANALYZER (Python)
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
        println!(
            "[Analyzer] Analyzing Python code ({} bytes)...",
            input.len()
        );

        let system = r#"You are a Python code analyzer. Analyze the code structure and identify:
1. All class definitions
2. All dataclasses
3. All async functions
4. All pytest tests

Return a brief summary of what you found and suggest how to better organize the code.
Keep the response concise (max 200 words)."#;

        let prompt = format!("Analyze this Python code:\n\n```python\n{}\n```", input);

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

        ctx.send_message(RefactorArtifact::new(&input)).await?;
        ctx.yield_output("Analysis complete".into()).await?;
        Ok(())
    }
}

// ============================================================================
// EXECUTOR 2: CODE GENERATOR (Python Refactoring)
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
        println!("[Generator] Refactoring Python code...");

        let error_context = if !input.errors.is_empty() {
            format!(
                "\n\n=== PREVIOUS ERRORS - FIX THESE ===\n{}\n",
                input.errors.join("\n")
            )
        } else {
            String::new()
        };

        let system = r#"You are a Python refactoring expert. Reorganize the code with:
1. Imports at the top (stdlib, third-party, local)
2. Constants after imports
3. Dataclasses/Types next
4. Classes with their methods
5. Standalone functions
6. Tests at the bottom with pytest

IMPORTANT:
- Return ONLY valid Python code
- NO markdown, NO explanations
- Keep ALL functionality intact
- Keep ALL tests working
- Use proper type hints"#;

        let prompt = format!(
            "Refactor this Python code into well-organized sections:\n\n```python\n{}\n```{}",
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

        ctx.send_message(input.with_refactored(&code)).await?;
        ctx.yield_output(format!("Generated {} bytes", code.len()))
            .await?;
        Ok(())
    }
}

// ============================================================================
// EXECUTOR 3: CODE VALIDATOR (Python via pytest)
// ============================================================================

struct CodeValidatorExecutor {
    id: ExecutorId,
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
        println!("[Validator] Validating Python code with pytest...");

        // PYTHON validation
        let validator_input = ValidatorInput::new(&input.refactored_code, "python")
            .with_task("Refactored Python code validation");
        let result = self.pipeline.validate(validator_input).await;

        let errors: Vec<String> = result
            .results
            .iter()
            .flat_map(|r| r.errors.clone())
            .collect();

        if result.passed {
            println!("[Validator] ✓ Validation PASSED");
            ctx.send_message(input.with_validation(true, vec![]))
                .await?;
            ctx.yield_output("Validation: PASSED".into()).await?;
        } else {
            println!("[Validator] ✗ Validation FAILED ({} errors)", errors.len());
            for err in errors.iter().take(3) {
                println!("  - {}", err.lines().next().unwrap_or(""));
            }
            ctx.send_message(input.with_validation(false, errors))
                .await?;
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
            "# ============================================================================\n\
             # REFACTORED PYTHON CODE - Validated ✓\n\
             # Original: {} bytes → Refactored: {} bytes\n\
             # ============================================================================\n\n\
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
    println!("  GRAPH REFACTOR DEMO - PYTHON HARDCORE MODE");
    println!("  Async Python with dataclasses, typing, pytest");
    println!("  Framework Features: SharedValidatorPipeline + Graph Retry");
    println!("{}\n", "=".repeat(70));

    // === Setup LLM ===
    let model = std::env::var("MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_string());
    let llm_executor = LLMExecutor::new("refactor-llm", "supervisor")
        .llm_gemini(&model)
        .build();
    let llm_resources = Arc::new(Mutex::new(LLMResources {
        executor: llm_executor,
        assembler: CodeAssembler::new(),
    }));
    println!("✓ LLM: {} (Arc<Mutex>)", model);

    // === Setup Validator Pipeline ===
    let sandbox = ProcessSandbox::new().with_timeout(120_000);
    let sandbox_validator = SandboxPipelineValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/refactor-validation-python"))
        .run_tests(true);
    let pipeline = ValidatorPipeline::new()
        .add(sandbox_validator)
        .into_shared();
    println!("✓ Validator: SharedValidatorPipeline (Python/pytest)");

    // === Build Workflow Graph ===
    println!("\n[1/3] Building workflow...\n");

    let definition = WorkflowBuilder::<RefactorArtifact>::new("refactor-python-pipeline")
        .name("Python Code Refactoring Pipeline")
        .set_start("code-analyzer")
        .add_executor_id("code-generator")
        .add_executor_id("code-validator")
        .add_executor_id("retry-handler")
        .add_executor_id("assembler")
        .add_direct_edge("code-analyzer", "code-generator")
        .add_direct_edge("code-generator", "code-validator")
        .add_conditional_edge(
            "code-validator",
            "assembler",
            |artifact: &RefactorArtifact| artifact.validation_passed == Some(true),
            "on_success",
        )
        .add_conditional_edge(
            "code-validator",
            "retry-handler",
            |artifact: &RefactorArtifact| artifact.validation_passed == Some(false),
            "on_failure",
        )
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

    println!("  ✓ All executors registered");

    // === HARDCORE PYTHON TEST ===
    // Complex async Python with dataclasses, generics, and pytest
    let python_code = r#"
import asyncio
from dataclasses import dataclass, field
from typing import TypeVar, Generic, Optional, Callable, Awaitable, Dict
from datetime import datetime, timedelta
import pytest

T = TypeVar('T')

class CacheError(Exception):
    pass

class KeyNotFoundError(CacheError):
    def __init__(self, key: str):
        self.key = key
        super().__init__(f"Key not found: {key}")

class CacheFullError(CacheError):
    def __init__(self, max_size: int):
        self.max_size = max_size
        super().__init__(f"Cache is full (max: {max_size})")

class TTLExpiredError(CacheError):
    def __init__(self, key: str):
        self.key = key
        super().__init__(f"TTL expired for key: {key}")

@dataclass
class CacheEntry(Generic[T]):
    value: T
    expires_at: Optional[datetime] = None

    def is_expired(self) -> bool:
        if self.expires_at is None:
            return False
        return datetime.now() > self.expires_at

@dataclass
class CacheStats:
    hits: int = 0
    misses: int = 0
    sets: int = 0
    deletes: int = 0

    def hit_rate(self) -> float:
        total = self.hits + self.misses
        return self.hits / total if total > 0 else 0.0

class AsyncCache(Generic[T]):
    def __init__(self, max_size: int = 1000):
        self._data: Dict[str, CacheEntry[T]] = {}
        self._max_size = max_size
        self._lock = asyncio.Lock()
        self._stats = CacheStats()

    async def get(self, key: str) -> Optional[T]:
        async with self._lock:
            entry = self._data.get(key)
            if entry is None:
                self._stats.misses += 1
                return None
            if entry.is_expired():
                del self._data[key]
                self._stats.misses += 1
                raise TTLExpiredError(key)
            self._stats.hits += 1
            return entry.value

    async def set(self, key: str, value: T, ttl: Optional[timedelta] = None) -> None:
        async with self._lock:
            if len(self._data) >= self._max_size and key not in self._data:
                raise CacheFullError(self._max_size)
            expires_at = datetime.now() + ttl if ttl else None
            self._data[key] = CacheEntry(value=value, expires_at=expires_at)
            self._stats.sets += 1

    async def delete(self, key: str) -> bool:
        async with self._lock:
            if key in self._data:
                del self._data[key]
                self._stats.deletes += 1
                return True
            return False

    async def clear(self) -> int:
        async with self._lock:
            count = len(self._data)
            self._data.clear()
            return count

    async def get_or_set(
        self,
        key: str,
        factory: Callable[[], Awaitable[T]],
        ttl: Optional[timedelta] = None
    ) -> T:
        try:
            result = await self.get(key)
            if result is not None:
                return result
        except TTLExpiredError:
            pass
        value = await factory()
        await self.set(key, value, ttl)
        return value

    @property
    def stats(self) -> CacheStats:
        return self._stats

# ============ TESTS ============

@pytest.mark.asyncio
async def test_basic_set_get():
    cache: AsyncCache[str] = AsyncCache()
    await cache.set("key1", "value1")
    result = await cache.get("key1")
    assert result == "value1"

@pytest.mark.asyncio
async def test_cache_miss():
    cache: AsyncCache[int] = AsyncCache()
    result = await cache.get("nonexistent")
    assert result is None

@pytest.mark.asyncio
async def test_cache_delete():
    cache: AsyncCache[str] = AsyncCache()
    await cache.set("key", "value")
    deleted = await cache.delete("key")
    assert deleted is True
    result = await cache.get("key")
    assert result is None

@pytest.mark.asyncio
async def test_cache_full():
    cache: AsyncCache[int] = AsyncCache(max_size=2)
    await cache.set("a", 1)
    await cache.set("b", 2)
    with pytest.raises(CacheFullError):
        await cache.set("c", 3)

@pytest.mark.asyncio
async def test_cache_stats():
    cache: AsyncCache[str] = AsyncCache()
    await cache.set("key", "value")
    await cache.get("key")
    await cache.get("missing")
    assert cache.stats.hits == 1
    assert cache.stats.misses == 1
    assert cache.stats.sets == 1
    assert abs(cache.stats.hit_rate() - 0.5) < 0.01

@pytest.mark.asyncio
async def test_get_or_set():
    cache: AsyncCache[int] = AsyncCache()
    async def compute():
        return 42
    value = await cache.get_or_set("computed", compute)
    assert value == 42
    cached = await cache.get("computed")
    assert cached == 42

@pytest.mark.asyncio
async def test_ttl_expiration():
    cache: AsyncCache[str] = AsyncCache()
    await cache.set("temp", "value", ttl=timedelta(milliseconds=50))
    await asyncio.sleep(0.1)
    with pytest.raises(TTLExpiredError):
        await cache.get("temp")
"#;

    // === Run Workflow ===
    println!("\n[3/3] Running workflow...\n");
    println!("Input: {} bytes\n", python_code.len());
    println!("{}", "-".repeat(70));

    let config = WorkflowConfig::new()
        .with_max_supersteps(25)
        .with_max_retries(8)
        .with_executor_timeout_ms(180_000);

    let workflow = Workflow::with_config(definition, registry, config);

    let start = Instant::now();
    let result = workflow.run(python_code.to_string()).await?;
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
        if final_output.starts_with("#") {
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

    println!("\n{}\n", "=".repeat(70));

    Ok(())
}
