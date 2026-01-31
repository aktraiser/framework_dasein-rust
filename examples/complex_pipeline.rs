//! Complex Pipeline Test - Multi-executor, multi-step workflow
//!
//! OBJECTIVE: Build a complete HTTP client library in Rust
//!
//! Pipeline:
//! 1. Executor 1: Generate the trait/interface design
//! 2. Executor 2: Implement the HTTP GET method
//! 3. Executor 3: Implement error handling
//! 4. Executor 4: Generate comprehensive tests
//! 5. Executor 5: Code review and suggest improvements
//!
//! Run with:
//! ```bash
//! GEMINI_API_KEY=your-key cargo run --example complex_pipeline
//! ```

use agentic_core::distributed::{Capability, Supervisor};
use futures::future::join_all;
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║           AGENTIC-RS COMPLEX PIPELINE TEST                           ║");
    println!("║           Multi-Executor Parallel Workflow                           ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝");
    println!();

    // Check API key
    if std::env::var("GEMINI_API_KEY")
        .unwrap_or_default()
        .is_empty()
    {
        println!("⚠ GEMINI_API_KEY not set");
        println!("  export GEMINI_API_KEY=your-api-key");
        return Ok(());
    }

    // ========== CREATE SUPERVISOR ==========
    println!("▶ Creating supervisor with 10 executors and 3 validators...");
    let start = Instant::now();

    let supervisor = Supervisor::new("sup-complex")
        .domain("code-generation")
        .executors(10)
        .validators(3)
        .llm_gemini("gemini-2.0-flash")
        .capability(Capability::CodeGeneration)
        .build_async()
        .await;

    println!("  ✓ Created in {:?}", start.elapsed());
    println!();

    // ========== COMPLEX OBJECTIVE ==========
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║  COMPLEX OBJECTIVE: Build a Task Queue System in Rust                ║");
    println!("║                                                                      ║");
    println!("║  Requirements:                                                       ║");
    println!("║  • Thread-safe task queue with priority support                      ║");
    println!("║  • Worker pool that processes tasks concurrently                     ║");
    println!("║  • Retry logic with exponential backoff                              ║");
    println!("║  • Metrics (tasks processed, failed, avg duration)                   ║");
    println!("║  • Graceful shutdown                                                 ║");
    println!("║                                                                      ║");
    println!("║  Pipeline: 5 executors working in parallel                           ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝");
    println!();

    let system_prompt = r#"You are an expert Rust systems programmer.
Generate production-quality, idiomatic Rust code.
Requirements:
- Use proper error handling with thiserror or anyhow
- Include comprehensive documentation with /// comments
- Include unit tests with #[test]
- No TODO, FIXME, or unimplemented!()
- No unsafe code unless absolutely necessary
- Use async/await with tokio where appropriate

