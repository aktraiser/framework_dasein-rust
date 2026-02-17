//! Graph Workflow Example - Full Pipeline with Superstep Execution
//!
//! Demonstrates the graph-based multi-agent architecture with real LLM:
//! - Executor trait with Input/Message/Output types
//! - WorkflowContext for inter-executor communication
//! - Edge types: Direct, Conditional
//! - Superstep execution (BSP/Pregel model)
//! - Integration with existing Executor (LLM) and ValidatorPipeline
//!
//! Environment variables:
//! - GEMINI_API_KEY: For code generation (required)
//! - CONTEXT7_API_KEY: Enable MCP Doc Validator (optional)
//!
//! Run with: cargo run --example graph_workflow

use async_trait::async_trait;
use dasein_agentic_core::distributed::graph::{
    Executor as GraphExecutor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind,
    ExecutorRegistry, Workflow, WorkflowBuilder, WorkflowConfig,
};
use dasein_agentic_core::distributed::{
    CodeAssembler, Executor as LLMExecutor, MCPDocConfig, MCPDocValidator,
    SandboxPipelineValidator, ValidatorInput, ValidatorPipeline,
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

/// Message that flows through the workflow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodeArtifact {
    /// The generated/validated code
    code: String,
    /// Programming language
    language: String,
    /// Current stage: "generated", "validated", "assembled"
    stage: String,
    /// Validation result if validated
    validation_passed: Option<bool>,
    /// Error feedback for retry
    errors: Vec<String>,
    /// Original task description
    task: String,
}

impl CodeArtifact {
    fn new(code: impl Into<String>, task: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            language: "rust".into(),
            stage: "initial".into(),
            validation_passed: None,
            errors: vec![],
            task: task.into(),
        }
    }

    fn with_stage(mut self, stage: impl Into<String>) -> Self {
        self.stage = stage.into();
        self
    }

    #[allow(dead_code)]
    fn with_validation(mut self, passed: bool, errors: Vec<String>) -> Self {
        self.validation_passed = Some(passed);
        self.errors = errors;
        self
    }
}

// ============================================================================
// SHARED STATE: LLM Executor + Pipeline
// ============================================================================

/// Shared resources for executors (LLM, validators, etc.)
struct SharedResources {
    llm_executor: LLMExecutor,
    pipeline: ValidatorPipeline,
    assembler: CodeAssembler,
    #[allow(dead_code)]
    max_retries: u32,
}

// ============================================================================
// EXECUTOR 1: CODE GENERATOR (Worker)
// ============================================================================

/// Generates Rust code using LLM from task description.
struct CodeGeneratorExecutor {
    id: ExecutorId,
    resources: Arc<Mutex<SharedResources>>,
}

impl CodeGeneratorExecutor {
    fn new(resources: Arc<Mutex<SharedResources>>) -> Self {
        Self {
            id: ExecutorId::new("code-generator"),
            resources,
        }
    }
}

#[async_trait]
impl GraphExecutor for CodeGeneratorExecutor {
    type Input = String; // Task description or retry prompt
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
        println!("[CodeGenerator] Processing input ({} chars)", input.len());

        let system = r#"You are an expert Rust developer.
Write clean, idiomatic async Rust code.
Use tokio for async runtime.
Include tests in a #[cfg(test)] module.
Return ONLY compilable Rust code, no markdown."#;

        let resources = self.resources.lock().await;

        // Generate code using LLM
        let result = resources
            .llm_executor
            .execute(system, &input)
            .await
            .map_err(|e| ExecutorError::new(self.id.clone(), format!("LLM error: {}", e)))?;

        let code = resources.assembler.clean_for_validation(&result.content);
        drop(resources);

        println!("[CodeGenerator] Generated {} bytes of code", code.len());

        // Create artifact and send to validator
        let artifact = CodeArtifact::new(&code, &input).with_stage("generated");

