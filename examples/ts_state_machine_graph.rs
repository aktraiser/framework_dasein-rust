//! TypeScript State Machine - Complete Graph Workflow Example
//!
//! Demonstrates the full graph-based multi-agent architecture with:
//! - Fan-out: Parallel generation (Types + Tests independently)
//! - Fan-in: Assembly of all parts
//! - Conditional edges: Success/failure routing
//! - Feedback loops: Retry on validation failure
//! - Superstep execution (BSP/Pregel model)
//!
//! Architecture:
//! ```text
//!                    ┌─────────────┐
//!                    │  TypeGen    │──┐
//!                    │  (Worker)   │  │
//!                    └─────────────┘  │
//!                          │          │
//!         ┌────────────────┼──────────┘
//!         ▼                ▼
//!  ┌─────────────┐  ┌─────────────┐
//!  │  ImplGen    │  │  TestGen    │  (parallel - fan-out)
//!  │  (Worker)   │  │  (Worker)   │
//!  └─────────────┘  └─────────────┘
//!         │                │
//!         └────────┬───────┘
//!                  ▼
//!           ┌─────────────┐
//!           │  Assembler  │  (fan-in)
//!           │  (Worker)   │
//!           └─────────────┘
//!                  │
//!                  ▼
//!           ┌─────────────┐
//!           │ CompileVal  │
//!           │ (Validator) │
//!           └─────────────┘
//!                  │
//!        ┌────────┴────────┐
//!        ▼ (success)       ▼ (failure)
//!  ┌─────────────┐   ┌─────────────┐
//!  │  TestVal    │   │  ImplGen    │  (feedback loop)
//!  │ (Validator) │   │  (retry)    │
//!  └─────────────┘   └─────────────┘
//!        │
//!        ▼ (success)
//!     [Output]
//! ```
//!
//! Usage:
//! ```bash
//! GEMINI_API_KEY=xxx cargo run --example ts_state_machine_graph
//! ```

use async_trait::async_trait;
use dasein_agentic_core::distributed::graph::{
    Executor as GraphExecutor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind,
    ExecutorRegistry, Workflow, WorkflowBuilder, WorkflowConfig,
};
use dasein_agentic_core::distributed::{
    CodeAssembler, Executor as LLMExecutor, SandboxPipelineValidator, ValidatorInput,
    ValidatorPipeline,
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

/// Code artifact that flows through the workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodeArtifact {
    /// The generated/validated code
    code: String,
    /// Programming language
    language: String,
    /// Current stage
    stage: ArtifactStage,
    /// Validation result if validated
    validation_passed: Option<bool>,
    /// Error feedback for retry
    errors: Vec<String>,
    /// Original task description
    task: String,
    /// Part name for assembly
    part_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
enum ArtifactStage {
    Initial,
    TypesGenerated,
    ImplGenerated,
    TestsGenerated,
    Assembled,
    CompileValidated,
    TestValidated,
    RetryNeeded,
}

impl CodeArtifact {
    fn new(task: impl Into<String>) -> Self {
        Self {
            code: String::new(),
            language: "typescript".into(),
            stage: ArtifactStage::Initial,
            validation_passed: None,
            errors: vec![],
            task: task.into(),
            part_name: None,
        }
    }

    fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = code.into();
        self
    }

    fn with_stage(mut self, stage: ArtifactStage) -> Self {
        self.stage = stage;
        self
    }

    fn with_part_name(mut self, name: impl Into<String>) -> Self {
        self.part_name = Some(name.into());
        self
    }
}

// ============================================================================
// SHARED RESOURCES
// ============================================================================

struct SharedResources {
    llm: LLMExecutor,
    pipeline: ValidatorPipeline,
    assembler: CodeAssembler,
}

// ============================================================================
// EXECUTOR 0: TASK SPLITTER (Worker) - Initial fan-out
// ============================================================================

/// Splits the initial task to multiple generators (fan-out entry point).
struct TaskSplitterExecutor {
    id: ExecutorId,
}

