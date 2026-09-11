//! Firecracker Demo - Rust Code validation in microVM
//!
//! Demonstrates:
//! - LLM generates Rust code
//! - Code is compiled & tested in Firecracker microVM (remote)
//! - Iterative correction loop until compilation + tests pass
//!
//! Run with: cargo run --example firecracker_demo

use dasein_agentic_llm::{GeminiAdapter, LLMAdapter, LLMMessage};
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::time::Instant;

const FIRECRACKER_URL: &str = "http://65.108.230.227:8080";
const MAX_ITERATIONS: u32 = 8;

#[derive(Serialize)]
struct ExecuteRequest {
    code: String,
    timeout_ms: u32,
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct ExecuteResponse {
    success: bool,
    #[serde(default)]
    exit_code: i32,
    #[serde(default)]
    stdout: String,
    #[serde(default)]
    stderr: String,
    #[serde(default)]
    execution_time_ms: u64,
    #[serde(default)]
    error: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "═".repeat(70));
    println!("  FIRECRACKER DEMO - Rust Compilation in microVM");
    println!("{}\n", "═".repeat(70));

    let client = reqwest::Client::new();
    let api_key = std::env::var("GEMINI_API_KEY")?;
    let llm = GeminiAdapter::new(&api_key, "gemini-2.0-flash");

    // Health check
    println!("[1/3] Checking Firecracker microVM...\n");
    let health: serde_json::Value = client
        .get(format!("{}/health", FIRECRACKER_URL))
        .send()
        .await?
        .json()
        .await?;
    println!("      Status: {}", health["status"]);
    println!("      Firecracker: {}", health["firecracker_available"]);
    println!("      KVM: {}", health["kvm_available"]);

    // Task - Log Parser (realistic CI/CD task)
    // Simule un agent qui génère un outil de parsing pour pipeline
    let task = r#"Write a Rust log parser with this function signature:

fn parse_logs<R: std::io::BufRead>(reader: R) -> String

INPUT FORMAT (one log line per line):
[timestamp] LEVEL message key=value ...

Example:
[2024-01-15 10:23:45] INFO deploy started service=api
[2024-01-15 10:23:48] ERROR connection_failed service=db

OUTPUT: JSON string with these fields:
- total_lines: number of lines parsed
- errors: count of ERROR lines
- info: count of INFO lines
- services: array of unique service names (from service=XXX)
- has_errors: boolean

Example output:
{"total_lines":2,"errors":1,"info":1,"services":["api","db"],"has_errors":true}

Requirements:
- Parse each line, extract level (INFO/ERROR) and service=XXX if present
- Build JSON manually (no serde), single line output
- Use only std library

Include main() that reads stdin and prints result.

Return ONLY Rust code, no markdown."#;

    // HIDDEN TESTS - Injection de stdin simulé
    // On remplace le main() du LLM par le nôtre qui teste avec des inputs connus
    // Note: use r##"..."## to allow r#"..."# inside
    let hidden_tests = r##"
use std::io::Cursor;

fn main() {
    // Test input - simulated deployment log
    let test_input = r#"[2024-01-15 10:00:00] INFO deploy started service=api version=2.0
[2024-01-15 10:00:01] INFO health_check ok service=api latency_ms=12
[2024-01-15 10:00:02] ERROR timeout service=cache retry=1
[2024-01-15 10:00:03] ERROR connection_refused service=db
[2024-01-15 10:00:04] INFO deploy completed service=api duration=3s
"#;

    // Redirect stdin to our test input
    let cursor = Cursor::new(test_input);
    let result = parse_logs(cursor);

    // Show actual output for debugging
    println!("YOUR OUTPUT: {}", result);
    println!("EXPECTED FORMAT: {{\"total_lines\":5,\"errors\":2,\"info\":3,\"services\":[...],\"has_errors\":true}}");

    // Validate output
    assert!(result.contains("\"total_lines\":5"), "FAIL: expected total_lines=5");
    assert!(result.contains("\"errors\":2"), "FAIL: expected errors=2 (found 2 ERROR lines)");
    assert!(result.contains("\"info\":3"), "FAIL: expected info=3 (found 3 INFO lines)");
    assert!(result.contains("\"has_errors\":true"), "FAIL: expected has_errors=true");

    // Check services extracted (order may vary)
    assert!(result.contains("api"), "FAIL: missing service 'api'");
    assert!(result.contains("cache"), "FAIL: missing service 'cache'");
    assert!(result.contains("db"), "FAIL: missing service 'db'");

    println!("OK");
}
"##;

    let system_prompt = "Write Rust. Use only std library. No external crates. No markdown fences.";

    println!("\n[2/3] Task: Log Parser for CI/CD Pipeline\n");

    // Grounded loop
    println!("[3/3] Running validation loop...\n");
    let total_start = Instant::now();
    let mut iteration = 0;
    let mut current_prompt = task.to_string();

