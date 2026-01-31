//! Validator Pipeline Example
//!
//! Demonstrates the multi-validator architecture:
//! - SandboxValidator: compiles and tests code
//! - MCPDocValidator: enriches feedback with documentation
//!
//! Run with: CONTEXT7_API_KEY=xxx cargo run --example validator_pipeline

use agentic_core::distributed::{
    Executor, MCPDocConfig, MCPDocValidator, PipelineResult, SandboxPipelineValidator,
    ValidatorInput, ValidatorPipeline,
};
use agentic_sandbox::ProcessSandbox;
use std::path::PathBuf;
use std::time::Instant;

const MAX_ITERATIONS: u32 = 6;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();

    println!("\n{}", "=".repeat(70));
    println!("  VALIDATOR PIPELINE DEMO");
    println!("  SandboxValidator + MCPDocValidator");
    println!("{}\n", "=".repeat(70));

    // === Setup Pipeline ===
    println!("[1/3] Setting up validator pipeline...\n");

    let sandbox = ProcessSandbox::new().with_timeout(120_000);
    let sandbox_validator = SandboxPipelineValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/pipeline-validation"))
        .run_tests(true);

    // Setup MCP doc validator (optional, needs API key)
    let api_key = std::env::var("CONTEXT7_API_KEY").ok();

    let pipeline = if let Some(key) = api_key {
        println!("     Context7 API key found - enabling doc enrichment");
        let mcp_config = MCPDocConfig::context7(&key);
        let mcp_validator = MCPDocValidator::new(mcp_config);

        ValidatorPipeline::new()
            .add(sandbox_validator)
            .add(mcp_validator)
    } else {
        println!("     No Context7 API key - using sandbox only");
        ValidatorPipeline::new().add(sandbox_validator)
    };

    // === Setup Executor ===
    println!("\n[2/3] Setting up executor...\n");

    let executor = Executor::new("exe-pipeline", "supervisor-main")
        .llm_gemini("gemini-2.0-flash")
        .build();

    // === Define Task ===
    let task = r#"Write a Rust async function that:
1. Takes a Vec<String> of URLs
2. Fetches all URLs concurrently using reqwest
3. Returns Vec<Result<String, Error>> with response bodies
4. Limits concurrency to 5 simultaneous requests using tokio::sync::Semaphore

Include a simple test.
Return ONLY the Rust code, no markdown."#;

    let system_prompt = r#"You are an expert Rust developer.
Write clean, idiomatic async Rust code.
Use tokio for async runtime and reqwest for HTTP.
Include tests in a #[cfg(test)] module.
Return ONLY compilable Rust code."#;

    // === Grounded Loop with Pipeline ===
    println!("[3/3] Running grounded validation loop...\n");
    println!("Task: Concurrent URL fetcher with semaphore\n");

    let total_start = Instant::now();
    let mut iteration = 0;
    let mut current_prompt = task.to_string();

    loop {
        iteration += 1;
        println!("--- Iteration {} / {} ---", iteration, MAX_ITERATIONS);

        // Generate code
        let gen_start = Instant::now();
        let result = executor.execute(system_prompt, &current_prompt).await?;
        let code = clean_code(&result.content);
        println!(
            "Generated {} chars in {}ms",
            code.len(),
            gen_start.elapsed().as_millis()
        );

        // Validate with pipeline
        let val_start = Instant::now();
        let input = ValidatorInput::new(&code, "rust").with_task(task);

        let pipeline_result = pipeline.validate(input).await;
        println!("Pipeline: {}ms", val_start.elapsed().as_millis());

        // Print summary
        print_pipeline_summary(&pipeline_result);

        if pipeline_result.passed {
            println!("\n{}", "=".repeat(70));
            println!(
                "SUCCESS after {} iterations ({}ms total)",
                iteration,
                total_start.elapsed().as_millis()
            );
            println!("{}\n", "=".repeat(70));
            println!("{}", code);
            return Ok(());
        }

        if iteration >= MAX_ITERATIONS {
            println!("\nMax iterations reached.");
            println!("\nLast feedback:\n{}", pipeline_result.combined_feedback);
            return Err("Max iterations".into());
        }

        // Build retry prompt with enriched feedback
        current_prompt = build_retry_prompt(task, &code, &pipeline_result);
    }
}

fn print_pipeline_summary(result: &PipelineResult) {
    for r in &result.results {
        let status = if r.passed { "OK" } else { "FAIL" };
        print!("  {}: {}", r.validator, status);

        if !r.passed {
            print!(" ({} errors)", r.errors.len());
        }

        if !r.documentation.is_empty() {
            print!(" [+{} docs]", r.documentation.len());
        }

        if !r.recommendations.is_empty() {
            print!(" [+{} recs]", r.recommendations.len());
        }

        println!();
    }

    // Show first error if any
    for r in &result.results {
        if !r.passed && !r.errors.is_empty() {
            println!(
                "    First error: {}",
                r.errors[0].lines().next().unwrap_or("")
            );
            break;
        }
    }
}

fn build_retry_prompt(task: &str, code: &str, result: &PipelineResult) -> String {
    let mut prompt = String::new();

    prompt.push_str("=== ORIGINAL TASK ===\n");
    prompt.push_str(task);
    prompt.push_str("\n\n=== YOUR CODE ===\n```rust\n");
    prompt.push_str(code);
    prompt.push_str("\n```\n\n");

    prompt.push_str("=== VALIDATION FEEDBACK ===\n");
    prompt.push_str(&result.combined_feedback);

    prompt.push_str(
        "\n\nFix the errors using the feedback above. Return ONLY the corrected Rust code.",
    );
    prompt
}

fn clean_code(content: &str) -> String {
    let mut code = content.to_string();

    if code.contains("```rust") {
        code = code
            .split("```rust")
            .nth(1)
            .unwrap_or(&code)
            .split("```")
            .next()
            .unwrap_or(&code)
            .to_string();
    } else if code.contains("```") {
        code = code
            .split("```")
            .nth(1)
            .unwrap_or(&code)
            .split("```")
            .next()
            .unwrap_or(&code)
            .to_string();
    }

    code.trim().to_string()
}