impl TaskSplitterExecutor {
    fn new() -> Self {
        Self {
            id: ExecutorId::new("task-splitter"),
        }
    }
}

#[async_trait]
impl GraphExecutor for TaskSplitterExecutor {
    type Input = String;
    type Message = CodeArtifact;
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
        println!("[TaskSplitter] Distributing task to generators...");

        // Create artifact with task for downstream generators
        let artifact = CodeArtifact::new(&input).with_stage(ArtifactStage::Initial);

        // Send to all connected executors (fan-out via edges)
        ctx.send_message(artifact).await?;
        ctx.yield_output("Task distributed to generators".into())
            .await?;

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 1: TYPE GENERATOR (Worker)
// ============================================================================

/// Generates TypeScript types and interfaces.
struct TypeGeneratorExecutor {
    id: ExecutorId,
    resources: Arc<Mutex<SharedResources>>,
}

impl TypeGeneratorExecutor {
    fn new(resources: Arc<Mutex<SharedResources>>) -> Self {
        Self {
            id: ExecutorId::new("type-generator"),
            resources,
        }
    }
}

#[async_trait]
impl GraphExecutor for TypeGeneratorExecutor {
    type Input = CodeArtifact;
    type Message = CodeArtifact;
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
        println!("[TypeGen] Generating TypeScript types and interfaces...");

        let system = r#"You are an expert TypeScript developer.
Generate ONLY the type definitions and interfaces for the given task.
Do NOT include any implementation code or tests.
Output must be valid TypeScript that compiles with `tsc --noEmit`.
No markdown, no explanations, just TypeScript code."#;

        let prompt = format!(
            "Generate TypeScript types and interfaces for:\n\n{}",
            input.task
        );

        let resources = self.resources.lock().await;
        let result = resources
            .llm
            .execute(system, &prompt)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), format!("LLM error: {}", e)))?;

        let code = resources.assembler.clean_for_validation(&result.content);
        drop(resources);

        println!("[TypeGen] Generated {} bytes of types", code.len());

        // Send to ImplGen and Assembler (fan-out handled by edges)
        let artifact = CodeArtifact::new(&input.task)
            .with_code(&code)
            .with_stage(ArtifactStage::TypesGenerated)
            .with_part_name("types");

        ctx.send_message(artifact).await?;
        ctx.yield_output(format!("Types: {} bytes", code.len()))
            .await?;

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 2: IMPLEMENTATION GENERATOR (Worker)
// ============================================================================

/// Generates TypeScript implementation based on types.
struct ImplGeneratorExecutor {
    id: ExecutorId,
    resources: Arc<Mutex<SharedResources>>,
}

impl ImplGeneratorExecutor {
    fn new(resources: Arc<Mutex<SharedResources>>) -> Self {
        Self {
            id: ExecutorId::new("impl-generator"),
            resources,
        }
    }
}

#[async_trait]
impl GraphExecutor for ImplGeneratorExecutor {
    type Input = CodeArtifact;
    type Message = CodeArtifact;
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
        let is_retry = input.stage == ArtifactStage::RetryNeeded;
        println!(
            "[ImplGen] Generating implementation{}...",
            if is_retry { " (RETRY)" } else { "" }
        );

        let system = r#"You are an expert TypeScript developer.
Generate the implementation code for the given types.
Use async/await throughout. Use proper TypeScript generics.
Do NOT include the type definitions (they are separate).
Do NOT include tests.
Output must be valid TypeScript.
No markdown, no explanations, just TypeScript code."#;

        let prompt = if is_retry && !input.errors.is_empty() {
            format!(
                "=== ORIGINAL TASK ===\n{}\n\n=== TYPES (given) ===\n```typescript\n{}\n```\n\n=== ERRORS TO FIX ===\n{}\n\nFix the errors and return ONLY the implementation code.",
                input.task,
                input.code,
                input.errors.join("\n")
            )
        } else {
            format!(
                "Task: {}\n\nTypes to implement:\n```typescript\n{}\n```\n\nGenerate the implementation.",
                input.task, input.code
            )
        };

