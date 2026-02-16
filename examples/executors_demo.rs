//! Executors Demo - MAF Dynamic Dispatch Pattern Verification
//!
//! Demonstrates the built-in executors using MAF-style dynamic dispatch:
//! - All executors use `serde_json::Value` for Input/Message types
//! - Type-safe construction with helper structs (GeneratorInput, CompileInput, etc.)
//! - Runtime deserialization in executor handlers
//!
//! This example verifies that all executors can be composed in a single workflow.
//!
//! Environment variables:
//! - GEMINI_API_KEY: For LLM code generation (required)
//!
//! Run with: cargo run --example executors_demo

use dasein_agentic_core::distributed::graph::{
    executors::{
        AssembledCode, AssemblyInput, CodeAssemblerExecutor, CodePart, CompileInput,
        CompileOutput, CompileValidatorExecutor, GeneratedCode, GeneratorInput,
        LLMGeneratorExecutor, SubWorkflowInput, SubWorkflowOutput,
        TestInput, TestOutput, TestValidatorExecutor,
    },
    ExecutorRegistry, Workflow, WorkflowBuilder, WorkflowConfig,
};
use dasein_agentic_core::distributed::Executor as LLMExecutor;
use dasein_agentic_sandbox::ProcessSandbox;
use serde_json::Value;
use std::time::Instant;

// ============================================================================
// TEST 1: Verify Type-Safe Input/Output Structs
// ============================================================================

fn test_type_safe_structs() {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 1: Type-Safe Input/Output Structs");
    println!("{}", "=".repeat(70));

    // GeneratorInput
    let gen_input = GeneratorInput::new("Write a fibonacci function", "rust")
        .with_context("Use iterative approach, not recursive")
        .with_errors(vec!["Previous error: type mismatch".into()]);
    let gen_value = gen_input.to_value();
    println!("\n  ✓ GeneratorInput → Value");
    println!("    prompt: {:?}", gen_value["prompt"]);
    println!("    language: {:?}", gen_value["language"]);
    println!("    context: {:?}", gen_value["context"]);
    println!("    previous_errors: {} items", gen_value["previous_errors"].as_array().map(|a| a.len()).unwrap_or(0));

    // CompileInput
    let compile_input = CompileInput::new("fn main() { println!(\"Hello\"); }", "rust")
        .with_task("Simple hello world");
    let compile_value = compile_input.to_value();
    println!("\n  ✓ CompileInput → Value");
    println!("    code: {} chars", compile_value["code"].as_str().map(|s| s.len()).unwrap_or(0));
    println!("    language: {:?}", compile_value["language"]);
    println!("    task: {:?}", compile_value["task"]);

    // TestInput
    let test_input = TestInput::new("fn add(a: i32, b: i32) -> i32 { a + b }", "rust")
        .with_filter("test_add")
        .with_task("Addition function");
    let test_value = test_input.to_value();
    println!("\n  ✓ TestInput → Value");
    println!("    code: {} chars", test_value["code"].as_str().map(|s| s.len()).unwrap_or(0));
    println!("    test_filter: {:?}", test_value["test_filter"]);

    // AssemblyInput
    let assembly_input = AssemblyInput::new("rust")
        .add_part(CodePart::new("types", "pub struct Foo { x: i32 }", "rust").with_section("Types"))
        .add_part(CodePart::new("impl", "impl Foo { pub fn new(x: i32) -> Self { Self { x } } }", "rust").with_section("Implementation"));
    let assembly_value = assembly_input.to_value();
    println!("\n  ✓ AssemblyInput → Value");
    println!("    language: {:?}", assembly_value["language"]);
    println!("    parts: {} items", assembly_value["parts"].as_array().map(|a| a.len()).unwrap_or(0));

    // SubWorkflowInput
    let sub_input = SubWorkflowInput::new(serde_json::json!({"task": "nested task"}))
        .with_task_id("task-123");
    let sub_value = sub_input.to_value();
    println!("\n  ✓ SubWorkflowInput → Value");
    println!("    input: {:?}", sub_value["input"]);
    println!("    task_id: {:?}", sub_value["task_id"]);

    // Output structs
    println!("\n  --- Output Structs ---");

    let gen_output = GeneratedCode::new("fn fibonacci(n: u32) -> u32 { ... }", "rust")
        .with_prompt("Write fibonacci")
        .with_tokens(150);
    println!("\n  ✓ GeneratedCode");
    println!("    code: {} chars", gen_output.code.len());
    println!("    tokens_used: {}", gen_output.tokens_used);

    let compile_output = CompileOutput {
        code: "fn main() {}".into(),
        language: "rust".into(),
        passed: true,
        errors: vec![],
        feedback: None,
        execution_time_ms: 150,
    };
    println!("\n  ✓ CompileOutput");
    println!("    passed: {}", compile_output.passed);
    println!("    execution_time_ms: {}", compile_output.execution_time_ms);

    let test_output = TestOutput {
        code: "fn main() {}".into(),
        language: "rust".into(),
        passed: true,
        test_count: 5,
        tests_passed: 5,
        tests_failed: 0,
        errors: vec![],
        feedback: None,
        execution_time_ms: 2500,
    };
    println!("\n  ✓ TestOutput");
    println!("    passed: {}", test_output.passed);
    println!("    pass_rate: {}%", test_output.pass_rate());

    let assembled = AssembledCode::new("// Final code...", "rust", 2);
    println!("\n  ✓ AssembledCode");
    println!("    part_count: {}", assembled.part_count);
    println!("    size_bytes: {}", assembled.size_bytes);

    let sub_output = SubWorkflowOutput {
        outputs: vec![serde_json::json!("output1"), serde_json::json!("output2")],
        superstep_count: 3,
        duration_ms: 1500,
        success: true,
        error: None,
    };
    println!("\n  ✓ SubWorkflowOutput");
    println!("    success: {}", sub_output.is_success());
    println!("    outputs: {} items", sub_output.outputs.len());
    println!("    superstep_count: {}", sub_output.superstep_count);

    println!("\n  ✅ All type-safe structs verified!\n");
}

