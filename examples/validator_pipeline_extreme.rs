//! EXTREME Validator Pipeline Test
//!
//! Task: Async Task Scheduler with Dependencies, Retry, and Backoff
//!
//! This is intentionally very hard to test the framework limits.
//!
//! Run with: CONTEXT7_API_KEY=xxx cargo run --example validator_pipeline_extreme

use agentic_core::distributed::{
    Executor, MCPDocConfig, MCPDocValidator, PipelineResult, SandboxPipelineValidator,
    ValidatorInput, ValidatorPipeline,
};
use agentic_sandbox::ProcessSandbox;
use std::path::PathBuf;
use std::time::Instant;

const MAX_ITERATIONS: u32 = 10;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();

    println!("\n{}", "=".repeat(70));
    println!("  EXTREME VALIDATOR PIPELINE TEST");
    println!("  Task: Async Task Scheduler with Dependencies");
    println!("{}\n", "=".repeat(70));

    // === Setup Pipeline ===
    let sandbox = ProcessSandbox::new().with_timeout(180_000);
    let sandbox_validator = SandboxPipelineValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/extreme-pipeline-validation"))
        .run_tests(true);

    let api_key = std::env::var("CONTEXT7_API_KEY").ok();

    let pipeline = if let Some(key) = api_key {
        println!("[Setup] Context7 enabled\n");
        let mcp_config = MCPDocConfig::context7(&key);
        let mcp_validator = MCPDocValidator::new(mcp_config);
        ValidatorPipeline::new()
            .add(sandbox_validator)
            .add(mcp_validator)
    } else {
        println!("[Setup] Sandbox only\n");
        ValidatorPipeline::new().add(sandbox_validator)
    };

    let executor = Executor::new("exe-extreme", "supervisor-main")
        .llm_gemini("gemini-2.0-flash")
        .build();

    // === EXTREME Task ===
    let task = r#"Implement an async task scheduler with dependencies in Rust.

REQUIREMENTS:

1. `TaskId` - newtype wrapper around u64

2. `TaskDef<T>` struct:
   - id: TaskId
   - name: String
   - dependencies: Vec<TaskId> (tasks that must complete before this one)
   - max_retries: u32
   - timeout: Duration
   - task: Box<dyn Fn() -> BoxFuture<'static, Result<T, TaskError>> + Send + Sync>

3. `TaskError` enum:
   - Timeout
   - MaxRetriesExceeded
   - DependencyFailed(TaskId)
   - ExecutionError(String)

4. `TaskResult<T>` struct:
   - task_id: TaskId
   - result: Result<T, TaskError>
   - attempts: u32
   - duration: Duration

5. `Scheduler<T>` where T: Clone + Send + 'static:
   - new(max_concurrent: usize) -> Self
   - add_task(&mut self, task: TaskDef<T>) -> Result<(), SchedulerError>
     (returns error if dependency doesn't exist or creates cycle)
   - run(&self) -> Vec<TaskResult<T>>
     (executes all tasks respecting dependencies, with retry and backoff)

6. Execution rules:
   - Tasks run only after all dependencies complete successfully
   - If dependency fails, dependent tasks fail with DependencyFailed
   - Retry with exponential backoff: 100ms, 200ms, 400ms, ...
   - Respect max_concurrent limit using Semaphore
   - Timeout per task attempt

7. Cycle detection: add_task must reject if adding would create a cycle

Include tests for:
- Simple task execution
- Dependency ordering
- Retry with backoff
- Timeout handling
- Cycle detection
- Concurrent execution limit

Return ONLY Rust code."#;

    let system_prompt = r#"You are an expert Rust developer.

CRITICAL RULES:
1. Use ONLY stable Rust - NO nightly features
2. Use tokio::sync for concurrency (RwLock, Semaphore, Mutex)
3. Use std::time::{Duration, Instant}
4. Use futures::future::BoxFuture for async closures
5. Use HashMap for task storage, HashSet for visited tracking in cycle detection
6. For topological sort, use Kahn's algorithm or DFS-based approach
7. Use tokio::time::timeout for task timeouts
8. Use tokio::time::sleep for backoff delays

Write production-quality async Rust code.
Include comprehensive tests with #[tokio::test].
Return ONLY compilable Rust code, no explanations."#;

    // === Grounded Loop ===
    println!("Task: Async Task Scheduler with Dependencies\n");
    println!("Complexity:");
    println!("  - DAG with cycle detection");
    println!("  - Topological ordering");
    println!("  - Retry with exponential backoff");
    println!("  - Timeout per task");
    println!("  - Concurrent execution limits");
    println!("  - Async closures (BoxFuture)\n");

    let total_start = Instant::now();
    let mut iteration = 0;
    let mut current_prompt = task.to_string();
    let mut last_code_hash: u64 = 0;
    let mut stuck_count = 0;

    loop {
        iteration += 1;
        println!("--- Iteration {} / {} ---", iteration, MAX_ITERATIONS);

        let gen_start = Instant::now();
        let result = executor.execute(system_prompt, &current_prompt).await?;
        let code = clean_code(&result.content);
        println!(
            "Generated {} chars in {}ms",
            code.len(),
            gen_start.elapsed().as_millis()
        );

        // Detect if LLM is stuck generating same code
        let code_hash = hash_code(&code);
        if code_hash == last_code_hash {
            stuck_count += 1;
            println!(
                "  ⚠️  Same code as previous iteration (stuck: {})",
                stuck_count
            );
            if stuck_count >= 2 {
                println!("  🔄 Adding hint to break the loop...");
                current_prompt = format!(
                    "{}\n\nIMPORTANT: Your previous code had syntax errors (unbalanced braces). \
                    Start fresh with a SIMPLER implementation. Focus on getting the basic structure right first.",
                    current_prompt
                );
            }
        } else {
            stuck_count = 0;
        }
        last_code_hash = code_hash;

        let val_start = Instant::now();
        let input = ValidatorInput::new(&code, "rust").with_task(task);
        let pipeline_result = pipeline.validate(input).await;
        println!("Pipeline: {}ms", val_start.elapsed().as_millis());

        print_pipeline_summary(&pipeline_result);

        if pipeline_result.passed {
            println!("\n{}", "=".repeat(70));
            println!(
                "SUCCESS after {} iterations ({}ms total)",
                iteration,
                total_start.elapsed().as_millis()
            );
            println!("{}\n", "=".repeat(70));

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

    let mut shown = 0;
    for r in &result.results {
        if !r.passed {
            for err in r.errors.iter().take(3) {
                if shown < 3 {
                    let first_line = err.lines().next().unwrap_or("");
                    // Truncate long errors
                    let display: String = first_line.chars().take(70).collect();
                    println!("    └─ {}", display);
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
    prompt.push_str("\n\nFix ALL errors carefully. Return ONLY the corrected Rust code.");
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

fn hash_code(code: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    code.hash(&mut hasher);
    hasher.finish()
}
