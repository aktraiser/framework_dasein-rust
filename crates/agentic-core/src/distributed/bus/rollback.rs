//! Selective Rollback - Track and restore best code attempts.
//!
//! Instead of letting the LLM wander, this module:
//! 1. Tracks all attempts with their validation scores
//! 2. Identifies the "best" attempt (compiles + fewest test failures)
//! 3. Enables rollback when things get worse
//! 4. Provides targeted diff context for fixes

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Score for ranking attempts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AttemptScore {
    /// Does it compile? (most important)
    pub compiles: bool,
    /// Number of tests that passed
    pub tests_passed: usize,
    /// Number of tests that failed
    pub tests_failed: usize,
    /// Number of compilation errors (if any)
    pub compile_errors: usize,
    /// Total errors (for comparison)
    pub total_errors: usize,
}

impl AttemptScore {
    pub fn from_errors(errors: &[String]) -> Self {
        let compile_errors = errors.iter().filter(|e| {
            let lower = e.to_lowercase();
            // Rust errors
            e.contains("error[E") ||
            e.contains("could not compile") ||
            e.contains("aborting due to") ||
            // TypeScript errors (error TS1005, error TS2304, etc.)
            e.contains("error TS") ||
            // Python errors
            lower.contains("syntaxerror") ||
            lower.contains("indentationerror") ||
            lower.contains("nameerror") ||
            lower.contains("typeerror") ||
            lower.contains("attributeerror") ||
            lower.contains("importerror") ||
            lower.contains("modulenotfounderror") ||
            // Go errors
            lower.contains("undefined:") ||
            lower.contains("syntax error") ||
            lower.contains("cannot use") ||
            lower.contains("imported and not used")
        }).count();

        let tests_failed = errors.iter().filter(|e| {
            let lower = e.to_lowercase();
            e.contains("FAILED") ||
            e.contains("panicked at") ||
            lower.contains("fail") ||
            lower.contains("assertion")
        }).count();

        let compiles = compile_errors == 0;
        let tests_passed = if compiles {
            // Estimate: if we see "test result: FAILED. X passed", extract X
            errors.iter()
                .find_map(|e| {
                    if e.contains("passed") {
                        let re = regex::Regex::new(r"(\d+) passed").ok()?;
                        re.captures(e)?.get(1)?.as_str().parse().ok()
                    } else {
                        None
                    }
                })
                .unwrap_or(0)
        } else {
            0
        };

        Self {
            compiles,
            tests_passed,
            tests_failed,
            compile_errors,
            total_errors: errors.len(),
        }
    }

    /// Higher is better.
    pub fn quality_score(&self) -> i32 {
        if !self.compiles {
            // Negative score for non-compiling code
            -(self.compile_errors as i32 * 100)
        } else {
            // Positive score based on test results
            (self.tests_passed as i32 * 10) - (self.tests_failed as i32 * 5)
        }
    }

    /// Is this attempt better than another?
    pub fn is_better_than(&self, other: &Self) -> bool {
        self.quality_score() > other.quality_score()
    }
}

/// A recorded attempt.
#[derive(Debug, Clone)]
pub struct Attempt {
    /// Attempt number
    pub number: u32,
    /// The code at this attempt
    pub code: String,
    /// Validation score
    pub score: AttemptScore,
    /// Raw errors
    pub errors: Vec<String>,
}

/// Rollback manager for tracking and restoring best attempts.
pub struct RollbackManager {
    /// All attempts (limited history)
    attempts: VecDeque<Attempt>,
    /// Best attempt so far
    best_attempt: Option<Attempt>,
    /// Max history to keep
    max_history: usize,
    /// Consecutive degradations counter
    degradation_count: u32,
    /// Threshold before triggering rollback
    degradation_threshold: u32,
}

impl Default for RollbackManager {
    fn default() -> Self {
        Self::new()
    }
}

impl RollbackManager {
    pub fn new() -> Self {
        Self {
            attempts: VecDeque::new(),
            best_attempt: None,
            max_history: 10,
            degradation_count: 0,
            degradation_threshold: 2,
        }
    }

