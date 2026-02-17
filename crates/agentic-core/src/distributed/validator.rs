//! Validator - Rule-based output quality checks.
//!
//! Fast, heuristic validation for any output type. Use this for:
//! - Text validation (not empty, no errors, length limits)
//! - JSON validation
//! - Quick syntax checks
//! - Content filtering (no secrets, no TODOs)
//!
//! For **code validation with real compilation/tests**, use
//! [`SandboxValidator`](super::SandboxValidator) instead.
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::{Validator, ValidationRule};
//!
//! let validator = Validator::new("val-001", "sup-001")
//!     .rule(ValidationRule::OutputNotEmpty)
//!     .rule(ValidationRule::NoErrors)
//!     .rule(ValidationRule::NoSecrets)
//!     .build();
//!
//! let result = validator.validate("Some LLM output", 0);
//! assert!(result.passed);
//! ```
//!
//! # Validator vs SandboxValidator
//!
//! | Aspect | Validator | SandboxValidator |
//! |--------|-----------|------------------|
//! | Speed | Fast (~1ms) | Slower (~1-5s) |
//! | Accuracy | Heuristic | Ground truth |
//! | Use for | Text, JSON | Code |
//! | Feedback | Rule-based | Real errors |

use serde::{Deserialize, Serialize};

use super::config::ValidatorConfig;

/// Predefined validation rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationRule {
    /// Output must not be empty
    OutputNotEmpty,
    /// No error in output
    NoErrors,
    /// Output must be valid JSON
    ValidJson,
    /// Code must compile (requires language)
    CodeCompiles { language: String },
    /// No TODO/FIXME in code
    NoTodos,
    /// Must have tests
    HasTests,
    /// No secrets/API keys
    NoSecrets,
    /// Max output length
    MaxLength { chars: usize },
    /// Min output length
    MinLength { chars: usize },
    /// Must contain pattern
    Contains { pattern: String },
    /// Must not contain pattern
    NotContains { pattern: String },
    /// Custom rule (name only, logic in validator)
    Custom { name: String },
}

impl ValidationRule {
    /// Get rule name.
    pub fn name(&self) -> &str {
        match self {
            Self::OutputNotEmpty => "output_not_empty",
            Self::NoErrors => "no_errors",
            Self::ValidJson => "valid_json",
            Self::CodeCompiles { .. } => "code_compiles",
            Self::NoTodos => "no_todos",
            Self::HasTests => "has_tests",
            Self::NoSecrets => "no_secrets",
            Self::MaxLength { .. } => "max_length",
            Self::MinLength { .. } => "min_length",
            Self::Contains { .. } => "contains",
            Self::NotContains { .. } => "not_contains",
            Self::Custom { name } => name,
        }
    }
}

/// Result of validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResult {
    /// Overall pass/fail
    pub passed: bool,
    /// Results per rule
    pub rule_results: Vec<RuleResult>,
    /// Overall score (0-100)
    pub score: u32,
    /// Feedback if failed
    pub feedback: Option<String>,
    /// Recommended action
    pub action: ValidationAction,
}

/// Result of a single rule check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleResult {
    pub rule: String,
    pub passed: bool,
    pub message: Option<String>,
}

/// Action to take after validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationAction {
    /// Accept the output
    Accept,
    /// Retry with feedback
    Retry { feedback: String, attempt: u32 },
    /// Reject definitively
    Reject { reason: String },
}

/// Validator that checks output quality.
#[derive(Debug)]
pub struct Validator {
    /// Unique identifier
    pub id: String,
    /// Associated supervisor
    pub supervisor: String,
    /// Validation rules
    rules: Vec<ValidationRule>,
    /// Max retries on failure
    max_retries: u32,
}

