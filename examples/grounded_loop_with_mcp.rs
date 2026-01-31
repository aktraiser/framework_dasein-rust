//! Grounded Validation Loop with MCP Code Mode
//!
//! This example combines:
//! - Grounded validation (real compilation + tests)
//! - MCP Code Mode (agent writes code to query documentation)
//!
//! The flow:
//! 1. Executor writes TypeScript to query Context7 for documentation
//! 2. Code runs in sandbox, documentation is retrieved
//! 3. Executor generates Rust code using that documentation
//! 4. SandboxValidator validates with real `cargo check` + `cargo test`
//! 5. If failed, retry with real compiler errors
//!
//! This demonstrates how agents can efficiently use external knowledge
//! while maintaining ground-truth code validation.
//!
//! Run with: CONTEXT7_API_KEY=xxx cargo run --example grounded_loop_with_mcp

use agentic_core::distributed::{Executor, SandboxValidator, SandboxValidationResult};
use agentic_mcp::{MCPConfig, MCPServerConfig, MCPClientPool};
use agentic_sandbox::ProcessSandbox;
use std::path::{Path, PathBuf};
use std::time::Instant;

const MAX_ITERATIONS: u32 = 6;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();

    println!("\n{}", "=".repeat(70));
    println!("  GROUNDED LOOP + MCP CODE MODE");
    println!("  Query docs via Code Mode + Validate code with real compilation");
    println!("{}\n", "=".repeat(70));

    // === Phase 1: Setup MCP for documentation lookup ===
    println!("[SETUP] Configuring MCP Code Mode...\n");

    let workspace = Path::new("/tmp/grounded-mcp-workspace");
    std::fs::create_dir_all(workspace)?;

    // Try to connect to Context7, fallback to mock
    let has_real_mcp = setup_mcp_workspace(workspace).await;
    if has_real_mcp {
        println!("     Connected to Context7 for documentation\n");
    } else {
        println!("     Using mock documentation (set CONTEXT7_API_KEY for real docs)\n");
    }

    // === Phase 2: Setup Executor and Validator ===
    let executor = Executor::new("exe-grounded-mcp", "supervisor-main")
        .llm_gemini("gemini-2.0-flash")
        .build();

    let sandbox = ProcessSandbox::new().with_timeout(120_000);
    let validator = SandboxValidator::new(sandbox.clone())
        .workspace(PathBuf::from("/tmp/grounded-validation"))
        .run_tests(true);

    // === Phase 3: Define the task ===
    // A task that benefits from documentation lookup
    let task = r#"Implement a Rust async task queue using tokio.

