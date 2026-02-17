//! Grounded Validation Loop
//!
//! Demonstrates the complete feedback loop:
//! 1. Executor generates code via LLM
//! 2. SandboxValidator validates with REAL compilation and tests
//! 3. If failed, send actual compiler errors back to Executor
//! 4. Executor corrects the code based on real errors
//! 5. Repeat until success or max retries
//!
//! This is the "ground truth" validation approach - no LLM-reviewing-LLM bias,
//! just real compilation and test results.
//!
//! Run with: cargo run --example grounded_loop

use dasein_agentic_core::distributed::{Executor, SandboxValidationResult, SandboxValidator};
use dasein_agentic_sandbox::ProcessSandbox;
use std::path::PathBuf;
use std::time::Instant;

const MAX_ITERATIONS: u32 = 8;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt().with_target(false).init();

    println!("\n{}", "=".repeat(60));
    println!("  GROUNDED VALIDATION LOOP");
    println!("  Real compilation + Real tests = Ground truth");
    println!("{}\n", "=".repeat(60));

    // Create executor with Gemini
    let executor = Executor::new("exe-grounded", "supervisor-main")
        .llm_gemini("gemini-2.0-flash")
        .build();

    // Create sandbox validator (using process sandbox for dev)
    let sandbox = ProcessSandbox::new().with_timeout(120_000); // 2 min timeout
    let validator = SandboxValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/grounded-validation"))
        .run_tests(true);

    // The task to accomplish - More complex: a thread-safe LRU cache
    let task = r#"Implement a thread-safe LRU (Least Recently Used) cache in Rust with the following requirements:

1. Generic over key type K (must be Hash + Eq + Clone) and value type V (must be Clone)
2. Has a configurable capacity
3. Methods:
   - `new(capacity: usize) -> Self`
   - `get(&self, key: &K) -> Option<V>` - returns value and marks as recently used
   - `put(&self, key: K, value: V)` - inserts/updates, evicts LRU if at capacity
   - `len(&self) -> usize`
   - `is_empty(&self) -> bool`

4. Must be thread-safe (usable from multiple threads concurrently)
5. Use standard library only (no external crates)

Write comprehensive tests including:
- Basic get/put operations
- LRU eviction behavior
- Capacity limits
- Thread safety (spawn multiple threads accessing the cache)
- Edge cases (empty cache, capacity 1, etc.)"#;

    let system_prompt = r#"You are an expert Rust developer.
Write clean, idiomatic, and well-tested Rust code.
Always include the function implementation AND tests in a #[cfg(test)] module.
The code must compile and all tests must pass.

IMPORTANT: Return ONLY the Rust code, no markdown formatting, no explanations.
The code should be a complete Rust library file (lib.rs compatible)."#;

    println!("Task: {}\n", task);
    println!("Starting grounded validation loop...\n");

    let total_start = Instant::now();
    let mut iteration = 0;
    let mut current_prompt = task.to_string();

    loop {
        iteration += 1;
        println!("\n{}", "=".repeat(60));
        println!("ITERATION {} / {}", iteration, MAX_ITERATIONS);
        println!("{}\n", "=".repeat(60));

        // Phase 1: Generate code with LLM
        println!("[1/3] Generating code with LLM...");
        let gen_start = Instant::now();

        let result = executor.execute(system_prompt, &current_prompt).await;

        match result {
            Ok(exec_result) => {
                println!(
                    "     Generated {} chars in {}ms ({} tokens)",
                    exec_result.content.len(),
                    gen_start.elapsed().as_millis(),
                    exec_result.tokens_used
                );

                // Clean up code (remove markdown if present)
                let code = clean_code(&exec_result.content);

                // Phase 2: Validate with real compilation + tests
                println!("\n[2/3] Validating with REAL compilation and tests...");
                let val_start = Instant::now();

                match validator.validate_rust_code(&code).await {
                    Ok(validation) => {
                        print_validation_result(&validation);
                        println!("     Validation took {}ms", val_start.elapsed().as_millis());

                        // Phase 3: Check if passed or need retry
                        println!("\n[3/3] Decision...");

                        if validation.passed {
                            println!("     SUCCESS! Code compiles and all tests pass.");
                            println!("\n{}", "=".repeat(60));
                            println!("FINAL RESULT");
                            println!("{}\n", "=".repeat(60));
                            println!("Iterations: {}", iteration);
                            println!("Total time: {}ms", total_start.elapsed().as_millis());
                            println!("\n--- Generated Code ---\n");
                            println!("{}", code);
                            return Ok(());
                        }

                        if iteration >= MAX_ITERATIONS {
                            println!("     MAX ITERATIONS REACHED - giving up.");
                            println!("\n--- Last Generated Code ---\n");
                            println!("{}", code);
                            println!("\n--- Last Errors ---\n");
                            if let Some(feedback) = &validation.feedback {
                                println!("{}", feedback);
                            }
                            return Err("Max iterations reached without success".into());
                        }

                        // Build feedback prompt with real errors
                        println!("     RETRY - Building feedback from real errors...");
                        current_prompt = build_feedback_prompt(task, &code, &validation);

                        // Show a preview of the feedback
                        println!("\n     --- Feedback Preview ---");
                        for error in &validation.test_errors {
                            for line in error.lines().take(3) {
                                println!("     {}", line);
                            }
                        }
                        println!("     -------------------------");
                    }
                    Err(e) => {
                        println!("     Sandbox error: {}", e);
                        if iteration >= MAX_ITERATIONS {
                            return Err(e.into());
                        }
                        // Retry with generic message
                        current_prompt = format!(
                            "{}\n\nPrevious attempt had a sandbox error. Please try again with simpler code.",
                            task
                        );
                    }
                }
            }
            Err(e) => {
                println!("     LLM Error: {}", e);
                if iteration >= MAX_ITERATIONS {
                    return Err(e.into());
                }
            }
        }
    }
}