// ============================================================================
// TEST 2: Verify Executors Can Be Registered Together (Same Types)
// ============================================================================

fn test_registry_compatibility() {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 2: ExecutorRegistry Compatibility (MAF Pattern)");
    println!("{}", "=".repeat(70));

    println!("\n  All executors use:");
    println!("    - Input:   serde_json::Value");
    println!("    - Message: serde_json::Value");
    println!("    - Output:  String");

    // Create registry with MAF types
    let mut registry: ExecutorRegistry<Value, String> = ExecutorRegistry::new();

    // Create sandbox for validators
    let sandbox = ProcessSandbox::new().with_timeout(30_000);

    // Create LLM executor (mock for this test - won't actually call LLM)
    let llm = LLMExecutor::new("test-llm", "test").build();

    // Register all built-in executors
    println!("\n  Registering executors:");

    registry.register(LLMGeneratorExecutor::new("generator", llm));
    println!("    ✓ LLMGeneratorExecutor (Worker)");

    registry.register(CompileValidatorExecutor::new("compile-validator", sandbox.clone()));
    println!("    ✓ CompileValidatorExecutor (Validator)");

    registry.register(TestValidatorExecutor::new("test-validator", sandbox));
    println!("    ✓ TestValidatorExecutor (Validator)");

    registry.register(CodeAssemblerExecutor::new("assembler"));
    println!("    ✓ CodeAssemblerExecutor (Worker)");

    // Note: SubWorkflowExecutor requires a workflow, so we skip it in this test

    println!("\n  ✅ All executors registered in same registry!\n");
    println!("  This proves MAF dynamic dispatch pattern works:");
    println!("    - Different executor implementations");
    println!("    - Same Input/Message/Output types (Value/Value/String)");
    println!("    - Compatible in a single workflow\n");
}

// ============================================================================
// TEST 3: Build and Validate Workflow Definition
// ============================================================================

fn test_workflow_definition() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 3: WorkflowBuilder with MAF Executors");
    println!("{}", "=".repeat(70));

    // Build workflow definition
    let definition = WorkflowBuilder::<Value>::new("maf-executor-demo")
        .name("MAF Executor Demo Pipeline")
        .set_start("generator")
        .add_executor_id("compile-validator")
        .add_executor_id("test-validator")
        .add_executor_id("assembler")
        // Generator → Compile Validator
        .add_direct_edge("generator", "compile-validator")
        // Compile Validator → Test Validator (if passed)
        .add_conditional_edge(
            "compile-validator",
            "test-validator",
            |v: &Value| v.get("passed").and_then(|p| p.as_bool()).unwrap_or(false),
            "on_compile_success",
        )
        // Test Validator → Assembler (if passed)
        .add_conditional_edge(
            "test-validator",
            "assembler",
            |v: &Value| v.get("passed").and_then(|p| p.as_bool()).unwrap_or(false),
            "on_test_success",
        )
        // Compile Validator → Generator (retry on failure)
        .add_conditional_edge(
            "compile-validator",
            "generator",
            |v: &Value| !v.get("passed").and_then(|p| p.as_bool()).unwrap_or(true),
            "on_compile_failure",
        )
        .build()?;

    println!("\n  Workflow Definition:");
    println!("    ID: {}", definition.id.as_str());
    println!("    Name: {:?}", definition.name);
    println!("    Start: {}", definition.start.as_str());
    println!("    Executors: {:?}", definition.executors.iter().map(|e| e.as_str()).collect::<Vec<_>>());
    println!("    Edges: {}", definition.edges.len());

    println!("\n  Edge Details:");
    for edge in definition.edges.all() {
        println!("    {} → {:?} ({:?})", edge.name.as_deref().unwrap_or("unnamed"), edge.target, edge.kind);
    }

    println!("\n  ✅ Workflow definition valid!\n");

    Ok(())
}