Requirements:
1. TaskQueue<T> struct where T: Send + 'static
2. Methods:
   - new(max_concurrent: usize) -> Self
   - spawn(&self, task: impl Future<Output = T> + Send + 'static) -> JoinHandle<T>
   - shutdown(&self) - gracefully stop accepting new tasks
   - wait_all(&self) -> Vec<T> - wait for all tasks to complete
3. Should limit concurrent task execution to max_concurrent
4. Use tokio::sync primitives (Semaphore, etc.)

Include tests for:
- Concurrent execution limit
- Graceful shutdown
- Multiple tasks completion"#;

    println!("Task: Implement async TaskQueue with tokio\n");

    // === Phase 4: Query documentation (Code Mode) ===
    println!("[1/4] Querying documentation via Code Mode...\n");

    let doc_context = query_documentation(&executor, workspace).await?;
    println!("     Retrieved {} chars of documentation context\n", doc_context.len());

    // === Phase 5: Grounded Loop with documentation context ===
    println!("[2/4] Starting grounded validation loop...\n");

    let system_prompt = format!(r#"You are an expert Rust developer specializing in async programming.

RELEVANT DOCUMENTATION:
{}

Write clean, idiomatic Rust code using tokio.
Include comprehensive tests in a #[cfg(test)] module.
The code must compile and all tests must pass.

IMPORTANT: Return ONLY the Rust code, no markdown, no explanations.
The code should be a complete lib.rs file."#, doc_context);

    let total_start = Instant::now();
    let mut iteration = 0;
    let mut current_prompt = task.to_string();

    loop {
        iteration += 1;
        println!("\n--- Iteration {} / {} ---", iteration, MAX_ITERATIONS);

        // Generate code
        let gen_start = Instant::now();
        let result = executor.execute(&system_prompt, &current_prompt).await?;
        println!("Generated {} chars in {}ms", result.content.len(), gen_start.elapsed().as_millis());

        let code = clean_code(&result.content);

        // Validate with real compilation
        println!("Validating with cargo check + cargo test...");
        let val_start = Instant::now();

        match validator.validate_rust_code(&code).await {
            Ok(validation) => {
                print_validation_summary(&validation);
                println!("Validation: {}ms", val_start.elapsed().as_millis());

                if validation.passed {
                    println!("\n{}", "=".repeat(70));
                    println!("SUCCESS after {} iterations ({}ms total)",
                        iteration, total_start.elapsed().as_millis());
                    println!("{}\n", "=".repeat(70));
                    println!("{}", code);
                    return Ok(());
                }

                if iteration >= MAX_ITERATIONS {
                    println!("\nMax iterations reached. Last code:\n{}", code);
                    return Err("Max iterations".into());
                }

                // Build retry prompt with real errors
                current_prompt = build_retry_prompt(task, &code, &validation);
            }
            Err(e) => {
                println!("Sandbox error: {}", e);
                if iteration >= MAX_ITERATIONS {
                    return Err(e.into());
                }
                current_prompt = format!("{}\n\nPrevious attempt failed. Try simpler code.", task);
            }
        }
    }
}

/// Setup MCP workspace, returns true if real MCP is available
async fn setup_mcp_workspace(workspace: &Path) -> bool {
    let api_key = std::env::var("CONTEXT7_API_KEY").ok();

    if let Some(key) = api_key {
        // Try real Context7
        let config = MCPConfig::new()
            .add_server(
                "context7",
                MCPServerConfig::http("https://mcp.context7.com/mcp")
                    .header("CONTEXT7_API_KEY", &key)
                    .timeout(30000),
            );

        if let Ok(pool) = MCPClientPool::new(config).await {
            if pool.generate_typescript(workspace).await.is_ok() {
                return true;
            }
        }
    }

    // Create mock
    create_mock_mcp_tools(workspace).unwrap_or(());
    false
}

/// Create mock MCP tool files
fn create_mock_mcp_tools(workspace: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let server_dir = workspace.join("servers/context7");
    std::fs::create_dir_all(&server_dir)?;

    // Minimal mock - we provide better docs directly in query_documentation
    std::fs::write(server_dir.join("queryDocs.ts"), "// mock")?;
    std::fs::write(server_dir.join("index.ts"), "// mock\n")?;

    Ok(())
}

/// Query documentation using Code Mode
async fn query_documentation(
    executor: &Executor,
    _workspace: &Path,
) -> Result<String, Box<dyn std::error::Error>> {
    // In a real implementation, this would:
    // 1. Have the executor write TypeScript code
    // 2. Execute it in sandbox
    // 3. Return the console output

    // For this example, we simulate with a direct approach
    let system = r#"You are querying documentation for tokio async primitives.
Based on your knowledge, provide relevant documentation about:
- tokio::sync::Semaphore
- tokio::task::JoinSet or JoinHandle
- Patterns for limiting concurrent tasks

Format as a brief reference (max 500 chars)."#;

    let query = "Give me tokio documentation for building a concurrent task limiter";

    let result = executor.execute(system, query).await?;

    Ok(result.content)
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

fn build_retry_prompt(task: &str, code: &str, validation: &SandboxValidationResult) -> String {
    let mut prompt = String::new();

    prompt.push_str("=== TASK ===\n");
    prompt.push_str(task);
    prompt.push_str("\n\n=== YOUR CODE ===\n```rust\n");
    prompt.push_str(code);
    prompt.push_str("\n```\n\n");

    if !validation.compiles {
        prompt.push_str("=== COMPILATION ERRORS ===\n```\n");
        for e in &validation.compiler_errors {
            prompt.push_str(e);
            prompt.push('\n');
        }
        prompt.push_str("```\n");
    } else if !validation.tests_passed {
        prompt.push_str(&format!("=== TESTS FAILED ({}/{}) ===\n```\n",
            validation.tests_ok, validation.test_count));
        for e in &validation.test_errors {
            prompt.push_str(e);
            prompt.push('\n');
        }
        prompt.push_str("```\n");
    }

    prompt.push_str("\nFix the errors and return ONLY the corrected Rust code.");
    prompt
}

fn print_validation_summary(v: &SandboxValidationResult) {
    if v.compiles {
        print!("  Compile: OK");
    } else {
        println!("  Compile: FAIL ({} errors)", v.compiler_errors.len());
        // Show first 3 errors for debugging
        for (i, err) in v.compiler_errors.iter().take(3).enumerate() {
            println!("    Error {}: {}", i + 1, err.lines().next().unwrap_or(err));
        }
    }

    if v.compiles {
        if v.tests_passed {
            println!(" | Tests: OK ({}/{})", v.tests_ok, v.test_count);
        } else {
            println!(" | Tests: FAIL ({}/{})", v.tests_ok, v.test_count);
            for err in v.test_errors.iter().take(2) {
                println!("    {}", err.lines().next().unwrap_or(err));
            }
        }
    } else {
        println!();
    }

    // Show feedback/recommendations if available
    if let Some(feedback) = &v.feedback {
        println!("  Feedback: {}", feedback.lines().next().unwrap_or(""));
    }
}