impl Validator {
    /// Create a new validator.
    ///
    /// # Example
    ///
    /// ```rust,ignore
    /// let val = Validator::new("val-001", "sup-001")
    ///     .rule(ValidationRule::OutputNotEmpty)
    ///     .build();
    /// ```
    pub fn new(id: impl Into<String>, supervisor: impl Into<String>) -> ValidatorBuilder {
        ValidatorBuilder::new(id.into(), supervisor.into())
    }

    /// Create from config.
    pub fn from_config(config: ValidatorConfig) -> Self {
        let rules = config
            .rules
            .iter()
            .map(|r| match r.as_str() {
                "output_not_empty" => ValidationRule::OutputNotEmpty,
                "no_errors" => ValidationRule::NoErrors,
                "valid_json" => ValidationRule::ValidJson,
                "no_todos" => ValidationRule::NoTodos,
                "has_tests" => ValidationRule::HasTests,
                "no_secrets" => ValidationRule::NoSecrets,
                _ => ValidationRule::Custom { name: r.clone() },
            })
            .collect();

        Self {
            id: config.id,
            supervisor: config.supervisor,
            rules,
            max_retries: config.max_retries,
        }
    }

    /// Validate output.
    pub fn validate(&self, output: &str, attempt: u32) -> ValidationResult {
        let mut rule_results = Vec::new();
        let mut all_passed = true;
        let mut feedback_parts = Vec::new();

        for rule in &self.rules {
            let (passed, message) = self.check_rule(rule, output);

            if !passed {
                all_passed = false;
                if let Some(ref msg) = message {
                    feedback_parts.push(format!("{}: {}", rule.name(), msg));
                }
            }

            rule_results.push(RuleResult {
                rule: rule.name().to_string(),
                passed,
                message,
            });
        }

        let passed_count = rule_results.iter().filter(|r| r.passed).count();
        let score = if self.rules.is_empty() {
            100
        } else {
            ((passed_count as f32 / self.rules.len() as f32) * 100.0) as u32
        };

        let action = if all_passed {
            ValidationAction::Accept
        } else if attempt < self.max_retries {
            ValidationAction::Retry {
                feedback: feedback_parts.join("; "),
                attempt: attempt + 1,
            }
        } else {
            ValidationAction::Reject {
                reason: feedback_parts.join("; "),
            }
        };

        let feedback = if all_passed {
            None
        } else {
            Some(feedback_parts.join("; "))
        };

        ValidationResult {
            passed: all_passed,
            rule_results,
            score,
            feedback,
            action,
        }
    }

    /// Check a single rule.
    fn check_rule(&self, rule: &ValidationRule, output: &str) -> (bool, Option<String>) {
        match rule {
            ValidationRule::OutputNotEmpty => {
                let passed = !output.trim().is_empty();
                (
                    passed,
                    if passed {
                        None
                    } else {
                        Some("Output is empty".into())
                    },
                )
            }
            ValidationRule::NoErrors => {
                let has_error = output.to_lowercase().contains("error:");
                (
                    !has_error,
                    if has_error {
                        Some("Output contains errors".into())
                    } else {
                        None
                    },
                )
            }
            ValidationRule::ValidJson => match serde_json::from_str::<serde_json::Value>(output) {
                Ok(_) => (true, None),
                Err(e) => (false, Some(format!("Invalid JSON: {}", e))),
            },
            ValidationRule::NoTodos => {
                let has_todo = output.contains("TODO") || output.contains("FIXME");
                (
                    !has_todo,
                    if has_todo {
                        Some("Contains TODO/FIXME".into())
                    } else {
                        None
                    },
                )
            }
            ValidationRule::HasTests => {
                let has_tests = output.contains("#[test]") || output.contains("fn test_");
                (
                    has_tests,
                    if has_tests {
                        None
                    } else {
                        Some("No tests found".into())
                    },
                )
            }
            ValidationRule::NoSecrets => {
                let patterns = ["api_key", "secret", "password", "token"];
                let has_secret = patterns.iter().any(|p| output.to_lowercase().contains(p));
                (
                    !has_secret,
                    if has_secret {
                        Some("Potential secret detected".into())
                    } else {
                        None
                    },
                )
            }
            ValidationRule::MaxLength { chars } => {
                let passed = output.len() <= *chars;
                (
                    passed,
                    if passed {
                        None
                    } else {
                        Some(format!("Output too long: {} > {}", output.len(), chars))
                    },
                )
            }
            ValidationRule::MinLength { chars } => {
                let passed = output.len() >= *chars;
                (
                    passed,
                    if passed {
                        None
                    } else {
                        Some(format!("Output too short: {} < {}", output.len(), chars))
                    },
                )
            }
            ValidationRule::Contains { pattern } => {
                let passed = output.contains(pattern);
                (
                    passed,
                    if passed {
                        None
                    } else {
                        Some(format!("Missing required pattern: {}", pattern))
                    },
                )
            }
            ValidationRule::NotContains { pattern } => {
                let passed = !output.contains(pattern);
                (
                    passed,
                    if passed {
                        None
                    } else {
                        Some(format!("Contains forbidden pattern: {}", pattern))
                    },
                )
            }
            ValidationRule::CodeCompiles { language: _ } => {
                // Placeholder - real implementation would compile
                (true, None)
            }
            ValidationRule::Custom { name: _ } => {
                // Custom rules always pass by default
                (true, None)
            }
        }
    }
}