    /// Record a new attempt.
    pub fn record(&mut self, attempt_num: u32, code: String, errors: Vec<String>) -> RollbackDecision {
        let score = AttemptScore::from_errors(&errors);

        let attempt = Attempt {
            number: attempt_num,
            code: code.clone(),
            score: score.clone(),
            errors: errors.clone(),
        };

        // Check if this is better than our best
        let is_improvement = match &self.best_attempt {
            None => true,
            Some(best) => score.is_better_than(&best.score),
        };

        if is_improvement {
            self.best_attempt = Some(attempt.clone());
            self.degradation_count = 0;
        } else {
            self.degradation_count += 1;
        }

        // Add to history
        self.attempts.push_back(attempt);
        if self.attempts.len() > self.max_history {
            self.attempts.pop_front();
        }

        // Decide what to do
        if self.should_rollback(&score) {
            if let Some(best) = &self.best_attempt {
                return RollbackDecision::Rollback {
                    to_attempt: best.number,
                    code: best.code.clone(),
                    reason: self.rollback_reason(&score, &best.score),
                    failing_tests: self.extract_failing_tests(&best.errors),
                };
            }
        }

        RollbackDecision::Continue {
            current_score: score,
            best_score: self.best_attempt.as_ref().map(|a| a.score.clone()),
        }
    }

    fn should_rollback(&self, current: &AttemptScore) -> bool {
        // Rollback if:
        // 1. Code stopped compiling but best attempt compiled
        // 2. Too many consecutive degradations
        if let Some(best) = &self.best_attempt {
            // Compilation regression
            if best.score.compiles && !current.compiles {
                return true;
            }

            // Significant test regression
            if best.score.compiles && current.compiles {
                if current.tests_failed > best.score.tests_failed + 2 {
                    return true;
                }
            }

            // Consecutive degradations
            if self.degradation_count >= self.degradation_threshold {
                return true;
            }
        }

        false
    }

    fn rollback_reason(&self, current: &AttemptScore, best: &AttemptScore) -> String {
        if best.compiles && !current.compiles {
            format!(
                "Code stopped compiling ({} errors). Rolling back to attempt that compiled.",
                current.compile_errors
            )
        } else if self.degradation_count >= self.degradation_threshold {
            format!(
                "{} consecutive attempts without improvement. Rolling back to best.",
                self.degradation_count
            )
        } else {
            format!(
                "Quality degraded from {} to {}. Rolling back.",
                best.quality_score(),
                current.quality_score()
            )
        }
    }

    fn extract_failing_tests(&self, errors: &[String]) -> Vec<String> {
        // First, try to find failing tests
        let failed_tests: Vec<String> = errors
            .iter()
            .filter(|e| {
                let lower = e.to_lowercase();
                e.contains("FAILED") ||
                lower.contains("fail") ||
                lower.contains("✕") ||  // Jest failure marker
                lower.contains("error")  // General errors
            })
            .map(|e| {
                // Extract Rust test name
                if let Some(start) = e.find("test ") {
                    let rest = &e[start + 5..];
                    if let Some(end) = rest.find(" ...") {
                        return rest[..end].to_string();
                    }
                }
                // Extract Jest test name (✕ test name)
                if let Some(start) = e.find("✕ ") {
                    let rest = &e[start + 4..];  // Skip "✕ "
                    if let Some(end) = rest.find(" (") {
                        return rest[..end].to_string();
                    }
                    return rest.to_string();
                }
                // Extract Go test name (--- FAIL: TestXxx)
                if let Some(start) = e.find("FAIL: ") {
                    let rest = &e[start + 6..];
                    if let Some(end) = rest.find(" (") {
                        return rest[..end].to_string();
                    }
                    return rest.to_string();
                }
                // Extract pytest failure (FAILED test_xxx.py::test_name)
                if let Some(start) = e.find("FAILED ") {
                    let rest = &e[start + 7..];
                    if let Some(end) = rest.find(" -") {
                        return rest[..end].to_string();
                    }
                    return rest.to_string();
                }
                e.clone()
            })
            .collect();

        // If no failed tests, extract compilation errors instead
        if failed_tests.is_empty() {
            return errors
                .iter()
                .filter(|e| {
                    e.contains("error[E") ||  // Rust
                    e.contains("error TS") ||  // TypeScript
                    e.to_lowercase().contains("error") ||  // General
                    e.to_lowercase().contains("syntaxerror")  // Python
                })
                .take(3)  // Limit to top 3 errors
                .cloned()
                .collect();
        }

        failed_tests
    }