    loop {
        iteration += 1;
        println!("--- Iteration {} / {} ---", iteration, MAX_ITERATIONS);

        // Generate code with Gemini
        let gen_start = Instant::now();
        let messages = vec![
            LLMMessage::system(system_prompt),
            LLMMessage::user(&current_prompt),
        ];
        let response = llm.generate(&messages).await?;
        let code = clean_code(&response.content);
        println!(
            "Generated {} chars in {}ms",
            code.len(),
            gen_start.elapsed().as_millis()
        );

        // Execute in Firecracker with hidden tests injection
        let exec_start = Instant::now();
        let result = execute_in_firecracker(&client, &code, hidden_tests).await?;
        println!(
            "Firecracker: {}ms (VM exec: {}ms)",
            exec_start.elapsed().as_millis(),
            result.execution_time_ms
        );

        // Check result - strict validation
        let has_error = result.error.is_some() || result.exit_code != 0;
        let stdout_ok = result.stdout.lines().any(|line| line.trim() == "OK");

        if !has_error && stdout_ok {
            println!("  Status: OK");
            println!("\n{}", "═".repeat(70));
            println!(
                "  SUCCESS after {} iteration(s) ({}ms total)",
                iteration,
                total_start.elapsed().as_millis()
            );
            println!("{}\n", "═".repeat(70));
            println!("Output: {}", result.stdout.trim());
            println!("\n--- Generated Code ---\n{}", code);
            return Ok(());
        } else if let Some(ref err) = result.error {
            println!("  Status: ERROR - {}", err);
        } else {
            println!("  Status: FAIL (exit: {})", result.exit_code);
            // Show stderr
            if !result.stderr.is_empty() {
                for line in result.stderr.lines().take(15) {
                    println!("    │ {}", line);
                }
            }
            // Show stdout
            if !result.stdout.is_empty() {
                for line in result.stdout.lines().take(15) {
                    println!("    │ {}", line);
                }
            }
            if result.stderr.is_empty() && result.stdout.is_empty() {
                println!("    │ (no output)");
            }
        }

        if iteration >= MAX_ITERATIONS {
            println!("\nMax iterations reached.");
            return Err("Max iterations".into());
        }

        // === LLM DEBUGGER: Analyze error and provide specific hint ===
        let debug_hint = get_debug_hint(&llm, &code, &result.stdout, &result.stderr).await?;
        println!("  Debug hint: {}", debug_hint);

        // Build retry prompt with enriched feedback
        current_prompt = format!(
            "=== TASK ===\n{}\n\n=== YOUR CODE ===\n{}\n\n=== ERROR OUTPUT ===\n{}\n{}\n\n=== DEBUGGER ANALYSIS ===\n{}\n\nFix the bug identified above. Return ONLY the corrected Rust code, no markdown.",
            task, code, result.stdout, result.stderr, debug_hint
        );
        println!();
    }
}

/// LLM Debugger: Analyzes the error and provides a specific hint
async fn get_debug_hint(
    llm: &GeminiAdapter,
    code: &str,
    stdout: &str,
    stderr: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let debug_prompt = format!(
        r#"You are a Rust debugging assistant. Analyze this failed code and identify the SPECIFIC bug.

CODE:
```rust
{}
```

OUTPUT:
{}
{}

Look at:
1. What the code ACTUALLY outputs vs what was EXPECTED
2. Find the EXACT line/function causing the bug
3. Explain WHY it fails (e.g., wrong parsing logic, missing condition, etc.)

Reply in 1-2 sentences with the specific fix needed. Be precise about which function or line is wrong."#,
        code, stdout, stderr
    );

    let messages = vec![
        LLMMessage::system("You are a concise Rust debugger. Identify bugs precisely."),
        LLMMessage::user(&debug_prompt),
    ];

    let response = llm.generate(&messages).await?;
    Ok(response
        .content
        .lines()
        .take(3)
        .collect::<Vec<_>>()
        .join(" "))
}

