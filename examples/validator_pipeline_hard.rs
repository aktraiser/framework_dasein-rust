//! Hard Validator Pipeline Example
//!
//! A complex task: Concurrent LRU Cache with TTL
//!
//! Run with: CONTEXT7_API_KEY=xxx cargo run --example validator_pipeline_hard

use agentic_core::distributed::{
    Executor, ValidatorPipeline, SandboxPipelineValidator, MCPDocValidator,
    MCPDocConfig, ValidatorInput, PipelineResult,
};
use agentic_sandbox::ProcessSandbox;
use std::path::PathBuf;
use std::time::Instant;

const MAX_ITERATIONS: u32 = 8;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();

    println!("\n{}", "=".repeat(70));
    println!("  HARD VALIDATOR PIPELINE TEST");
    println!("  Task: Concurrent LRU Cache with TTL");
    println!("{}\n", "=".repeat(70));

    // === Setup Pipeline ===
    let sandbox = ProcessSandbox::new().with_timeout(180_000);
    let sandbox_validator = SandboxPipelineValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/hard-pipeline-validation"))
        .run_tests(true);

    let api_key = std::env::var("CONTEXT7_API_KEY").ok();

    let pipeline = if let Some(key) = api_key {
        println!("[Setup] Context7 enabled for doc enrichment\n");
        let mcp_config = MCPDocConfig::context7(&key);
        let mcp_validator = MCPDocValidator::new(mcp_config);
        ValidatorPipeline::new()
            .add(sandbox_validator)
            .add(mcp_validator)
    } else {
        println!("[Setup] No Context7 - sandbox only\n");
        ValidatorPipeline::new().add(sandbox_validator)
    };

    let executor = Executor::new("exe-hard", "supervisor-main")
        .llm_gemini("gemini-2.0-flash")
        .build();

    // === Complex Task ===
    let task = r#"Implement a thread-safe LRU cache with TTL in Rust.

Requirements:
1. `AsyncLruCache<K, V>` where K: Hash + Eq + Clone, V: Clone
2. Constructor: `new(capacity: usize, default_ttl: Duration)`
3. Methods (all async):
   - `get(&self, key: &K) -> Option<V>` - returns None if expired or missing
   - `insert(&self, key: K, value: V)` - uses default TTL
   - `insert_with_ttl(&self, key: K, value: V, ttl: Duration)`
   - `remove(&self, key: &K) -> Option<V>`
   - `len(&self) -> usize` - count of non-expired entries
   - `clear(&self)`

4. LRU eviction: when at capacity, remove least recently used
5. TTL: entries expire after their TTL, get() returns None for expired
6. Thread-safe: use tokio::sync::RwLock
7. Use std::collections::HashMap and a way to track access order

Include tests for:
- Basic get/insert
- LRU eviction (insert beyond capacity)
- TTL expiration
- Concurrent access

Return ONLY Rust code, no markdown."#;

    let system_prompt = r#"You are an expert Rust developer specializing in async and concurrent programming.

CRITICAL RULES:
1. Use ONLY stable Rust features - NO nightly features, NO unstable APIs
2. Do NOT use LinkedList - use Vec or VecDeque instead
3. Use tokio::sync::RwLock (not std::sync)
4. Use std::time::Instant for timestamps
5. For LRU tracking, use Vec<K> and move accessed items to the end

Write production-quality async Rust code.
Include comprehensive tests with #[tokio::test].
Return ONLY compilable Rust code, no explanations."#;

    // === Grounded Loop ===
    println!("Task: Concurrent LRU Cache with TTL\n");
    println!("This is a complex task requiring:");
    println!("  - Generic types with multiple bounds");
    println!("  - Interior mutability (RwLock)");
    println!("  - LRU eviction logic");
    println!("  - TTL expiration handling");
    println!("  - Concurrent safety\n");

    let total_start = Instant::now();
    let mut iteration = 0;
    let mut current_prompt = task.to_string();

    loop {
        iteration += 1;
        println!("--- Iteration {} / {} ---", iteration, MAX_ITERATIONS);

        let gen_start = Instant::now();
        let result = executor.execute(system_prompt, &current_prompt).await?;
        let code = clean_code(&result.content);
        println!("Generated {} chars in {}ms", code.len(), gen_start.elapsed().as_millis());

        let val_start = Instant::now();
        let input = ValidatorInput::new(&code, "rust").with_task(task);
        let pipeline_result = pipeline.validate(input).await;
        println!("Pipeline: {}ms", val_start.elapsed().as_millis());

        print_pipeline_summary(&pipeline_result);

        if pipeline_result.passed {
            println!("\n{}", "=".repeat(70));
            println!("SUCCESS after {} iterations ({}ms total)",
                iteration, total_start.elapsed().as_millis());
            println!("{}\n", "=".repeat(70));

            // Print code stats
            let lines = code.lines().count();
            let test_count = code.matches("#[tokio::test]").count();
            println!("Code: {} lines, {} tests\n", lines, test_count);
            println!("{}", code);
            return Ok(());
        }

        if iteration >= MAX_ITERATIONS {
            println!("\n{}", "=".repeat(70));
            println!("FAILED after {} iterations", MAX_ITERATIONS);
            println!("{}\n", "=".repeat(70));
            println!("Last feedback:\n{}", pipeline_result.combined_feedback);
            println!("\nLast code:\n{}", code);
            return Err("Max iterations".into());
        }

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

    // Show first 2 errors
    let mut shown = 0;
    for r in &result.results {
        if !r.passed {
            for err in r.errors.iter().take(2) {
                if shown < 2 {
                    println!("    └─ {}", err.lines().next().unwrap_or(""));
                    shown += 1;
                }
            }
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
    prompt.push_str("\n\nFix ALL errors. Return ONLY the corrected Rust code.");
    prompt
}

fn clean_code(content: &str) -> String {
    let mut code = content.to_string();
    for marker in ["```rust", "```rs", "```"] {
        if code.contains(marker) {
            code = code
                .split(marker)
                .nth(1)
                .unwrap_or(&code)
                .split("```")
                .next()
                .unwrap_or(&code)
                .to_string();
            break;
        }
    }
    code.trim().to_string()
}