        ctx.send_message(artifact).await?;
        ctx.yield_output(format!("Generated: {} bytes", code.len()))
            .await?;

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 2: CODE VALIDATOR (Validator)
// ============================================================================

/// Validates code using SandboxValidator + optional MCP Doc.
struct CodeValidatorExecutor {
    id: ExecutorId,
    resources: Arc<Mutex<SharedResources>>,
}

impl CodeValidatorExecutor {
    fn new(resources: Arc<Mutex<SharedResources>>) -> Self {
        Self {
            id: ExecutorId::new("code-validator"),
            resources,
        }
    }
}

#[async_trait]
impl GraphExecutor for CodeValidatorExecutor {
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
        println!("[Validator] Validating {} code...", input.language);

        let resources = self.resources.lock().await;

        // Validate with pipeline
        let validator_input =
            ValidatorInput::new(&input.code, &input.language).with_task(&input.task);
        let result = resources.pipeline.validate(validator_input).await;

        drop(resources);

        let errors: Vec<String> = result
            .results
            .iter()
            .flat_map(|r| r.errors.clone())
            .collect();

        if result.passed {
            println!("[Validator] ✓ Validation PASSED");

            let artifact = CodeArtifact {
                code: input.code,
                language: input.language,
                stage: "validated".into(),
                validation_passed: Some(true),
                errors: vec![],
                task: input.task,
            };

            ctx.send_message(artifact).await?;
            ctx.yield_output("Validation: PASSED".into()).await?;
        } else {
            println!("[Validator] ✗ Validation FAILED ({} errors)", errors.len());
            for err in errors.iter().take(3) {
                println!("  - {}", err.lines().next().unwrap_or(""));
            }

            let artifact = CodeArtifact {
                code: input.code,
                language: input.language,
                stage: "validation_failed".into(),
                validation_passed: Some(false),
                errors: errors.clone(),
                task: input.task,
            };

            ctx.send_message(artifact).await?;
            ctx.yield_output(format!("Validation: FAILED ({} errors)", errors.len()))
                .await?;
        }

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 3: RETRY HANDLER (Worker)
// ============================================================================

/// Handles validation failures by building retry prompts.
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
        println!("[RetryHandler] Building retry prompt...");

        // Build retry prompt with error feedback
        let retry_prompt = format!(
            r#"=== ORIGINAL TASK ===
{}

=== YOUR PREVIOUS CODE ===
```rust
{}
```

=== VALIDATION ERRORS ===
{}

Fix the errors above and return the complete corrected Rust code.
Return ONLY valid Rust code, no markdown."#,
            input.task,
            input.code,
            input.errors.join("\n\n")
        );

        // Create new artifact with retry prompt as task
        let artifact = CodeArtifact::new("", &retry_prompt).with_stage("retry");

        ctx.send_message(artifact).await?;
        ctx.yield_output("Retry prompt built".into()).await?;

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 4: CODE ASSEMBLER (Worker)
// ============================================================================

/// Assembles final validated code.
struct CodeAssemblerExecutor {
    id: ExecutorId,
}

impl CodeAssemblerExecutor {
    fn new() -> Self {
        Self {
            id: ExecutorId::new("code-assembler"),
        }
    }
}

#[async_trait]
impl GraphExecutor for CodeAssemblerExecutor {
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
        println!("[Assembler] Assembling final code...");

        let final_code = format!(
            "// === VALIDATED CODE ===\n// Task: {}\n// Status: {}\n\n{}",
            input.task.lines().next().unwrap_or(""),
            if input.validation_passed.unwrap_or(false) {
                "PASSED"
            } else {
                "UNKNOWN"
            },
            input.code
        );

        println!("[Assembler] Final: {} bytes", final_code.len());

        // Yield the final assembled code as output
        ctx.yield_output(final_code).await?;

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
    println!("  GRAPH WORKFLOW DEMO");
    println!("  Generator → Validator → [Assembler | RetryHandler]");
    println!("  Using Superstep Execution (BSP/Pregel model)");
    println!("{}\n", "=".repeat(70));