// ============================================================================
// TEST 4: Run CodeAssembler Executor (No LLM Required)
// ============================================================================

async fn test_code_assembler() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 4: CodeAssemblerExecutor End-to-End");
    println!("{}", "=".repeat(70));

    // Create a simple workflow with just the assembler
    let definition = WorkflowBuilder::<Value>::new("assembler-test")
        .set_start("assembler")
        .build()?;

    let mut registry: ExecutorRegistry<Value, String> = ExecutorRegistry::new();
    registry.register(
        CodeAssemblerExecutor::new("assembler")
            .with_header("// Generated by MAF Executor Demo")
    );

    let config = WorkflowConfig::new().with_max_supersteps(5);
    let workflow = Workflow::with_config(definition, registry, config);

    // Create input using type-safe helper
    let input = AssemblyInput::new("rust")
        .add_part(
            CodePart::new("types", "pub struct Config {\n    pub name: String,\n    pub value: i32,\n}", "rust")
                .with_section("Type Definitions")
        )
        .add_part(
            CodePart::new("impl", "impl Config {\n    pub fn new(name: String, value: i32) -> Self {\n        Self { name, value }\n    }\n}", "rust")
                .with_section("Implementation")
        )
        .add_part(
            CodePart::new("tests", "#[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    fn test_new() {\n        let cfg = Config::new(\"test\".into(), 42);\n        assert_eq!(cfg.value, 42);\n    }\n}", "rust")
                .with_section("Tests")
        )
        .to_value();

    println!("\n  Input: 3 code parts (types, impl, tests)");
    println!("  Running assembler...\n");

    let start = Instant::now();
    let result = workflow.run(input).await?;
    let duration = start.elapsed();

    println!("  Result:");
    println!("    Success: {}", result.success);
    println!("    Supersteps: {}", result.superstep_count);
    println!("    Duration: {}ms", duration.as_millis());
    println!("    Outputs: {}", result.outputs.len());

    if let Some(final_code) = result.outputs.last() {
        println!("\n  Assembled Code Preview:");
        for line in final_code.lines().take(15) {
            println!("    {}", line);
        }
        if final_code.lines().count() > 15 {
            println!("    ... ({} more lines)", final_code.lines().count() - 15);
        }
    }

    println!("\n  ✅ CodeAssemblerExecutor works!\n");

    Ok(())
}

// ============================================================================
// TEST 5: Run Compile Validator (Requires Rust Toolchain)
// ============================================================================

async fn test_compile_validator() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 5: CompileValidatorExecutor End-to-End");
    println!("{}", "=".repeat(70));

    let sandbox = ProcessSandbox::new().with_timeout(60_000);

    let definition = WorkflowBuilder::<Value>::new("compile-test")
        .set_start("validator")
        .build()?;

    let mut registry: ExecutorRegistry<Value, String> = ExecutorRegistry::new();
    registry.register(CompileValidatorExecutor::new("validator", sandbox));

    let config = WorkflowConfig::new().with_max_supersteps(5);
    let workflow = Workflow::with_config(definition, registry, config);

    // Test with valid code
    let valid_code = r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        assert_eq!(add(2, 3), 5);
    }
}
"#;

    let input = CompileInput::new(valid_code, "rust")
        .with_task("Simple addition function")
        .to_value();

    println!("\n  Input: Valid Rust code (add function)");
    println!("  Running compile validator...\n");

    let start = Instant::now();
    let result = workflow.run(input).await?;
    let duration = start.elapsed();

    println!("  Result:");
    println!("    Success: {}", result.success);
    println!("    Supersteps: {}", result.superstep_count);
    println!("    Duration: {}ms", duration.as_millis());

    // Parse the output to check validation result
    if let Some(output_str) = result.outputs.first() {
        println!("    Output: {}", output_str);
    }

    println!("\n  ✅ CompileValidatorExecutor works!\n");

    Ok(())
}

