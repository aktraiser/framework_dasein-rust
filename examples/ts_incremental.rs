//! TypeScript Incremental Generation - Build code piece by piece
//!
//! This example demonstrates the CORRECT approach:
//! 1. Decompose task into sub-tasks
//! 2. Generate each sub-task separately
//! 3. Validate after each generation
//! 4. Fix only the failing part, not everything
//!
//! Usage:
//! ```bash
//! GEMINI_API_KEY=xxx cargo run --example ts_incremental
//!
//! # With Firecracker:
//! FIRECRACKER_URL=http://65.108.230.227:8080 \
//! GEMINI_API_KEY=xxx cargo run --example ts_incremental --features remote
//! ```

use dasein_agentic_core::distributed::{
    repair_engine::SurgicalRepair, Executor, SandboxPipelineValidator, TaskDecomposer,
    ValidatorInput, ValidatorPipeline,
};
use dasein_agentic_sandbox::ProcessSandbox;
#[cfg(feature = "remote")]
use dasein_agentic_sandbox::RemoteSandbox;
use std::path::PathBuf;
use std::time::Instant;

const MAX_RETRIES_PER_SUBTASK: u32 = 3;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();

    println!("\n{}", "=".repeat(60));
    println!("  TYPESCRIPT INCREMENTAL GENERATION");
    println!("  Build & Validate piece by piece");
    println!("{}\n", "=".repeat(60));

    // === Task (simplified for incremental approach) ===
    let task = r#"Create a TypeScript event-driven state machine with:

1. Types:
   - State and Event as generic type parameters
   - Transition<S, E> with target, guard?, action?
   - TransitionMap<S, E> mapping state -> event -> Transition

2. StateMachine<S, E> class:
   - constructor(initialState: S, transitions: TransitionMap<S, E>)
   - send(event: E): Promise<S>
   - getState(): S
   - canTransition(event: E): boolean