        let resources = self.resources.lock().await;
        let result = resources
            .llm
            .execute(system, &prompt)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), format!("LLM error: {}", e)))?;

        let code = resources.assembler.clean_for_validation(&result.content);
        drop(resources);

        println!("[ImplGen] Generated {} bytes of implementation", code.len());

        let artifact = CodeArtifact::new(&input.task)
            .with_code(&code)
            .with_stage(ArtifactStage::ImplGenerated)
            .with_part_name("impl");

        ctx.send_message(artifact).await?;
        ctx.yield_output(format!("Impl: {} bytes", code.len()))
            .await?;

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 3: TEST GENERATOR (Worker) - INDEPENDENT
// ============================================================================

/// Generates Jest tests independently (TDD pattern).
struct TestGeneratorExecutor {
    id: ExecutorId,
    resources: Arc<Mutex<SharedResources>>,
}

impl TestGeneratorExecutor {
    fn new(resources: Arc<Mutex<SharedResources>>) -> Self {
        Self {
            id: ExecutorId::new("test-generator"),
            resources,
        }
    }
}

#[async_trait]
impl GraphExecutor for TestGeneratorExecutor {
    type Input = CodeArtifact;
    type Message = CodeArtifact;
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
        println!("[TestGen] Generating Jest tests (independent)...");

        let system = r#"You are an expert TypeScript developer writing Jest tests.
Generate comprehensive Jest tests for the given specification.
Tests should cover: basic transitions, guards, async actions, middleware, edge cases.
Use describe/it blocks. Use async/await.
Do NOT include any implementation - just the tests.
Assume the implementation exports from './state-machine'.
No markdown, no explanations, just TypeScript test code."#;

        let prompt = format!(
            "Generate Jest tests for a TypeScript state machine with this specification:\n\n{}",
            input.task
        );

        let resources = self.resources.lock().await;
        let result = resources
            .llm
            .execute(system, &prompt)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), format!("LLM error: {}", e)))?;

        let code = resources.assembler.clean_for_validation(&result.content);
        drop(resources);

        println!("[TestGen] Generated {} bytes of tests", code.len());

        let artifact = CodeArtifact::new(&input.task)
            .with_code(&code)
            .with_stage(ArtifactStage::TestsGenerated)
            .with_part_name("tests");

        ctx.send_message(artifact).await?;
        ctx.yield_output(format!("Tests: {} bytes", code.len()))
            .await?;

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 4: CODE ASSEMBLER (Worker) - FAN-IN
// ============================================================================

/// Assembles types + implementation + tests into a single file.
struct AssemblerExecutor {
    id: ExecutorId,
    /// Collected parts waiting for assembly
    parts: Arc<Mutex<Vec<CodeArtifact>>>,
}

