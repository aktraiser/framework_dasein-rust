//! Real LLM Test - Actual API calls with complex objective
//!
//! Run with:
//! ```bash
//! GEMINI_API_KEY=your-key cargo run --example llm_real_test
//! ```

use agentic_core::distributed::{Capability, Supervisor, ValidationRule};
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           AGENTIC-RS REAL LLM TEST                           ║");
    println!("║           Complex Objective: Code Generation Pipeline        ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Check API key
    let api_key = std::env::var("GEMINI_API_KEY");
    if api_key.is_err() || api_key.as_ref().unwrap().is_empty() {
        println!("⚠ GEMINI_API_KEY not set. Set it to run real LLM tests:");
        println!("  export GEMINI_API_KEY=your-api-key");
        println!();
        println!("Running in MOCK mode for demonstration...");
        run_mock_demo().await;
        return Ok(());
    }

    println!("▶ API Key found, running REAL LLM calls...");
    println!();

    // ========== CREATE SUPERVISOR ==========
    println!("▶ Creating supervisor with 5 executors and 2 validators...");
    let start = Instant::now();

    let supervisor = Supervisor::new("sup-llm-test")
        .domain("code-generation")
        .executors(5)
        .validators(2)
        .llm_gemini("gemini-2.0-flash")
        .sandbox_process()
        .capability(Capability::CodeGeneration)
        .capability(Capability::CodeExecution)
        .build_async()
        .await;

    println!("  ✓ Created in {:?}", start.elapsed());
    println!("  └── Pool size: {}", supervisor.pool_size().await);
    println!();

    // ========== COMPLEX OBJECTIVE ==========
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  OBJECTIVE: Generate a Rust function that:                   ║");
    println!("║  1. Checks if a number is prime                              ║");
    println!("║  2. Finds all primes up to N                                 ║");
    println!("║  3. Includes proper documentation                            ║");
    println!("║  4. Has unit tests                                           ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Get an executor
    let executor = supervisor
        .get_executor()
        .await
        .expect("Should have idle executor");

    println!("▶ Executor {} executing task...", executor.id);
    let start = Instant::now();

    let system_prompt = r#"You are an expert Rust developer. Generate clean, idiomatic Rust code.
Always include:
- Proper documentation with /// comments
- Unit tests with #[test]
- No TODO or FIXME comments
- No secrets or API keys"#;

    let user_prompt = r#"Create a Rust module for prime numbers with:
1. A function `is_prime(n: u64) -> bool` that checks if a number is prime
2. A function `primes_up_to(n: u64) -> Vec<u64>` that returns all primes up to n
3. Use the Sieve of Eratosthenes algorithm for efficiency
4. Include comprehensive unit tests

Return only the Rust code, no explanations."#;

    let result = executor.execute(system_prompt, user_prompt).await;

    match result {
        Ok(exec_result) => {
            println!("  ✓ Execution completed in {:?}", start.elapsed());
            println!("  ├── Task ID: {}", exec_result.task_id);
            println!("  ├── Tokens used: {}", exec_result.tokens_used);
            println!("  ├── Model: {}", exec_result.model);
            println!("  └── Duration: {}ms", exec_result.duration_ms);
            println!();

            // ========== VALIDATION ==========
            println!("▶ Validating output with 2 validators...");

            // Validator 0: Basic checks
            let result0 = supervisor.validate_with(0, &exec_result.content, 0);
            println!(
                "  Validator 0: {} (score: {})",
                if result0.passed {
                    "✓ PASS"
                } else {
                    "✗ FAIL"
                },
                result0.score
            );

            // Validator 1: Code quality checks
            let result1 = supervisor.validate_with(1, &exec_result.content, 0);
            println!(
                "  Validator 1: {} (score: {})",
                if result1.passed {
                    "✓ PASS"
                } else {
                    "✗ FAIL"
                },
                result1.score
            );

            // Manual checks for complex requirements
            let has_is_prime = exec_result.content.contains("fn is_prime");
            let has_primes_up_to = exec_result.content.contains("fn primes_up_to");
            let has_tests =
                exec_result.content.contains("#[test]") || exec_result.content.contains("fn test_");
            let has_docs = exec_result.content.contains("///");
            let no_todos =
                !exec_result.content.contains("TODO") && !exec_result.content.contains("FIXME");

            println!();
            println!("▶ Objective Checks:");
            println!(
                "  {} has is_prime function",
                if has_is_prime { "✓" } else { "✗" }
            );
            println!(
                "  {} has primes_up_to function",
                if has_primes_up_to { "✓" } else { "✗" }
            );
            println!("  {} has unit tests", if has_tests { "✓" } else { "✗" });
            println!("  {} has documentation", if has_docs { "✓" } else { "✗" });
            println!("  {} no TODOs/FIXMEs", if no_todos { "✓" } else { "✗" });
            println!();

            let all_passed = has_is_prime && has_primes_up_to && has_tests && has_docs && no_todos;

            // ========== GENERATED CODE ==========
            println!("╔══════════════════════════════════════════════════════════════╗");
            println!("║                    GENERATED CODE                            ║");
            println!("╚══════════════════════════════════════════════════════════════╝");

            // Print first 50 lines of generated code
            let lines: Vec<&str> = exec_result.content.lines().take(50).collect();
            for line in &lines {
                println!("  {}", line);
            }
            if exec_result.content.lines().count() > 50 {
                println!(
                    "  ... ({} more lines)",
                    exec_result.content.lines().count() - 50
                );
            }
            println!();

            // ========== FINAL RESULT ==========
            println!("╔══════════════════════════════════════════════════════════════╗");
            if all_passed {
                println!("║  ✓ SUCCESS - All objectives met!                             ║");
            } else {
                println!("║  ✗ PARTIAL - Some objectives not met                         ║");
            }
            println!("╚══════════════════════════════════════════════════════════════╝");
        }
        Err(e) => {
            println!("  ✗ Execution failed: {}", e);
        }
    }

    Ok(())
}

async fn run_mock_demo() {
    println!();
    println!("▶ [MOCK] Creating supervisor...");

    let supervisor = Supervisor::new("sup-mock")
        .domain("code-generation")
        .executors(5)
        .validators(2)
        .llm_gemini("gemini-2.0-flash")
        .build_async()
        .await;

    println!(
        "  ✓ Supervisor created with {} executors",
        supervisor.pool_size().await
    );

    // Simulate what would happen with real LLM
    let mock_output = r#"/// Checks if a number is prime.
///
/// # Examples
///
/// ```
/// assert!(is_prime(7));
/// assert!(!is_prime(4));
/// ```
pub fn is_prime(n: u64) -> bool {
    if n < 2 {
        return false;
    }
    if n == 2 {
        return true;
    }
    if n % 2 == 0 {
        return false;
    }
    let sqrt = (n as f64).sqrt() as u64;
    for i in (3..=sqrt).step_by(2) {
        if n % i == 0 {
            return false;
        }
    }
    true
}

/// Returns all prime numbers up to n using the Sieve of Eratosthenes.
pub fn primes_up_to(n: u64) -> Vec<u64> {
    if n < 2 {
        return vec![];
    }
    let mut sieve = vec![true; (n + 1) as usize];
    sieve[0] = false;
    sieve[1] = false;

    for i in 2..=((n as f64).sqrt() as usize) {
        if sieve[i] {
            for j in ((i * i)..=(n as usize)).step_by(i) {
                sieve[j] = false;
            }
        }
    }

    sieve.iter()
        .enumerate()
        .filter(|(_, &is_prime)| is_prime)
        .map(|(i, _)| i as u64)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_prime() {
        assert!(!is_prime(0));
        assert!(!is_prime(1));
        assert!(is_prime(2));
        assert!(is_prime(3));
        assert!(!is_prime(4));
        assert!(is_prime(5));
        assert!(is_prime(97));
    }

    #[test]
    fn test_primes_up_to() {
        assert_eq!(primes_up_to(10), vec![2, 3, 5, 7]);
        assert_eq!(primes_up_to(1), vec![]);
        assert_eq!(primes_up_to(2), vec![2]);
    }
}
"#;

    println!();
    println!("▶ [MOCK] Validating output...");

    let result = supervisor.validate(&mock_output, 0);
    println!(
        "  {} Validation (score: {})",
        if result.passed {
            "✓ PASS"
        } else {
            "✗ FAIL"
        },
        result.score
    );

    println!();
    println!("To run with real LLM calls:");
    println!("  export GEMINI_API_KEY=your-api-key");
    println!("  cargo run --example llm_real_test");
}