3. Tests for:
   - Basic state transitions
   - Guards that block transitions
   - Invalid transition handling"#;

    // === Setup Executor ===
    let model = std::env::var("MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_string());
    let executor = Executor::new("ts-gen", "supervisor")
        .llm_gemini(&model)
        .build();
    println!("✓ Model: {}", model);

    // === Setup Validators ===
    // Two pipelines: one for compilation-only, one with tests
    #[cfg(feature = "remote")]
    let (compile_pipeline, test_pipeline) = {
        if let Ok(url) = std::env::var("FIRECRACKER_URL") {
            println!("✓ Firecracker: {}", url);
            let sandbox1 = RemoteSandbox::builder(&url).timeout_ms(60_000).build();
            let sandbox2 = RemoteSandbox::builder(&url).timeout_ms(60_000).build();
            (
                ValidatorPipeline::new().add(
                    SandboxPipelineValidator::new(sandbox1)
                        .workspace(PathBuf::from("/tmp/ts-incremental"))
                        .run_tests(false), // Compilation only
                ),
                ValidatorPipeline::new().add(
                    SandboxPipelineValidator::new(sandbox2)
                        .workspace(PathBuf::from("/tmp/ts-incremental"))
                        .run_tests(true), // With tests
                ),
            )
        } else {
            println!("✓ Using ProcessSandbox (local)");
            let sandbox1 = ProcessSandbox::new().with_timeout(60_000);
            let sandbox2 = ProcessSandbox::new().with_timeout(60_000);
            (
                ValidatorPipeline::new().add(
                    SandboxPipelineValidator::new(sandbox1)
                        .workspace(PathBuf::from("/tmp/ts-incremental"))
                        .run_tests(false),
                ),
                ValidatorPipeline::new().add(
                    SandboxPipelineValidator::new(sandbox2)
                        .workspace(PathBuf::from("/tmp/ts-incremental"))
                        .run_tests(true),
                ),
            )
        }
    };

    #[cfg(not(feature = "remote"))]
    let (compile_pipeline, test_pipeline) = {
        if std::env::var("FIRECRACKER_URL").is_ok() {
            println!("⚠ FIRECRACKER_URL set but 'remote' feature not enabled");
        }
        println!("✓ Using ProcessSandbox (local)");
        let sandbox1 = ProcessSandbox::new().with_timeout(60_000);
        let sandbox2 = ProcessSandbox::new().with_timeout(60_000);
        (
            ValidatorPipeline::new().add(
                SandboxPipelineValidator::new(sandbox1)
                    .workspace(PathBuf::from("/tmp/ts-incremental"))
                    .run_tests(false),
            ),
            ValidatorPipeline::new().add(
                SandboxPipelineValidator::new(sandbox2)
                    .workspace(PathBuf::from("/tmp/ts-incremental"))
                    .run_tests(true),
            ),
        )
    };

    // === Decompose Task ===
    let decomposer = TaskDecomposer::new("typescript");
    let subtasks = decomposer.decompose(task);

    println!("\n📋 Task decomposed into {} sub-tasks:", subtasks.len());
    for (i, st) in subtasks.iter().enumerate() {
        println!(
            "   {}. {} (depends on: {:?})",
            i + 1,
            st.name,
            st.depends_on
        );
    }

    // === Generate Incrementally ===
    let total_start = Instant::now();
    let mut accumulated_code = String::new();
    let mut all_passed = true;

    let system = r#"You are an expert TypeScript developer. Return ONLY valid TypeScript code.
No markdown, no explanations, no comments unless necessary.
CRITICAL: Template literals MUST use backticks: `text ${var}` NOT quotes."#;

    for (idx, subtask) in subtasks.iter().enumerate() {
        println!("\n{}", "─".repeat(60));
        println!(
            "📦 Sub-task {}/{}: {}",
            idx + 1,
            subtasks.len(),
            subtask.name
        );
        println!("{}", "─".repeat(60));

        // Use compile-only for stages 1-3, tests for stage 4
        let is_test_stage = subtask.id == "tests";
        let pipeline = if is_test_stage {
            &test_pipeline
        } else {
            &compile_pipeline
        };
        let validation_mode = if is_test_stage {
            "compile + tests"
        } else {
            "compile only"
        };

        let mut subtask_code: Option<String> = None;

        for attempt in 1..=MAX_RETRIES_PER_SUBTASK {
            println!(
                "\n  [Attempt {}/{}] ({})",
                attempt, MAX_RETRIES_PER_SUBTASK, validation_mode
            );

            // Build prompt with context from previous parts
            let prompt = if accumulated_code.is_empty() {
                subtask.prompt.clone()
            } else {
                format!(
                    "{}\n\n=== ALREADY GENERATED (do not repeat) ===\n```typescript\n{}\n```\n\n\
                    Generate ONLY the next part, assuming the above code exists.",
                    subtask.prompt, accumulated_code
                )
            };

            // Generate
            let gen_start = Instant::now();
            let result = executor.execute(system, &prompt).await?;
            println!(
                "  Generated {} chars in {}ms",
                result.content.len(),
                gen_start.elapsed().as_millis()
            );

            // Clean the generated code
            let new_code = clean_code(&result.content);

            // Combine with accumulated code for validation
            let combined = if accumulated_code.is_empty() {
                new_code.clone()
            } else {
                format!("{}\n\n{}", accumulated_code, new_code)
            };

            // Validate the combined code using appropriate pipeline
            println!("  Validating ({})...", validation_mode);
            let input = ValidatorInput::new(&combined, "typescript").with_task(task);
            let val_result = pipeline.validate(input).await;

            if val_result.passed {
                println!("  ✓ Validation passed!");
                subtask_code = Some(new_code);
                break;
            } else {
                let errors: Vec<String> = val_result
                    .results
                    .iter()
                    .flat_map(|r| r.errors.clone())
                    .collect();
                println!("  ✗ {} errors", errors.len());

                // Show first few errors
                for err in errors.iter().take(3) {
                    let first_line = err.lines().next().unwrap_or(err);
                    println!("    {}", first_line);
                }

                // Try surgical repair first
                let (fixed, fix_count) = SurgicalRepair::fix_template_literals(&new_code);
                if fix_count > 0 {
                    println!("  [SurgicalRepair] Fixed {} template literals", fix_count);

                    // Re-validate with surgical fix
                    let fixed_combined = if accumulated_code.is_empty() {
                        fixed.clone()
                    } else {
                        format!("{}\n\n{}", accumulated_code, fixed)
                    };

                    let input2 = ValidatorInput::new(&fixed_combined, "typescript").with_task(task);
                    let revalidate = pipeline.validate(input2).await;
                    if revalidate.passed {
                        println!("  ✓ Surgical fix passed!");
                        subtask_code = Some(fixed);
                        break;
                    }
                }

                // If last attempt, log it
                if attempt == MAX_RETRIES_PER_SUBTASK {
                    println!("  ⚠ Max retries reached for this sub-task");
                }
            }
        }

        // Add successful code to accumulated
        if let Some(code) = subtask_code {
            if accumulated_code.is_empty() {
                accumulated_code = code;
            } else {
                accumulated_code = format!("{}\n\n{}", accumulated_code, code);
            }
            println!(
                "  ✓ Sub-task completed. Total code: {} chars",
                accumulated_code.len()
            );
        } else {
            println!(
                "  ✗ Sub-task FAILED after {} attempts",
                MAX_RETRIES_PER_SUBTASK
            );
            all_passed = false;
            // Continue with what we have
        }
    }

    // === Final Validation ===
    println!("\n{}", "=".repeat(60));
    println!("FINAL VALIDATION");
    println!("{}", "=".repeat(60));

    let final_input = ValidatorInput::new(&accumulated_code, "typescript").with_task(task);
    let final_result = test_pipeline.validate(final_input).await;

    if final_result.passed {
        println!("✓ ALL TESTS PASSED!");
    } else {
        let errors: Vec<String> = final_result
            .results
            .iter()
            .flat_map(|r| r.errors.clone())
            .collect();
        println!("✗ Final validation failed with {} errors", errors.len());
        for err in errors.iter().take(5) {
            let first_line = err.lines().next().unwrap_or(err);
            println!("  {}", first_line);
        }
    }

    // === Summary ===
    println!("\n{}", "=".repeat(60));
    println!("SUMMARY");
    println!("{}", "=".repeat(60));
    println!("Total time: {}ms", total_start.elapsed().as_millis());
    println!("Sub-tasks: {}", subtasks.len());
    println!("Final code: {} chars", accumulated_code.len());
    println!(
        "Status: {}",
        if all_passed && final_result.passed {
            "SUCCESS"
        } else {
            "PARTIAL"
        }
    );

    // === Output Code ===
    println!("\n{}", "=".repeat(60));
    println!("GENERATED CODE");
    println!("{}", "=".repeat(60));
    println!("{}", accumulated_code);

    Ok(())
}

/// Clean generated code (remove markdown, etc.)
fn clean_code(content: &str) -> String {
    let mut result = content.to_string();

    // Remove markdown code blocks
    if result.contains("```typescript") {
        if let Some(start) = result.find("```typescript") {
            if let Some(end) = result[start + 13..].find("```") {
                result = result[start + 13..start + 13 + end].to_string();
            }
        }
    } else if result.contains("```ts") {
        if let Some(start) = result.find("```ts") {
            if let Some(end) = result[start + 5..].find("```") {
                result = result[start + 5..start + 5 + end].to_string();
            }
        }
    } else if result.contains("```") {
        // Generic code block
        let parts: Vec<&str> = result.split("```").collect();
        if parts.len() >= 2 {
            result = parts[1].to_string();
            // Remove language identifier if present
            if let Some(newline) = result.find('\n') {
                let first_line = &result[..newline];
                if first_line
                    .chars()
                    .all(|c| c.is_alphanumeric() || c.is_whitespace())
                {
                    result = result[newline + 1..].to_string();
                }
            }
        }
    }

    result.trim().to_string()
}