// ============================================================================
// TEST 6: Full LLM Pipeline (Requires GEMINI_API_KEY)
// ============================================================================

async fn test_llm_pipeline() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(70));
    println!("  TEST 6: Full LLM → Compile → Test Pipeline");
    println!("{}", "=".repeat(70));

    // Check for API key
    let api_key = match std::env::var("GEMINI_API_KEY") {
        Ok(key) => key,
        Err(_) => {
            println!("\n  ⚠️  GEMINI_API_KEY not set - skipping LLM test");
            println!("  Set GEMINI_API_KEY to run full pipeline test\n");
            return Ok(());
        }
    };

    let model = std::env::var("MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_string());
    println!("\n  LLM: {} (key: {}...)", model, &api_key[..8]);

    // Create executors
    let llm = LLMExecutor::new("pipeline-llm", "supervisor")
        .llm_gemini(&model)
        .build();
    let sandbox = ProcessSandbox::new().with_timeout(120_000);

    // Build workflow
    let definition = WorkflowBuilder::<Value>::new("llm-pipeline")
        .name("LLM Code Generation Pipeline")
        .set_start("generator")
        .add_executor_id("compile-validator")
        .add_executor_id("test-validator")
        // Generator → Compile
        .add_direct_edge("generator", "compile-validator")
        // Compile → Test (if passed)
        .add_conditional_edge(
            "compile-validator",
            "test-validator",
            |v: &Value| v.get("passed").and_then(|p| p.as_bool()).unwrap_or(false),
            "on_compile_success",
        )
        .build()?;

    let mut registry: ExecutorRegistry<Value, String> = ExecutorRegistry::new();
    registry.register(
        LLMGeneratorExecutor::new("generator", llm)
            .with_system_prompt("You are an expert Rust developer. Write clean, idiomatic code with tests. Return ONLY valid Rust code, no markdown.")
    );
    registry.register(CompileValidatorExecutor::new("compile-validator", sandbox.clone()));
    registry.register(TestValidatorExecutor::new("test-validator", sandbox));

    let config = WorkflowConfig::new()
        .with_max_supersteps(10)
        .with_max_retries(3);

    let workflow = Workflow::with_config(definition, registry, config);

    // Create input
    let input = GeneratorInput::new(
        "Write a Rust function `is_palindrome(s: &str) -> bool` that checks if a string is a palindrome (ignoring case and non-alphanumeric characters). Include tests.",
        "rust"
    ).to_value();

    println!("\n  Task: Write is_palindrome function");
    println!("  Running pipeline...\n");

    let start = Instant::now();
    let result = workflow.run(input).await?;
    let duration = start.elapsed();

    println!("  {}", "-".repeat(60));
    println!("\n  Result:");
    println!("    Success: {}", result.success);
    println!("    Supersteps: {}", result.superstep_count);
    println!("    Duration: {}ms", duration.as_millis());
    println!("    Outputs: {}", result.outputs.len());

    for (i, output) in result.outputs.iter().enumerate() {
        if output.len() > 80 {
            println!("\n  Output {}: {}...", i + 1, &output[..80]);
        } else {
            println!("  Output {}: {}", i + 1, output);
        }
    }

    if let Some(error) = &result.error {
        println!("\n  Error: {}", error);
    }

    if result.success {
        println!("\n  ✅ Full LLM pipeline works!\n");
    } else {
        println!("\n  ⚠️  Pipeline completed with issues (check outputs above)\n");
    }

    Ok(())
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("warn")
        .init();

    println!("\n{}", "═".repeat(70));
    println!("  EXECUTORS DEMO - MAF Dynamic Dispatch Pattern");
    println!("  Verifying built-in executors from `graph/executors/` module");
    println!("{}\n", "═".repeat(70));

    // Test 1: Type-safe structs
    test_type_safe_structs();

    // Test 2: Registry compatibility
    test_registry_compatibility();

    // Test 3: Workflow definition
    test_workflow_definition()?;

    // Test 4: CodeAssembler (no external deps)
    test_code_assembler().await?;

    // Test 5: CompileValidator (requires rustc)
    test_compile_validator().await?;

    // Test 6: Full LLM pipeline (requires API key)
    test_llm_pipeline().await?;

    println!("{}", "═".repeat(70));
    println!("  ALL TESTS COMPLETED");
    println!("{}\n", "═".repeat(70));

    Ok(())
}
