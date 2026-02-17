//! Iterative Pipeline - Self-improving code generation
//!
//! This pipeline iterates until code review scores reach target threshold.
//!
//! Loop: Generate → Review → Fix → Review → Fix → ... until 10/10
//!
//! Run with:
//! ```bash
//! GEMINI_API_KEY=your-key cargo run --example iterative_pipeline
//! ```

use dasein_agentic_core::distributed::{Capability, Supervisor};
use std::time::Instant;

const TARGET_SCORE: u8 = 9; // Minimum acceptable score (out of 10)
const MAX_ITERATIONS: u8 = 3; // Maximum improvement iterations

#[derive(Debug, Clone)]
struct ReviewScores {
    architecture: u8,
    safety: u8,
    completeness: u8,
    quality: u8,
    issues: Vec<String>,
}

impl ReviewScores {
    fn average(&self) -> f32 {
        (self.architecture + self.safety + self.completeness + self.quality) as f32 / 4.0
    }

    fn all_above_target(&self) -> bool {
        self.architecture >= TARGET_SCORE
            && self.safety >= TARGET_SCORE
            && self.completeness >= TARGET_SCORE
            && self.quality >= TARGET_SCORE
    }

    fn min_score(&self) -> u8 {
        *[
            self.architecture,
            self.safety,
            self.completeness,
            self.quality,
        ]
        .iter()
        .min()
        .unwrap()
    }
}

fn parse_review_scores(review_text: &str) -> ReviewScores {
    let mut scores = ReviewScores {
        architecture: 5,
        safety: 5,
        completeness: 5,
        quality: 5,
        issues: Vec::new(),
    };

    for line in review_text.lines() {
        let line_lower = line.to_lowercase();

        // Parse scores like "ARCHITECTURE: 8/10" or "Architecture Score: 8"
        if line_lower.contains("architecture") {
            if let Some(score) = extract_score(line) {
                scores.architecture = score;
            }
        } else if line_lower.contains("safety") {
            if let Some(score) = extract_score(line) {
                scores.safety = score;
            }
        } else if line_lower.contains("completeness") {
            if let Some(score) = extract_score(line) {
                scores.completeness = score;
            }
        } else if line_lower.contains("quality") || line_lower.contains("code quality") {
            if let Some(score) = extract_score(line) {
                scores.quality = score;
            }
        }

        // Extract issues (lines starting with - or * that mention problems)
        if (line.trim().starts_with('-') || line.trim().starts_with('*'))
            && (line_lower.contains("issue")
                || line_lower.contains("problem")
                || line_lower.contains("should")
                || line_lower.contains("missing")
                || line_lower.contains("could")
                || line_lower.contains("consider"))
        {
            scores.issues.push(line.trim().to_string());
        }
    }

    scores
}

