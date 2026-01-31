//! Graph Workflow Example
//!
//! Demonstrates the graph-based multi-agent architecture:
//! - Executor trait with Input/Message/Output types
//! - WorkflowContext for inter-executor communication
//! - Edge types: Direct, Conditional, FanOut, FanIn
//! - Superstep execution (BSP/Pregel model)
//!
//! Run with: cargo run --example graph_workflow

use agentic_core::distributed::graph::{
    Executor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind, ExecutorRegistry,
    Workflow, WorkflowBuilder, WorkflowConfig,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ============================================================================
// MESSAGE TYPES
// ============================================================================

/// Message that flows through the workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct CodeMessage {
    code: String,
    language: String,
    metadata: Option<String>,
}

impl CodeMessage {
    fn new(code: impl Into<String>, language: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            language: language.into(),
            metadata: None,
        }
    }

    fn with_metadata(mut self, meta: impl Into<String>) -> Self {
        self.metadata = Some(meta.into());
        self
    }
}

// ============================================================================
// EXECUTOR 1: CODE GENERATOR
// ============================================================================

/// Generates code from a task description.
struct CodeGeneratorExecutor {
    id: ExecutorId,
}

impl CodeGeneratorExecutor {
    fn new() -> Self {
        Self {
            id: ExecutorId::new("code-generator"),
        }
    }
}

#[async_trait]
impl Executor for CodeGeneratorExecutor {
    type Input = String; // Task description
    type Message = CodeMessage;
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
        println!("[CodeGenerator] Received task: {}", input);

        // Simulate code generation
        let generated_code = format!(
            r#"// Generated from: {}
fn main() {{
    println!("Hello from generated code!");
}}
"#,
            input
        );

        println!(
            "[CodeGenerator] Generated {} bytes of code",
            generated_code.len()
        );

        // Send code to next executor(s)
        let message =
            CodeMessage::new(&generated_code, "rust").with_metadata(format!("Generated from: {}", input));

        ctx.send_message(message).await?;

        // Also yield output for the caller
        ctx.yield_output(format!("Code generated: {} bytes", generated_code.len()))
            .await?;

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 2: VALIDATOR
// ============================================================================

/// Validates code by checking syntax.
struct ValidatorExecutor {
    id: ExecutorId,
}

impl ValidatorExecutor {
    fn new() -> Self {
        Self {
            id: ExecutorId::new("validator"),
        }
    }
}

#[async_trait]
impl Executor for ValidatorExecutor {
    type Input = CodeMessage;
    type Message = CodeMessage;
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

        // Simple validation: check for main function
        let has_main = input.code.contains("fn main");
        let has_println = input.code.contains("println!");

        if has_main && has_println {
            println!("[Validator] ✓ Validation passed!");

            // Forward to assembler with validation metadata
            let message = CodeMessage {
                code: input.code,
                language: input.language,
                metadata: Some("validated".into()),
            };
            ctx.send_message(message).await?;
            ctx.yield_output("Validation: PASSED".into()).await?;
        } else {
            println!("[Validator] ✗ Validation failed");
            ctx.yield_output("Validation: FAILED".into()).await?;

            return Err(ExecutorError::validation(
                "Code missing main function or println",
            ));
        }

        Ok(())
    }
}

// ============================================================================
// EXECUTOR 3: ASSEMBLER
// ============================================================================

/// Assembles validated code into final output.
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
impl Executor for AssemblerExecutor {
    type Input = CodeMessage;
    type Message = CodeMessage;
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

        let assembled = format!(
            "// === ASSEMBLED CODE ===\n// Language: {}\n// Status: {}\n\n{}",
            input.language,
            input.metadata.as_deref().unwrap_or("unknown"),
            input.code
        );

        println!("[Assembler] Final output: {} bytes", assembled.len());

        // Yield the final assembled code
        ctx.yield_output(assembled).await?;