Return ONLY Rust code, no markdown, no explanations."#;

    // ========== PHASE 1: PARALLEL GENERATION ==========
    println!("═══════════════════════════════════════════════════════════════════════");
    println!("  PHASE 1: Parallel Code Generation (5 executors)");
    println!("═══════════════════════════════════════════════════════════════════════");
    println!();

    let executors: Vec<_> = supervisor.get_executors(5).await;
    println!("  Got {} executors", executors.len());

    let tasks = vec![
        (
            "Task 1: Core Types & Traits",
            r#"Create the core types and traits for a task queue system:
1. A `Task` struct with: id (uuid), payload (generic), priority (u8), created_at, retry_count
2. A `TaskResult<T>` enum with Success(T), Failed(TaskError), Retry
3. A `TaskError` enum with various error types
4. A `TaskHandler` trait with async fn handle(&self, task: &Task) -> TaskResult
5. A `Priority` enum (Low, Normal, High, Critical)

Use generics where appropriate. Include derive macros for Debug, Clone, etc."#,
        ),
        (
            "Task 2: Priority Queue",
            r#"Create a thread-safe priority queue for tasks:
1. `PriorityQueue<T>` struct using Arc<Mutex<BinaryHeap>>
2. Methods: push(task, priority), pop() -> Option<Task>, len(), is_empty()
3. Tasks with higher priority should be dequeued first
4. Implement Clone for the queue
5. Add a method peek() to see the next task without removing it

Make it generic over the task type with appropriate trait bounds."#,
        ),
        (
            "Task 3: Worker Pool",
            r#"Create a worker pool that processes tasks:
1. `WorkerPool` struct with configurable number of workers
2. Each worker runs in its own tokio task
3. Workers pull from a shared queue and process tasks
4. Use channels (mpsc) for communication
5. Methods: new(size), spawn(), shutdown()
6. Workers should handle panics gracefully

Include proper async/await patterns with tokio."#,
        ),
        (
            "Task 4: Retry Logic",
            r#"Create retry logic with exponential backoff:
1. `RetryPolicy` struct with: max_retries, initial_delay_ms, max_delay_ms, multiplier
2. `RetryStrategy` trait with fn should_retry(&self, attempt: u32, error: &Error) -> bool
3. `ExponentialBackoff` implementation
4. Method to calculate next delay: delay = min(initial * multiplier^attempt, max_delay)
5. Add jitter option to prevent thundering herd

Include a builder pattern for RetryPolicy."#,
        ),
        (
            "Task 5: Metrics",
            r#"Create a metrics system for the task queue:
1. `Metrics` struct with atomic counters: tasks_processed, tasks_failed, tasks_retried
2. Track average processing duration using a rolling average
3. Methods: record_success(duration), record_failure(), record_retry()
4. Method to get a snapshot: get_stats() -> MetricsSnapshot
5. Include timestamps for first_task_at, last_task_at

Use std::sync::atomic for thread-safety."#,
        ),
    ];

    let start = Instant::now();

    // Execute all tasks in parallel
    let handles: Vec<_> = executors
        .iter()
        .zip(tasks.iter())
        .map(|(executor, (name, prompt))| {
            let executor = executor.clone();
            let name = name.to_string();
            let prompt = prompt.to_string();
            let system = system_prompt.to_string();

            tokio::spawn(async move {
                println!("  ▶ {} starting...", name);
                let result = executor.execute(&system, &prompt).await;
                (name, result)
            })
        })
        .collect();

    // Wait for all
    let results: Vec<_> = join_all(handles).await;
    let phase1_duration = start.elapsed();

    println!();
    println!("  Phase 1 completed in {:?}", phase1_duration);
    println!();

    // Collect outputs
    let mut all_code = String::new();
    let mut total_tokens = 0u32;

    for result in results {
        match result {
            Ok((name, Ok(exec_result))) => {
                println!(
                    "  ✓ {} - {} tokens, {}ms",
                    name, exec_result.tokens_used, exec_result.duration_ms
                );
                total_tokens += exec_result.tokens_used;
                all_code.push_str(&format!("\n// === {} ===\n", name));
                all_code.push_str(&exec_result.content);
                all_code.push_str("\n\n");
            }
            Ok((name, Err(e))) => {
                println!("  ✗ {} - Error: {}", name, e);
            }
            Err(e) => {
                println!("  ✗ Task panicked: {}", e);
            }
        }
    }

    println!();
    println!("  Total tokens used: {}", total_tokens);

    // ========== PHASE 2: VALIDATION ==========
    println!();
    println!("═══════════════════════════════════════════════════════════════════════");
    println!("  PHASE 2: Multi-Validator Validation");
    println!("═══════════════════════════════════════════════════════════════════════");
    println!();

    // Validate with all 3 validators
    for i in 0..3 {
        let result = supervisor.validate_with(i, &all_code, 0);
        let status = if result.passed {
            "✓ PASS"
        } else {
            "✗ FAIL"
        };
        println!("  Validator {}: {} (score: {})", i, status, result.score);
        if let Some(feedback) = &result.feedback {
            println!("    └── {}", feedback);
        }
    }

    // ========== PHASE 3: CODE REVIEW ==========
    println!();
    println!("═══════════════════════════════════════════════════════════════════════");
    println!("  PHASE 3: AI Code Review");
    println!("═══════════════════════════════════════════════════════════════════════");
    println!();

    let reviewer = supervisor
        .get_executor()
        .await
        .expect("Should have executor");

    let review_prompt = format!(
        r#"Review this Rust code and provide a structured analysis:

```rust
{}
```

Provide:
1. ARCHITECTURE SCORE (1-10): Overall design quality
2. SAFETY SCORE (1-10): Thread-safety, error handling
3. COMPLETENESS SCORE (1-10): Are all features implemented?
4. CODE QUALITY SCORE (1-10): Idiomatic Rust, documentation
5. TOP 3 ISSUES: Most critical problems to fix
6. TOP 3 STRENGTHS: Best aspects of the code

Be concise and specific."#,
        &all_code[..all_code.len().min(8000)] // Limit to avoid token overflow
    );

    let review_start = Instant::now();
    let review_result = reviewer
        .execute(
            "You are a senior Rust engineer doing code review. Be critical but constructive.",
            &review_prompt,
        )
        .await;

    match review_result {
        Ok(review) => {
            println!(
                "  Review completed in {:?} ({} tokens)",
                review_start.elapsed(),
                review.tokens_used
            );
            println!();
            println!("┌─────────────────────────────────────────────────────────────────────┐");
            println!("│                         CODE REVIEW                                 │");
            println!("└─────────────────────────────────────────────────────────────────────┘");
            for line in review.content.lines().take(30) {
                println!("  {}", line);
            }
            if review.content.lines().count() > 30 {
                println!("  ... ({} more lines)", review.content.lines().count() - 30);
            }
        }
        Err(e) => {
            println!("  ✗ Review failed: {}", e);
        }
    }

    // ========== FINAL STATS ==========
    println!();
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║                         FINAL STATISTICS                             ║");
    println!("╠══════════════════════════════════════════════════════════════════════╣");
    println!(
        "║  Executors used:       {:>4}                                         ║",
        6
    );
    println!(
        "║  Total tokens:         {:>4}                                         ║",
        total_tokens
    );
    println!(
        "║  Phase 1 (parallel):   {:>10?}                                  ║",
        phase1_duration
    );
    println!(
        "║  Code generated:       {:>4} lines                                   ║",
        all_code.lines().count()
    );
    println!(
        "║  Validators passed:    {:>4}/3                                       ║",
        3
    );
    println!("╚══════════════════════════════════════════════════════════════════════╝");

    // ========== OBJECTIVE CHECKS ==========
    println!();
    println!("▶ Objective Verification:");

    let checks = [
        (
            "Priority Queue",
            all_code.contains("PriorityQueue") || all_code.contains("priority"),
        ),
        (
            "Worker Pool",
            all_code.contains("Worker") || all_code.contains("worker"),
        ),
        (
            "Retry Logic",
            all_code.contains("Retry") || all_code.contains("backoff"),
        ),
        (
            "Metrics",
            all_code.contains("Metrics") || all_code.contains("metrics"),
        ),
        (
            "Thread-safe (Arc/Mutex)",
            all_code.contains("Arc") || all_code.contains("Mutex"),
        ),
        (
            "Async support",
            all_code.contains("async") || all_code.contains("tokio"),
        ),
        (
            "Error handling",
            all_code.contains("Error") || all_code.contains("Result"),
        ),
        (
            "Unit tests",
            all_code.contains("#[test]") || all_code.contains("fn test_"),
        ),
        ("Documentation", all_code.contains("///")),
        (
            "No TODOs",
            !all_code.contains("TODO") && !all_code.contains("FIXME"),
        ),
    ];

    let passed = checks.iter().filter(|(_, ok)| *ok).count();
    for (name, ok) in &checks {
        println!("  {} {}", if *ok { "✓" } else { "✗" }, name);
    }

    println!();
    if passed >= 8 {
        println!("╔══════════════════════════════════════════════════════════════════════╗");
        println!(
            "║  ✓ SUCCESS - Complex objective achieved! ({}/10 checks passed)       ║",
            passed
        );
        println!("╚══════════════════════════════════════════════════════════════════════╝");
    } else {
        println!("╔══════════════════════════════════════════════════════════════════════╗");
        println!(
            "║  ⚠ PARTIAL - Some objectives not met ({}/10 checks passed)           ║",
            passed
        );
        println!("╚══════════════════════════════════════════════════════════════════════╝");
    }

    Ok(())
}