    /// Get the best attempt so far.
    pub fn best(&self) -> Option<&Attempt> {
        self.best_attempt.as_ref()
    }

    /// Get statistics.
    pub fn stats(&self) -> RollbackStats {
        RollbackStats {
            total_attempts: self.attempts.len(),
            best_attempt_num: self.best_attempt.as_ref().map(|a| a.number),
            best_score: self.best_attempt.as_ref().map(|a| a.score.quality_score()),
            degradation_count: self.degradation_count,
        }
    }

    /// Generate a targeted fix prompt using the best attempt.
    pub fn generate_targeted_prompt(&self, failing_test: &str) -> Option<String> {
        let best = self.best_attempt.as_ref()?;

        // Extract the relevant parts of the code
        let mut prompt = String::new();

        prompt.push_str("=== BEST WORKING CODE (COMPILES, ALMOST PASSES) ===\n\n");
        prompt.push_str(&best.code);

        prompt.push_str("\n\n=== SINGLE FAILING TEST ===\n");
        prompt.push_str(&format!("Only {} is failing.\n\n", failing_test));

        prompt.push_str("=== YOUR TASK ===\n");
        prompt.push_str("Fix ONLY the logic that makes this specific test fail.\n");
        prompt.push_str("DO NOT change other parts of the code that work.\n");
        prompt.push_str("Return the COMPLETE fixed code.\n");

        Some(prompt)
    }
}

/// Decision after recording an attempt.
#[derive(Debug, Clone)]
pub enum RollbackDecision {
    /// Continue with current code
    Continue {
        current_score: AttemptScore,
        best_score: Option<AttemptScore>,
    },
    /// Rollback to a better attempt
    Rollback {
        to_attempt: u32,
        code: String,
        reason: String,
        failing_tests: Vec<String>,
    },
}

/// Statistics about rollback state.
#[derive(Debug, Clone)]
pub struct RollbackStats {
    pub total_attempts: usize,
    pub best_attempt_num: Option<u32>,
    pub best_score: Option<i32>,
    pub degradation_count: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_compiling_is_better() {
        let compiles = AttemptScore {
            compiles: true,
            tests_passed: 3,
            tests_failed: 1,
            compile_errors: 0,
            total_errors: 1,
        };

        let doesnt_compile = AttemptScore {
            compiles: false,
            tests_passed: 0,
            tests_failed: 0,
            compile_errors: 5,
            total_errors: 5,
        };

        assert!(compiles.is_better_than(&doesnt_compile));
        assert!(!doesnt_compile.is_better_than(&compiles));
    }

    #[test]
    fn test_rollback_on_compile_regression() {
        let mut manager = RollbackManager::new();

        // First attempt: compiles, 1 test fails
        let decision = manager.record(1, "good code".into(), vec![
            "test tests::test_a ... FAILED".into(),
        ]);
        assert!(matches!(decision, RollbackDecision::Continue { .. }));

        // Second attempt: doesn't compile
        let decision = manager.record(2, "broken code".into(), vec![
            "error[E0308]: mismatched types".into(),
            "could not compile".into(),
        ]);

        // Should trigger rollback
        assert!(matches!(decision, RollbackDecision::Rollback { .. }));
        if let RollbackDecision::Rollback { code, .. } = decision {
            assert_eq!(code, "good code");
        }
    }

    #[test]
    fn test_rollback_after_degradation() {
        let mut manager = RollbackManager::new();

        // Best attempt
        manager.record(1, "good".into(), vec![
            "test tests::test_a ... FAILED".into(),
        ]);

        // Two worse attempts
        manager.record(2, "worse".into(), vec![
            "test tests::test_a ... FAILED".into(),
            "test tests::test_b ... FAILED".into(),
        ]);

        let decision = manager.record(3, "even worse".into(), vec![
            "test tests::test_a ... FAILED".into(),
            "test tests::test_b ... FAILED".into(),
            "test tests::test_c ... FAILED".into(),
        ]);

        // Should rollback after 2 degradations
        assert!(matches!(decision, RollbackDecision::Rollback { to_attempt: 1, .. }));
    }
}