fn extract_score(line: &str) -> Option<u8> {
    // Look for patterns like "8/10", "8 /10", "(8)", ": 8"
    for word in line.split_whitespace() {
        // Handle "8/10" pattern
        if word.contains('/') {
            if let Some(num_str) = word.split('/').next() {
                if let Ok(n) = num_str
                    .trim_matches(|c: char| !c.is_numeric())
                    .parse::<u8>()
                {
                    if n <= 10 {
                        return Some(n);
                    }
                }
            }
        }
        // Handle "(8)" or just "8" after a colon
        if let Ok(n) = word.trim_matches(|c: char| !c.is_numeric()).parse::<u8>() {
            if n <= 10 && n > 0 {
                return Some(n);
            }
        }
    }
    None
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║           AGENTIC-RS ITERATIVE PIPELINE                              ║");
    println!("║           Self-Improving Code Generation                             ║");
    println!(
        "║           Target: {}/10 on all metrics                                ║",
        TARGET_SCORE
    );
    println!("╚══════════════════════════════════════════════════════════════════════╝");
    println!();

    if std::env::var("GEMINI_API_KEY")
        .unwrap_or_default()
        .is_empty()
    {
        println!("⚠ GEMINI_API_KEY not set");
        return Ok(());
    }

    // Create supervisor
    let supervisor = Supervisor::new("sup-iterative")
        .domain("code-generation")
        .executors(5)
        .validators(2)
        .llm_gemini("gemini-2.0-flash")
        .capability(Capability::CodeGeneration)
        .build_async()
        .await;

    println!(
        "▶ Supervisor created with {} executors",
        supervisor.pool_size().await
    );
    println!();

    // ========== OBJECTIVE ==========
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║  OBJECTIVE: Create a Rate Limiter in Rust                            ║");
    println!("║                                                                      ║");
    println!("║  Requirements:                                                       ║");
    println!("║  • Token bucket algorithm                                            ║");
    println!("║  • Thread-safe (can be shared across threads)                        ║");
    println!("║  • Async-compatible with tokio                                       ║");
    println!("║  • Configurable: rate, burst capacity, refill interval               ║");
    println!("║  • Methods: acquire(), try_acquire(), available_tokens()             ║");
    println!("║  • Comprehensive tests and documentation                             ║");
    println!("╚══════════════════════════════════════════════════════════════════════╝");
    println!();

    let base_prompt = r#"Create a production-quality Rate Limiter in Rust using the token bucket algorithm.

Requirements:
1. Token bucket algorithm with configurable:
   - rate (tokens per second)
   - burst capacity (max tokens)
   - refill interval

2. Thread-safe: must work with Arc<RateLimiter> shared across threads

3. Async-compatible with tokio:
   - async fn acquire(&self) -> waits until token available
   - fn try_acquire(&self) -> Option<()> non-blocking
   - fn available_tokens(&self) -> u32

4. Builder pattern for configuration:
   RateLimiter::builder()
       .rate(100)
       .burst(150)
       .build()

5. Full documentation with /// comments and examples
6. Comprehensive unit tests with #[tokio::test]
7. No TODO, FIXME, or unimplemented!()
8. Use proper error handling

Return ONLY Rust code, no markdown fences, no explanations."#;

    let mut current_code = String::new();
    let mut iteration = 0;
    let mut best_scores = ReviewScores {
        architecture: 0,
        safety: 0,
        completeness: 0,
        quality: 0,
        issues: Vec::new(),
    };

    let total_start = Instant::now();

    loop {
        iteration += 1;
        println!("═══════════════════════════════════════════════════════════════════════");
        println!("  ITERATION {} / {}", iteration, MAX_ITERATIONS);
        println!("═══════════════════════════════════════════════════════════════════════");
        println!();

        // ========== GENERATE OR FIX ==========
        let executor = supervisor.get_executor().await.expect("Need executor");

        let (prompt, phase_name) = if iteration == 1 {
            (base_prompt.to_string(), "Initial Generation")
        } else {
            let fix_prompt = format!(
                r#"Here is code that needs improvement:

```rust
{}
```

The code review found these issues:
{}

Current scores:
- Architecture: {}/10
- Safety: {}/10
- Completeness: {}/10
- Quality: {}/10

IMPROVE the code to fix ALL issues and get 10/10 on every metric.
Focus especially on the lowest scores.
Return the COMPLETE improved code, not just the changes.
Return ONLY Rust code, no markdown fences, no explanations."#,
                current_code,
                best_scores.issues.join("\n"),
                best_scores.architecture,
                best_scores.safety,
                best_scores.completeness,
                best_scores.quality,
            );
            (fix_prompt, "Improvement")
        };

        println!("▶ {} phase...", phase_name);
        let gen_start = Instant::now();

        let gen_result = executor
            .execute(
                "You are an expert Rust systems programmer. Write production-quality code.",
                &prompt,
            )
            .await?;

        current_code = gen_result.content.clone();
        println!(
            "  ✓ Generated {} lines in {:?} ({} tokens)",
            current_code.lines().count(),
            gen_start.elapsed(),
            gen_result.tokens_used
        );
        println!();

        // ========== CODE REVIEW ==========
        println!("▶ Code Review phase...");
        let reviewer = supervisor.get_executor().await.expect("Need executor");

        let review_prompt = format!(
            r#"Review this Rust Rate Limiter implementation:

```rust
{}
```

Score each category from 1-10 and explain briefly:

ARCHITECTURE: X/10
[brief explanation]

SAFETY: X/10
[brief explanation]

COMPLETENESS: X/10
[brief explanation]

CODE QUALITY: X/10
[brief explanation]

TOP ISSUES TO FIX:
- issue 1
- issue 2
- issue 3

Be strict but fair. Only give 10/10 if truly exceptional."#,
            &current_code[..current_code.len().min(6000)]
        );

        let review_result = reviewer
            .execute(
                "You are a senior Rust engineer. Be critical and specific in your review.",
                &review_prompt,
            )
            .await?;

        let scores = parse_review_scores(&review_result.content);
        println!(
            "  ✓ Review completed ({} tokens)",
            review_result.tokens_used
        );
        println!();

        // Display scores
        println!("┌─────────────────────────────────────────────────────────────────────┐");
        println!("│                      REVIEW SCORES                                  │");
        println!("├─────────────────────────────────────────────────────────────────────┤");
        println!(
            "│  Architecture:  {:>2}/10  {}                                      │",
            scores.architecture,
            score_bar(scores.architecture)
        );
        println!(
            "│  Safety:        {:>2}/10  {}                                      │",
            scores.safety,
            score_bar(scores.safety)
        );
        println!(
            "│  Completeness:  {:>2}/10  {}                                      │",
            scores.completeness,
            score_bar(scores.completeness)
        );
        println!(
            "│  Code Quality:  {:>2}/10  {}                                      │",
            scores.quality,
            score_bar(scores.quality)
        );
        println!("├─────────────────────────────────────────────────────────────────────┤");
        println!(
            "│  Average:       {:.1}/10                                            │",
            scores.average()
        );
        println!(
            "│  Minimum:       {:>2}/10                                            │",
            scores.min_score()
        );
        println!("└─────────────────────────────────────────────────────────────────────┘");
        println!();

        if !scores.issues.is_empty() {
            println!("  Issues identified:");
            for (i, issue) in scores.issues.iter().take(5).enumerate() {
                println!(
                    "    {}. {}",
                    i + 1,
                    issue.chars().take(70).collect::<String>()
                );
            }
            println!();
        }

        best_scores = scores.clone();

        // Check if target reached
        if scores.all_above_target() {
            println!("╔══════════════════════════════════════════════════════════════════════╗");
            println!(
                "║  ✓ TARGET REACHED! All scores >= {}/10                               ║",
                TARGET_SCORE
            );
            println!("╚══════════════════════════════════════════════════════════════════════╝");
            break;
        }

        if iteration >= MAX_ITERATIONS {
            println!("╔══════════════════════════════════════════════════════════════════════╗");
            println!("║  ⚠ MAX ITERATIONS REACHED                                            ║");
            println!(
                "║  Best average score: {:.1}/10                                         ║",
                scores.average()
            );
            println!("╚══════════════════════════════════════════════════════════════════════╝");
            break;
        }

        println!("  → Scores below target, iterating...");
        println!();
    }

    // ========== FINAL SUMMARY ==========
    println!();
    println!("╔══════════════════════════════════════════════════════════════════════╗");
    println!("║                        FINAL SUMMARY                                 ║");
    println!("╠══════════════════════════════════════════════════════════════════════╣");
    println!(
        "║  Total iterations:     {:>2}                                          ║",
        iteration
    );
    println!(
        "║  Total time:           {:>10?}                                  ║",
        total_start.elapsed()
    );
    println!(
        "║  Final code lines:     {:>4}                                         ║",
        current_code.lines().count()
    );
    println!(
        "║  Final average score:  {:.1}/10                                       ║",
        best_scores.average()
    );
    println!("╚══════════════════════════════════════════════════════════════════════╝");

    // Show final code excerpt
    println!();
    println!("▶ Final Code (first 40 lines):");
    println!("─────────────────────────────────────────────────────────────────────────");
    for line in current_code.lines().take(40) {
        println!("  {}", line);
    }
    if current_code.lines().count() > 40 {
        println!("  ... ({} more lines)", current_code.lines().count() - 40);
    }

    Ok(())
}

fn score_bar(score: u8) -> &'static str {
    match score {
        10 => "██████████",
        9 => "█████████░",
        8 => "████████░░",
        7 => "███████░░░",
        6 => "██████░░░░",
        5 => "█████░░░░░",
        4 => "████░░░░░░",
        3 => "███░░░░░░░",
        2 => "██░░░░░░░░",
        1 => "█░░░░░░░░░",
        _ => "░░░░░░░░░░",
    }
}