/// Builder for Validator.
pub struct ValidatorBuilder {
    id: String,
    supervisor: String,
    rules: Vec<ValidationRule>,
    max_retries: u32,
}

impl ValidatorBuilder {
    fn new(id: String, supervisor: String) -> Self {
        Self {
            id,
            supervisor,
            rules: Vec::new(),
            max_retries: 2,
        }
    }

    /// Add a validation rule.
    pub fn rule(mut self, rule: ValidationRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Add multiple rules.
    pub fn rules(mut self, rules: impl IntoIterator<Item = ValidationRule>) -> Self {
        self.rules.extend(rules);
        self
    }

    /// Use default rules (output_not_empty, no_errors).
    pub fn default_rules(self) -> Self {
        self.rule(ValidationRule::OutputNotEmpty)
            .rule(ValidationRule::NoErrors)
    }

    /// Use strict rules for code.
    pub fn strict_code_rules(self) -> Self {
        self.rule(ValidationRule::OutputNotEmpty)
            .rule(ValidationRule::NoErrors)
            .rule(ValidationRule::NoTodos)
            .rule(ValidationRule::NoSecrets)
    }

    /// Set max retries.
    pub fn max_retries(mut self, n: u32) -> Self {
        self.max_retries = n;
        self
    }

    /// Build the validator.
    pub fn build(self) -> Validator {
        let rules = if self.rules.is_empty() {
            vec![ValidationRule::OutputNotEmpty]
        } else {
            self.rules
        };

        Validator {
            id: self.id,
            supervisor: self.supervisor,
            rules,
            max_retries: self.max_retries,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validator_pass() {
        let val = Validator::new("val-001", "sup-001")
            .rule(ValidationRule::OutputNotEmpty)
            .rule(ValidationRule::NoErrors)
            .build();

        let result = val.validate("Hello world", 0);
        assert!(result.passed);
        assert_eq!(result.score, 100);
    }

    #[test]
    fn test_validator_fail_empty() {
        let val = Validator::new("val-001", "sup-001")
            .rule(ValidationRule::OutputNotEmpty)
            .build();

        let result = val.validate("", 0);
        assert!(!result.passed);
        assert!(matches!(result.action, ValidationAction::Retry { .. }));
    }

    #[test]
    fn test_validator_fail_max_retries() {
        let val = Validator::new("val-001", "sup-001")
            .rule(ValidationRule::OutputNotEmpty)
            .max_retries(2)
            .build();

        let result = val.validate("", 2);
        assert!(!result.passed);
        assert!(matches!(result.action, ValidationAction::Reject { .. }));
    }
}