impl AssemblerExecutor {
    fn new() -> Self {
        Self {
            id: ExecutorId::new("assembler"),
            parts: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl GraphExecutor for AssemblerExecutor {
    type Input = CodeArtifact;
    type Message = CodeArtifact;
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
        let part_name = input.part_name.clone().unwrap_or_else(|| "unknown".into());
        println!("[Assembler] Received part: {}", part_name);

        let mut parts = self.parts.lock().await;
        parts.push(input.clone());

        // Need 3 parts: types, impl, tests
        if parts.len() < 3 {
            println!("[Assembler] Waiting for more parts ({}/3)...", parts.len());
            return Ok(());
        }

        println!("[Assembler] All parts received, assembling...");

        // Find each part
        let types_code = parts
            .iter()
            .find(|p| p.part_name.as_deref() == Some("types"))
            .map(|p| p.code.clone())
            .unwrap_or_default();

        let impl_code = parts
            .iter()
            .find(|p| p.part_name.as_deref() == Some("impl"))
            .map(|p| p.code.clone())
            .unwrap_or_default();

        let test_code = parts
            .iter()
            .find(|p| p.part_name.as_deref() == Some("tests"))
            .map(|p| p.code.clone())
            .unwrap_or_default();

        // Assemble with clear sections
        let assembled = format!(
            r#"// ============================================================================
// TYPES & INTERFACES
// ============================================================================

{}

// ============================================================================
// IMPLEMENTATION
// ============================================================================

{}

// ============================================================================
// TESTS
// ============================================================================

{}
"#,
            types_code, impl_code, test_code
        );

        println!("[Assembler] Assembled {} bytes total", assembled.len());

        // Clear parts for potential retry
        parts.clear();
        drop(parts);

        let artifact = CodeArtifact::new(&input.task)
            .with_code(&assembled)
            .with_stage(ArtifactStage::Assembled);

        ctx.send_message(artifact).await?;
        ctx.yield_output(format!("Assembled: {} bytes", assembled.len()))
            .await?;

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 5: COMPILE VALIDATOR (Validator)
// ============================================================================

/// Validates TypeScript compilation.
struct CompileValidatorExecutor {
    id: ExecutorId,
    resources: Arc<Mutex<SharedResources>>,
}

impl CompileValidatorExecutor {
    fn new(resources: Arc<Mutex<SharedResources>>) -> Self {
        Self {
            id: ExecutorId::new("compile-validator"),
            resources,
        }
    }
}

#[async_trait]
impl GraphExecutor for CompileValidatorExecutor {
    type Input = CodeArtifact;
    type Message = CodeArtifact;
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
        println!("[CompileVal] Validating TypeScript compilation...");

        let resources = self.resources.lock().await;
        let validator_input = ValidatorInput::new(&input.code, "typescript").with_task(&input.task);
        let result = resources.pipeline.validate(validator_input).await;
        drop(resources);

        let errors: Vec<String> = result
            .results
            .iter()
            .flat_map(|r| r.errors.clone())
            .collect();

        if result.passed {
            println!("[CompileVal] ✓ Compilation PASSED");

            let artifact = CodeArtifact {
                validation_passed: Some(true),
                stage: ArtifactStage::CompileValidated,
                ..input
            };

            ctx.send_message(artifact).await?;
            ctx.yield_output("Compile: PASSED".into()).await?;
        } else {
            println!(
                "[CompileVal] ✗ Compilation FAILED ({} errors)",
                errors.len()
            );
            for err in errors.iter().take(3) {
                println!("  - {}", err.lines().next().unwrap_or(""));
            }

            let artifact = CodeArtifact {
                validation_passed: Some(false),
                stage: ArtifactStage::RetryNeeded,
                errors,
                ..input
            };

            ctx.send_message(artifact).await?;
            ctx.yield_output("Compile: FAILED".into()).await?;
        }

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 6: TEST VALIDATOR (Validator)
// ============================================================================

/// Runs Jest tests.
struct TestValidatorExecutor {
    id: ExecutorId,
    resources: Arc<Mutex<SharedResources>>,
}

impl TestValidatorExecutor {
    fn new(resources: Arc<Mutex<SharedResources>>) -> Self {
        Self {
            id: ExecutorId::new("test-validator"),
            resources,
        }
    }
}

#[async_trait]
impl GraphExecutor for TestValidatorExecutor {
    type Input = CodeArtifact;
    type Message = CodeArtifact;
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
        println!("[TestVal] Running Jest tests...");

        let resources = self.resources.lock().await;
        let validator_input = ValidatorInput::new(&input.code, "typescript").with_task(&input.task);
        let result = resources.pipeline.validate(validator_input).await;
        drop(resources);

        let errors: Vec<String> = result
            .results
            .iter()
            .flat_map(|r| r.errors.clone())
            .collect();

        if result.passed {
            println!("[TestVal] ✓ All tests PASSED");

            let artifact = CodeArtifact {
                validation_passed: Some(true),
                stage: ArtifactStage::TestValidated,
                ..input
            };

            ctx.send_message(artifact).await?;
            ctx.yield_output("Tests: PASSED".into()).await?;
        } else {
            println!("[TestVal] ✗ Tests FAILED ({} errors)", errors.len());
            for err in errors.iter().take(3) {
                println!("  - {}", err.lines().next().unwrap_or(""));
            }

            let artifact = CodeArtifact {
                validation_passed: Some(false),
                stage: ArtifactStage::RetryNeeded,
                errors,
                ..input
            };

            ctx.send_message(artifact).await?;
            ctx.yield_output("Tests: FAILED".into()).await?;
        }

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
    println!("  TYPESCRIPT STATE MACHINE - Graph Workflow");
    println!("  Fan-out (Types||Tests) → Fan-in (Assembler) → Validators");
    println!("  With Feedback Loops for Retry on Failure");
    println!("{}\n", "=".repeat(70));

    // === Task: Event-driven state machine ===
    let task = r#"Create a TypeScript event-driven state machine library with:

1. StateMachine<S, E> generic class:
   - constructor(initialState: S, transitions: TransitionMap<S, E>)
   - send(event: E): Promise<S> - async transition with middleware
   - getState(): S
   - subscribe(listener: (state: S, event: E) => void): () => void
   - canTransition(event: E): boolean

2. TransitionMap type:
   - Map of state -> event -> { target: state, guard?: () => boolean, action?: () => Promise<void> }

3. Middleware support:
   - before(hook: (state: S, event: E) => Promise<boolean>): void
   - after(hook: (prevState: S, newState: S, event: E) => Promise<void>): void

4. Built-in features:
   - History tracking (last N states)
   - Timeout transitions (auto-transition after delay)

Requirements:
- Use async/await throughout
- Proper TypeScript generics with constraints
- Export all types"#;

    // === Setup LLM ===
    let model = std::env::var("MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_string());
    let llm = LLMExecutor::new("ts-graph-llm", "supervisor")
        .llm_gemini(&model)
        .build();
    println!("✓ LLM: {}", model);

    // === Setup Validator Pipeline ===
    let sandbox = ProcessSandbox::new().with_timeout(120_000);
    let validator = SandboxPipelineValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/ts-state-machine-validation"))
        .run_tests(true);
    let pipeline = ValidatorPipeline::new().add(validator);
    println!("✓ Validator Pipeline ready");

    // === Shared Resources ===
    let resources = Arc::new(Mutex::new(SharedResources {
        llm,
        pipeline,
        assembler: CodeAssembler::new(),
    }));

    // === Build Workflow Graph ===
    println!("\n[1/3] Building workflow graph...\n");

    let definition = WorkflowBuilder::<CodeArtifact>::new("ts-state-machine-pipeline")
        .name("TypeScript State Machine Generator")
        .set_start("task-splitter")
        .add_executor_id("type-generator")
        .add_executor_id("impl-generator")
        .add_executor_id("test-generator")
        .add_executor_id("assembler")
        .add_executor_id("compile-validator")
        .add_executor_id("test-validator")
        // TaskSplitter → TypeGen AND TestGen (fan-out: parallel generation)
        .add_direct_edge("task-splitter", "type-generator")
        .add_direct_edge("task-splitter", "test-generator")
        // TypeGen → ImplGen (impl needs types)
        .add_direct_edge("type-generator", "impl-generator")
        // TypeGen → Assembler (types go to assembly)
        .add_direct_edge("type-generator", "assembler")
        // TestGen → Assembler (tests go to assembly)
        .add_direct_edge("test-generator", "assembler")
        // ImplGen → Assembler
        .add_direct_edge("impl-generator", "assembler")
        // Assembler → CompileValidator
        .add_direct_edge("assembler", "compile-validator")
        // CompileValidator → TestValidator (on success)
        .add_conditional_edge(
            "compile-validator",
            "test-validator",
            |a: &CodeArtifact| a.validation_passed == Some(true),
            "on_compile_success",
        )
        // CompileValidator → ImplGen (on failure - feedback loop)
        .add_conditional_edge(
            "compile-validator",
            "impl-generator",
            |a: &CodeArtifact| a.validation_passed == Some(false),
            "on_compile_failure",
        )
        // TestValidator → ImplGen (on failure - feedback loop)
        .add_conditional_edge(
            "test-validator",
            "impl-generator",
            |a: &CodeArtifact| a.validation_passed == Some(false),
            "on_test_failure",
        )
        .build()?;

    println!("  Workflow: {}", definition.id.as_str());
    println!("  Start: {}", definition.start.as_str());
    println!(
        "  Executors: {:?}",
        definition
            .executors
            .iter()
            .map(|e| e.as_str())
            .collect::<Vec<_>>()
    );
    println!("  Edges: {}", definition.edges.len());
    for edge in definition.edges.all() {
        let source = match &edge.source {
            dasein_agentic_core::distributed::graph::EdgeSource::Single(id) => {
                id.as_str().to_string()
            }
            dasein_agentic_core::distributed::graph::EdgeSource::Multiple(ids) => {
                format!(
                    "[{}]",
                    ids.iter()
                        .map(|id| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        };
        let target = match &edge.target {
            dasein_agentic_core::distributed::graph::EdgeTarget::Single(id) => {
                id.as_str().to_string()
            }
            dasein_agentic_core::distributed::graph::EdgeTarget::Multiple(ids) => {
                format!(
                    "[{}]",
                    ids.iter()
                        .map(|id| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        };
        println!("    {} → {} ({:?})", source, target, edge.kind);
    }

    // === Register Executors ===
    println!("\n[2/3] Registering executors...\n");

    let mut registry: ExecutorRegistry<CodeArtifact, String> = ExecutorRegistry::new();
    registry.register(TaskSplitterExecutor::new());
    registry.register(TypeGeneratorExecutor::new(resources.clone()));
    registry.register(ImplGeneratorExecutor::new(resources.clone()));
    registry.register(TestGeneratorExecutor::new(resources.clone()));
    registry.register(AssemblerExecutor::new());
    registry.register(CompileValidatorExecutor::new(resources.clone()));
    registry.register(TestValidatorExecutor::new(resources.clone()));

    println!("  ✓ task-splitter (Worker) [Fan-out entry]");
    println!("  ✓ type-generator (Worker)");
    println!("  ✓ impl-generator (Worker)");
    println!("  ✓ test-generator (Worker) [Independent/TDD]");
    println!("  ✓ assembler (Worker) [Fan-in]");
    println!("  ✓ compile-validator (Validator)");
    println!("  ✓ test-validator (Validator)");

    // === Run Workflow ===
    println!("\n[3/3] Running workflow...\n");
    println!("Task: TypeScript State Machine Library\n");
    println!("{}", "-".repeat(70));

    let config = WorkflowConfig::new()
        .with_max_supersteps(20) // Allow for retry cycles
        .with_max_retries(5);

    let workflow = Workflow::with_config(definition, registry, config);

    let start = Instant::now();
    let result = workflow.run(task.to_string()).await?;
    let duration = start.elapsed();

    println!("{}", "-".repeat(70));

    // === Results ===
    println!("\n{}", "=".repeat(70));
    println!("  WORKFLOW RESULT");
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
    println!("  Outputs: {}", result.outputs.len());

    if !result.outputs.is_empty() {
        println!("\n  --- Execution Log ---");
        for output in &result.outputs {
            if !output.starts_with("//") && output.len() < 100 {
                println!("  • {}", output);
            }
        }
    }

    // Find final code in outputs
    let final_code = result
        .outputs
        .iter()
        .find(|o| o.contains("TYPES & INTERFACES"));

    if let Some(code) = final_code {
        println!("\n  --- Final Code Preview ---");
        let lines: Vec<&str> = code.lines().collect();
        for line in lines.iter().take(30) {
            println!("    {}", line);
        }
        if lines.len() > 30 {
            println!("    ... ({} more lines)", lines.len() - 30);
        }
        println!("\n  Total size: {} bytes", code.len());
    }

    if let Some(error) = &result.error {
        println!("\n  Error: {}", error);
    }

    println!("\n{}\n", "=".repeat(70));

    Ok(())
}