/// Clean up code by removing markdown formatting.
fn clean_code(content: &str) -> String {
    let mut code = content.to_string();

    // Remove markdown code blocks
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

/// Build a feedback prompt with real compiler/test errors.
fn build_feedback_prompt(
    original_task: &str,
    code: &str,
    validation: &SandboxValidationResult,
) -> String {
    let mut prompt = String::new();

    prompt.push_str("=== ORIGINAL TASK ===\n");
    prompt.push_str(original_task);
    prompt.push_str("\n\n");

    prompt.push_str("=== YOUR PREVIOUS CODE ===\n```rust\n");
    prompt.push_str(code);
    prompt.push_str("\n```\n\n");

    if !validation.compiles {
        prompt.push_str("=== COMPILATION FAILED ===\n");
        prompt.push_str("The code was compiled with `cargo check` and FAILED.\n\n");
        prompt.push_str("COMPILER OUTPUT:\n");
        prompt.push_str("```\n");
        for error in &validation.compiler_errors {
            prompt.push_str(error);
            prompt.push_str("\n\n");
        }
        prompt.push_str("```\n\n");
        prompt.push_str("=== INSTRUCTIONS ===\n");
        prompt.push_str("1. Read the compiler error messages carefully\n");
        prompt.push_str("2. Identify the exact line and column of the error\n");
        prompt.push_str("3. Fix the type mismatch, missing import, or syntax error\n");
        prompt.push_str("4. Return the COMPLETE corrected code\n");
    } else if !validation.tests_passed {
        prompt.push_str(&format!(
            "=== TESTS FAILED: {}/{} passed ===\n\n",
            validation.tests_ok, validation.test_count
        ));
        prompt.push_str("The code compiled successfully but some tests failed.\n");
        prompt.push_str("Tests were run with `cargo test`.\n\n");
        prompt.push_str("FAILED TEST OUTPUT:\n");
        prompt.push_str("```\n");
        for error in &validation.test_errors {
            prompt.push_str(error);
            prompt.push('\n');
        }
        prompt.push_str("```\n\n");
        prompt.push_str("=== INSTRUCTIONS ===\n");
        prompt.push_str("1. Identify which test failed and WHY\n");
        prompt.push_str("2. Look at 'left' vs 'right' values in assertions\n");
        prompt.push_str("3. The test expectations are CORRECT - fix your implementation\n");
        prompt.push_str("4. Think about edge cases: empty, capacity 0, concurrent access\n");
        prompt.push_str("5. Return the COMPLETE corrected code\n");
    }

    prompt.push_str("\nIMPORTANT: Return ONLY valid Rust code. No markdown. No explanations.");

    prompt
}

/// Print validation result in a readable format.
fn print_validation_result(result: &SandboxValidationResult) {
    if result.compiles {
        println!("     Compilation: PASS");
    } else {
        println!("     Compilation: FAIL");
        for error in &result.compiler_errors {
            // Print first line of each error
            if let Some(first_line) = error.lines().next() {
                println!("       - {}", first_line);
            }
        }
    }

    if result.compiles {
        if result.tests_passed {
            println!(
                "     Tests: PASS ({}/{} passed)",
                result.tests_ok, result.test_count
            );
        } else {
            println!(
                "     Tests: FAIL ({}/{} passed)",
                result.tests_ok, result.test_count
            );
            for error in &result.test_errors {
                if let Some(first_line) = error.lines().next() {
                    println!("       - {}", first_line);
                }
            }
        }
    }

    if !result.warnings.is_empty() {
        println!("     Warnings: {}", result.warnings.len());
    }
}