        Ok(())
    }
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(70));
    println!("  GRAPH WORKFLOW DEMO");
    println!("  Executor → Validator → Assembler Pipeline");
    println!("{}\n", "=".repeat(70));

    // === Step 1: Build the workflow definition ===
    println!("[1/3] Building workflow graph...\n");

    let definition = WorkflowBuilder::<CodeMessage>::new("code-pipeline")
        .name("Code Generation Pipeline")
        .set_start("code-generator")
        .add_executor_id("validator")
        .add_executor_id("assembler")
        // Direct edge: generator → validator
        .add_direct_edge("code-generator", "validator")
        // Direct edge: validator → assembler
        .add_direct_edge("validator", "assembler")
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

    // === Step 2: Create executor registry ===
    println!("\n[2/3] Registering executors...\n");

    let mut registry: ExecutorRegistry<CodeMessage, String> = ExecutorRegistry::new();
    registry.register(CodeGeneratorExecutor::new());
    registry.register(ValidatorExecutor::new());
    registry.register(AssemblerExecutor::new());

    println!("  Registered: code-generator (Worker)");
    println!("  Registered: validator (Validator)");
    println!("  Registered: assembler (Worker)");

    // === Step 3: Run the workflow ===
    println!("\n[3/3] Running workflow...\n");
    println!("{}", "-".repeat(70));

    let config = WorkflowConfig::new()
        .with_max_supersteps(10)
        .with_max_retries(3);

    let workflow = Workflow::with_config(definition, registry, config);

    let input = "Create a hello world program".to_string();
    println!("Input: \"{}\"\n", input);

    let result = workflow.run(input).await?;

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
    println!("  Duration: {}ms", result.duration_ms);
    println!("  Outputs: {}", result.outputs.len());

    if !result.outputs.is_empty() {
        println!("\n  --- Outputs ---");
        for (i, output) in result.outputs.iter().enumerate() {
            println!("\n  [Output {}]", i + 1);
            let lines: Vec<&str> = output.lines().collect();
            for line in lines.iter().take(5) {
                println!("    {}", line);
            }
            if lines.len() > 5 {
                println!("    ... ({} more lines)", lines.len() - 5);
            }
        }
    }

    if let Some(error) = &result.error {
        println!("\n  Error: {}", error);
    }

    println!("\n{}\n", "=".repeat(70));

    Ok(())
}

// ============================================================================
// ADVANCED EXAMPLE: CONDITIONAL WORKFLOW
// ============================================================================

/// Example of a conditional workflow with branching.
#[allow(dead_code)]
async fn conditional_workflow_example() -> Result<(), Box<dyn std::error::Error>> {
    // This demonstrates conditional edges based on validation results

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct ValidationMessage {
        code: String,
        passed: bool,
        errors: Vec<String>,
    }

    let _definition = WorkflowBuilder::<ValidationMessage>::new("conditional-pipeline")
        .set_start("generator")
        .add_executor_id("validator")
        .add_executor_id("success_handler")
        .add_executor_id("error_handler")
        // Generator → Validator (always)
        .add_direct_edge("generator", "validator")
        // Validator → SuccessHandler (if passed)
        .add_conditional_edge(
            "validator",
            "success_handler",
            |msg: &ValidationMessage| msg.passed,
            "on_success",
        )
        // Validator → ErrorHandler (if failed)
        .add_conditional_edge(
            "validator",
            "error_handler",
            |msg: &ValidationMessage| !msg.passed,
            "on_error",
        )
        .build()?;

    println!("Conditional workflow built successfully!");
    Ok(())
}

/// Example of a fan-out/fan-in workflow for parallel processing.
#[allow(dead_code)]
async fn parallel_workflow_example() -> Result<(), Box<dyn std::error::Error>> {
    // This demonstrates parallel processing with fan-out and fan-in

    let _definition = WorkflowBuilder::<CodeMessage>::new("parallel-pipeline")
        .set_start("splitter")
        // Fan-out: splitter → [worker1, worker2, worker3]
        .add_fan_out_edge("splitter", vec!["worker1", "worker2", "worker3"])
        // Fan-in: [worker1, worker2, worker3] → aggregator
        .add_fan_in_edge(vec!["worker1", "worker2", "worker3"], "aggregator")
        .build()?;

    println!("Parallel workflow built successfully!");
    Ok(())
}