async fn execute_in_firecracker(
    client: &reqwest::Client,
    code: &str,
    hidden_tests: &str,
) -> Result<ExecuteResponse, Box<dyn std::error::Error>> {
    // Strip LLM's main() and inject our hidden tests
    let final_code = inject_hidden_tests(code, hidden_tests);

    // Step 1: Write code and start compilation in background
    // Random delimiter to prevent injection
    let delimiter = random_delimiter();
    let write_and_compile = format!(
        r#"cat << '{delim}' > /tmp/main.rs
{code}
{delim}
rm -f /tmp/build.done /tmp/build.log /tmp/main
setsid sh -c 'rustc /tmp/main.rs -o /tmp/main > /tmp/build.log 2>&1; echo $? > /tmp/build.done' &
echo "compilation_started""#,
        delim = delimiter,
        code = final_code
    );

    let req = ExecuteRequest {
        code: write_and_compile,
        timeout_ms: 5000,
    };

    let start_resp: ExecuteResponse = client
        .post(format!("{}/execute", FIRECRACKER_URL))
        .json(&req)
        .send()
        .await?
        .json()
        .await?;

    if !start_resp.stdout.contains("compilation_started") {
        return Ok(ExecuteResponse {
            success: false,
            exit_code: 1,
            stdout: start_resp.stdout,
            stderr: "Failed to start compilation".to_string(),
            execution_time_ms: 0,
            error: Some("Failed to start compilation".to_string()),
        });
    }

    // Step 2: Poll for compilation completion (max 120s)
    let start = std::time::Instant::now();
    let max_wait = std::time::Duration::from_secs(120);

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        let check_req = ExecuteRequest {
            code: "cat /tmp/build.done 2>/dev/null || echo pending".to_string(),
            timeout_ms: 5000,
        };

        let check_resp: ExecuteResponse = client
            .post(format!("{}/execute", FIRECRACKER_URL))
            .json(&check_req)
            .send()
            .await?
            .json()
            .await?;

        let status = check_resp.stdout.trim();
        if status != "pending" {
            // Compilation finished
            break;
        }

        if start.elapsed() > max_wait {
            return Ok(ExecuteResponse {
                success: false,
                exit_code: 1,
                stdout: String::new(),
                stderr: "Compilation timeout (120s)".to_string(),
                execution_time_ms: start.elapsed().as_millis() as u64,
                error: Some("timeout".to_string()),
            });
        }

        print!(".");
        use std::io::Write;
        std::io::stdout().flush().ok();
    }

    // Step 3: Get compilation result
    let result_req = ExecuteRequest {
        code: "cat /tmp/build.done; echo '---LOG---'; cat /tmp/build.log; echo '---END---'"
            .to_string(),
        timeout_ms: 5000,
    };

    let result_resp: ExecuteResponse = client
        .post(format!("{}/execute", FIRECRACKER_URL))
        .json(&result_req)
        .send()
        .await?
        .json()
        .await?;

    let output = result_resp.stdout;
    let exit_code: i32 = output
        .lines()
        .next()
        .unwrap_or("1")
        .trim()
        .parse()
        .unwrap_or(1);

    let log = output
        .split("---LOG---")
        .nth(1)
        .unwrap_or("")
        .split("---END---")
        .next()
        .unwrap_or("")
        .trim();

    if exit_code != 0 {
        return Ok(ExecuteResponse {
            success: false,
            exit_code,
            stdout: log.to_string(),
            stderr: String::new(),
            execution_time_ms: start.elapsed().as_millis() as u64,
            error: None,
        });
    }

    // Step 4: Run the compiled binary
    let run_req = ExecuteRequest {
        code: "/tmp/main 2>&1".to_string(),
        timeout_ms: 10000,
    };

    let run_resp: ExecuteResponse = client
        .post(format!("{}/execute", FIRECRACKER_URL))
        .json(&run_req)
        .send()
        .await?
        .json()
        .await?;

    Ok(ExecuteResponse {
        success: run_resp.exit_code == 0,
        exit_code: run_resp.exit_code,
        stdout: run_resp.stdout,
        stderr: run_resp.stderr,
        execution_time_ms: start.elapsed().as_millis() as u64,
        error: run_resp.error,
    })
}

fn random_delimiter() -> String {
    let mut rng = rand::rng();
    let suffix: u32 = rng.random_range(100000..999999);
    format!("RUSTCODE_{}", suffix)
}

/// Strip LLM's main() function and inject our hidden tests
fn inject_hidden_tests(code: &str, hidden_tests: &str) -> String {
    // Find and remove the LLM's main() function
    // We look for "fn main()" and remove everything from there to the matching closing brace

    let mut result = String::new();
    let mut chars = code.chars().peekable();
    let mut in_main = false;
    let mut brace_depth = 0;
    let mut buffer = String::new();

    while let Some(c) = chars.next() {
        buffer.push(c);

        // Check if we're at "fn main()"
        if buffer.ends_with("fn main()") && !in_main {
            // Remove "fn main()" from result
            for _ in 0.."fn main()".len() {
                buffer.pop();
            }
            result.push_str(&buffer);
            buffer.clear();

            // Skip until we find the opening brace
            for c in chars.by_ref() {
                if c == '{' {
                    in_main = true;
                    brace_depth = 1;
                    break;
                }
            }
            continue;
        }

        if in_main {
            buffer.pop(); // Don't add to buffer while in main

            if c == '{' {
                brace_depth += 1;
            } else if c == '}' {
                brace_depth -= 1;
                if brace_depth == 0 {
                    in_main = false;
                }
            }
        }
    }

    result.push_str(&buffer);

    // Append hidden tests
    result.push_str(hidden_tests);
    result
}

fn clean_code(content: &str) -> String {
    let mut code = content.to_string();

    // Try rust-specific markers first
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