    // === Setup LLM Executor ===
    let model = std::env::var("MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_string());
    let llm_executor = LLMExecutor::new("graph-llm", "supervisor")
        .llm_gemini(&model)
        .build();
    println!("✓ LLM: {}", model);

    // === Setup Validator Pipeline ===
    let sandbox = ProcessSandbox::new().with_timeout(120_000);
    let sandbox_validator = SandboxPipelineValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/graph-validation"))
        .run_tests(true);

    let pipeline = if let Ok(context7_key) = std::env::var("CONTEXT7_API_KEY") {
        println!("✓ MCP Doc Validator enabled");
        let mcp_config = MCPDocConfig::context7(&context7_key);
        let mcp_validator = MCPDocValidator::new(mcp_config);
        ValidatorPipeline::new()
            .add(sandbox_validator)
            .add(mcp_validator)
    } else {
        println!("⚠ MCP Doc Validator disabled (set CONTEXT7_API_KEY)");
        ValidatorPipeline::new().add(sandbox_validator)
    };

    let assembler = CodeAssembler::new();

    // === Shared Resources ===
    let resources = Arc::new(Mutex::new(SharedResources {
        llm_executor,
        pipeline,
        assembler,
        max_retries: 5,
    }));

    // === Build Workflow Graph ===
    println!("\n[1/3] Building workflow graph...\n");

    let definition = WorkflowBuilder::<CodeArtifact>::new("code-pipeline")
        .name("Code Generation Pipeline with Validation")
        .set_start("code-generator")
        .add_executor_id("code-validator")
        .add_executor_id("code-assembler")
        .add_executor_id("retry-handler")
        // Generator → Validator (always)
        .add_direct_edge("code-generator", "code-validator")
        // Validator → Assembler (if validation passed)
        .add_conditional_edge(
            "code-validator",
            "code-assembler",
            |artifact: &CodeArtifact| artifact.validation_passed == Some(true),
            "on_success",
        )
        // Validator → RetryHandler (if validation failed)
        .add_conditional_edge(
            "code-validator",
            "retry-handler",
            |artifact: &CodeArtifact| artifact.validation_passed == Some(false),
            "on_failure",
        )
        // RetryHandler → Generator (feedback loop)
        .add_direct_edge("retry-handler", "code-generator")
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

    // === Register Executors ===
    println!("\n[2/3] Registering executors...\n");

    let mut registry: ExecutorRegistry<CodeArtifact, String> = ExecutorRegistry::new();
    registry.register(CodeGeneratorExecutor::new(resources.clone()));
    registry.register(CodeValidatorExecutor::new(resources.clone()));
    registry.register(CodeAssemblerExecutor::new());
    registry.register(RetryHandlerExecutor::new());

    println!("  ✓ code-generator (Worker)");
    println!("  ✓ code-validator (Validator)");
    println!("  ✓ code-assembler (Worker)");
    println!("  ✓ retry-handler (Worker)");

    // === Define Task ===
    let task = r#"Write a Rust function that:
1. Takes a Vec<i32> as input
2. Returns the sum of all even numbers squared
3. Uses iterator methods (filter, map, sum)
4. Include a test that verifies sum_even_squared(vec![1,2,3,4,5,6]) == 56

Return ONLY the Rust code, no markdown."#;

    // === Run Workflow ===
    println!("\n[3/3] Running workflow...\n");
    println!("Task: Sum of even numbers squared\n");
    println!("{}", "-".repeat(70));

    let config = WorkflowConfig::new()
        .with_max_supersteps(15) // Allow for retry cycles
        .with_max_retries(3);

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
        for (i, output) in result.outputs.iter().enumerate() {
            if output.starts_with("//") || output.len() > 100 {
                // This is likely the final code
                println!("\n  [Final Code]");
                let lines: Vec<&str> = output.lines().collect();
                for line in lines.iter().take(20) {
                    println!("    {}", line);
                }
                if lines.len() > 20 {
                    println!("    ... ({} more lines)", lines.len() - 20);
                }
            } else {
                println!("  [{}] {}", i + 1, output);
            }
        }
    }

    if let Some(error) = &result.error {
        println!("\n  Error: {}", error);
    }

    println!("\n{}\n", "=".repeat(70));

    Ok(())
}
