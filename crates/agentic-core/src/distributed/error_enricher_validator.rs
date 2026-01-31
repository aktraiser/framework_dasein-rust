//! Error Enricher Validator - Transforms raw errors into actionable feedback.
//!
//! Takes raw error messages from previous validators (e.g., SandboxValidator)
//! and enriches them with:
//! - **Localization**: Where is the bug? (file, line, function)
//! - **Instructions**: What to do to fix it?
//!
//! # Example
//!
//! Input error:
//! ```text
//! AssertionError: assert 'OPEN' == 'HALF_OPEN'
//! ```
//!
//! Enriched output:
//! ```text
//! LOCATION: test_circuit_breaker_open_to_half_open_to_open, line 453
//! PROBLEM: State is 'OPEN' but expected 'HALF_OPEN' after failure in HALF_OPEN mode
//! HINT: Check if failure_count is reset when transitioning to HALF_OPEN state
//! ```

use async_trait::async_trait;
use regex::Regex;
use std::collections::HashMap;

use super::validator_pipeline::{PipelineValidator, ValidatorInput, ValidatorOutput};

/// Validator that enriches error messages with localization and hints.
pub struct ErrorEnricherValidator {
    /// Language-specific enrichers
    enrichers: HashMap<String, Box<dyn ErrorEnricher + Send + Sync>>,
}

impl ErrorEnricherValidator {
    pub fn new() -> Self {
        let mut enrichers: HashMap<String, Box<dyn ErrorEnricher + Send + Sync>> = HashMap::new();
        enrichers.insert("python".to_string(), Box::new(PythonEnricher::new()));
        enrichers.insert("rust".to_string(), Box::new(RustEnricher::new()));
        enrichers.insert("go".to_string(), Box::new(GoEnricher::new()));
        enrichers.insert(
            "typescript".to_string(),
            Box::new(TypeScriptEnricher::new()),
        );

        Self { enrichers }
    }
}

impl Default for ErrorEnricherValidator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PipelineValidator for ErrorEnricherValidator {
    fn name(&self) -> &str {
        "error-enricher"
    }

    async fn validate(&self, input: &ValidatorInput) -> Result<ValidatorOutput, String> {
        // Get errors from previous validators
        let previous_errors: Vec<String> = input
            .previous_results
            .iter()
            .flat_map(|r| r.errors.clone())
            .collect();

        if previous_errors.is_empty() {
            return Ok(ValidatorOutput::success(self.name()));
        }

        // Get the appropriate enricher for the language
        let lang_enricher = self.enrichers.get(&input.language.to_lowercase());

        let mut enriched_errors = Vec::new();
        let mut recommendations = Vec::new();

        for error in &previous_errors {
            if let Some(err_enricher) = lang_enricher {
                let enriched_error = err_enricher.enrich(error, &input.code);
                enriched_errors.push(enriched_error.formatted());
                recommendations.extend(enriched_error.hints);
            } else {
                // No enricher for this language, pass through
                enriched_errors.push(error.clone());
            }
        }

        // Deduplicate recommendations
        recommendations.sort();
        recommendations.dedup();

        Ok(ValidatorOutput::failure(self.name(), enriched_errors)
            .with_recommendations(recommendations))
    }
}

/// Enriched error with location and hints.
#[derive(Debug, Clone)]
pub struct EnrichedError {
    /// Original error message
    pub original: String,
    /// File where the error occurred
    pub file: Option<String>,
    /// Line number
    pub line: Option<u32>,
    /// Function/method name
    pub function: Option<String>,
    /// What the problem is (human readable)
    pub problem: Option<String>,
    /// Actionable hints to fix the issue
    pub hints: Vec<String>,
}

impl EnrichedError {
    fn new(original: &str) -> Self {
        Self {
            original: original.to_string(),
            file: None,
            line: None,
            function: None,
            problem: None,
            hints: vec![],
        }
    }

    fn formatted(&self) -> String {
        let mut parts = Vec::new();

        // Location
        let location = match (&self.file, self.line, &self.function) {
            (Some(f), Some(l), Some(func)) => Some(format!("{}:{} in {}", f, l, func)),
            (Some(f), Some(l), None) => Some(format!("{}:{}", f, l)),
            (None, None, Some(func)) => Some(format!("in {}", func)),
            _ => None,
        };

        if let Some(loc) = location {
            parts.push(format!("LOCATION: {}", loc));
        }

        // Problem description
        if let Some(ref prob) = self.problem {
            parts.push(format!("PROBLEM: {}", prob));
        }

        // Original error (shortened if too long)
        let short_original: String = self
            .original
            .lines()
            .take(3)
            .collect::<Vec<_>>()
            .join(" | ");
        parts.push(format!("ERROR: {}", short_original));

        // Hints
        for hint in &self.hints {
            parts.push(format!("HINT: {}", hint));
        }

        parts.join("\n")
    }
}

/// Trait for language-specific error enrichment.
trait ErrorEnricher {
    fn enrich(&self, error: &str, code: &str) -> EnrichedError;
}

// =============================================================================
// Python Enricher
// =============================================================================

struct PythonEnricher {
    assertion_re: Regex,
    file_line_re: Regex,
    test_func_re: Regex,
}

impl PythonEnricher {
    fn new() -> Self {
        Self {
            assertion_re: Regex::new(r"assert\s+(.+?)\s*==\s*(.+)").unwrap(),
            file_line_re: Regex::new(r"(\w+\.py):(\d+)").unwrap(),
            test_func_re: Regex::new(r"(test_\w+)").unwrap(),
        }
    }
}

impl ErrorEnricher for PythonEnricher {
    fn enrich(&self, error: &str, code: &str) -> EnrichedError {
        let mut enriched = EnrichedError::new(error);

        // Extract file and line
        if let Some(caps) = self.file_line_re.captures(error) {
            enriched.file = Some(caps[1].to_string());
            enriched.line = caps[2].parse().ok();
        }

        // Extract test function name
        if let Some(caps) = self.test_func_re.captures(error) {
            enriched.function = Some(caps[1].to_string());
        }

        // Analyze assertion errors
        if error.contains("AssertionError") {
            if let Some(caps) = self.assertion_re.captures(error) {
                let actual = caps.get(1).map(|m| m.as_str()).unwrap_or("?");
                let expected = caps.get(2).map(|m| m.as_str()).unwrap_or("?");
                enriched.problem = Some(format!(
                    "Assertion failed: got {} but expected {}",
                    actual, expected
                ));
            }

            // State machine specific hints
            if error.contains("'OPEN'") && error.contains("'HALF_OPEN'") {
                enriched.hints.push(
                    "State machine bug: Check if failure_count is reset when transitioning to HALF_OPEN".to_string()
                );
                enriched.hints.push(
                    "The HALF_OPEN state should allow half_open_max failures before going back to OPEN".to_string()
                );
            }

            if error.contains("'CLOSED'") && error.contains("'OPEN'") {
                enriched.hints.push(
                    "Check the failure_threshold: state should only go to OPEN after threshold failures".to_string()
                );
            }
        }

        // Threading hints
        if error.contains("deadlock") || error.contains("timeout") {
            enriched
                .hints
                .push("Possible deadlock: avoid acquiring locks in nested order".to_string());
            enriched.hints.push(
                "Use threading.RLock if the same thread needs to acquire the lock multiple times"
                    .to_string(),
            );
        }

        // Look for the function in code and add context
        if let Some(ref func_name) = enriched.function {
            if code.contains(&format!("def {}(", func_name)) {
                enriched.hints.push(format!(
                    "Review the test '{}' - it may have incorrect expectations or the implementation logic is wrong",
                    func_name
                ));
            }
        }

        enriched
    }
}

// =============================================================================
// Rust Enricher
// =============================================================================

struct RustEnricher {
    error_code_re: Regex,
    file_line_re: Regex,
}

impl RustEnricher {
    fn new() -> Self {
        Self {
            error_code_re: Regex::new(r"error\[E(\d+)\]").unwrap(),
            file_line_re: Regex::new(r"(\w+\.rs):(\d+)").unwrap(),
        }
    }
}

impl ErrorEnricher for RustEnricher {
    fn enrich(&self, error: &str, _code: &str) -> EnrichedError {
        let mut enriched = EnrichedError::new(error);

        // Extract error code
        if let Some(caps) = self.error_code_re.captures(error) {
            let code = &caps[1];
            enriched.problem = Some(match code {
                "0277" => "Trait bound not satisfied".to_string(),
                "0282" => "Type annotations needed - compiler can't infer the type".to_string(),
                "0308" => "Mismatched types".to_string(),
                "0382" => "Borrow of moved value".to_string(),
                "0499" => "Cannot borrow as mutable more than once".to_string(),
                "0502" => "Cannot borrow as immutable because it's borrowed as mutable".to_string(),
                "0599" => "Method exists but trait bounds not satisfied".to_string(),
                "0614" => "Trait not in scope".to_string(),
                _ => format!("Rust error E{}", code),
            });
        }

        // Extract file and line
        if let Some(caps) = self.file_line_re.captures(error) {
            enriched.file = Some(caps[1].to_string());
            enriched.line = caps[2].parse().ok();
        }

        // Add hints based on error type
        if error.contains("Clone") && error.contains("not satisfied") {
            enriched
                .hints
                .push("Add #[derive(Clone)] to the struct/enum definition".to_string());
        }

        if error.contains("Send") && error.contains("not satisfied") {
            enriched.hints.push("The type must be Send to be used across threads. Consider using Arc<Mutex<T>> instead of Rc<RefCell<T>>".to_string());
        }

        if error.contains("type annotations needed") {
            enriched
                .hints
                .push("Add explicit type annotation, e.g., let x: Type = ...".to_string());
            enriched
                .hints
                .push("Or use turbofish syntax: function::<Type>(...)".to_string());
        }

        if error.contains("mismatched types") {
            enriched
                .hints
                .push("Check the return type matches the function signature".to_string());
            enriched
                .hints
                .push("Use .into() or From/Into traits for type conversion".to_string());
        }

        // Test failures
        if error.contains("FAILED") && error.contains("panicked") {
            enriched.hints.push("Test assertion failed - check if the expected value matches implementation behavior".to_string());
        }

        enriched
    }
}

// =============================================================================
// Go Enricher
// =============================================================================

struct GoEnricher {
    file_line_re: Regex,
}

impl GoEnricher {
    fn new() -> Self {
        Self {
            file_line_re: Regex::new(r"(\w+\.go):(\d+)").unwrap(),
        }
    }
}

impl ErrorEnricher for GoEnricher {
    fn enrich(&self, error: &str, _code: &str) -> EnrichedError {
        let mut enriched = EnrichedError::new(error);

        // Extract file and line
        if let Some(caps) = self.file_line_re.captures(error) {
            enriched.file = Some(caps[1].to_string());
            enriched.line = caps[2].parse().ok();
        }

        // Go-specific hints
        if error.contains("undefined:") {
            enriched.hints.push(
                "Variable or function is not defined - check spelling or imports".to_string(),
            );
        }

        if error.contains("cannot use") && error.contains("as") {
            enriched
                .hints
                .push("Type mismatch - check if you need type conversion or assertion".to_string());
        }

        if error.contains("deadlock") {
            enriched.hints.push(
                "Deadlock detected - check channel operations and goroutine synchronization"
                    .to_string(),
            );
            enriched
                .hints
                .push("Ensure all channel sends have corresponding receives".to_string());
        }

        if error.contains("nil pointer") {
            enriched.hints.push(
                "Nil pointer dereference - add nil check before using the pointer".to_string(),
            );
        }

        enriched
    }
}

// =============================================================================
// TypeScript Enricher
// =============================================================================

struct TypeScriptEnricher {
    file_line_col_re: Regex,
    file_line_re: Regex,
    ts_error_re: Regex,
    template_literal_re: Regex,
}

impl TypeScriptEnricher {
    fn new() -> Self {
        Self {
            // Pattern: main.ts(34,31) - with parentheses
            file_line_col_re: Regex::new(r"(\w+\.tsx?)\((\d+),(\d+)\)").unwrap(),
            // Pattern: main.ts:34 - with colon
            file_line_re: Regex::new(r"(\w+\.tsx?):(\d+)").unwrap(),
            ts_error_re: Regex::new(r"TS(\d+)").unwrap(),
            // Detect template literal syntax without backticks: ${...} not inside backticks
            template_literal_re: Regex::new(r"\$\{[^}]+\}").unwrap(),
        }
    }

    /// Extract the specific line of code from the source
    fn get_code_line(&self, code: &str, line_num: u32) -> Option<String> {
        code.lines()
            .nth((line_num - 1) as usize)
            .map(|s| s.to_string())
    }

    /// Check if a line has template literal syntax without backticks
    fn detect_missing_backticks(&self, line: &str) -> bool {
        // Has ${...} pattern
        if !self.template_literal_re.is_match(line) {
            return false;
        }

        // Check if it's properly inside backticks
        // A proper template literal looks like: `text ${var} more text`
        // A broken one looks like: "text ${var}" or Error(text ${var})

        // Count backticks - if odd number or zero, it's broken
        let backtick_count = line.chars().filter(|&c| c == '`').count();
        if backtick_count == 0 {
            return true; // Has ${} but no backticks at all
        }

        // Simple heuristic: check if ${} appears after an opening ( or " or '
        // without an intervening backtick
        let chars: Vec<char> = line.chars().collect();
        let mut in_template = false;
        let mut found_template_syntax = false;

        for i in 0..chars.len() {
            if chars[i] == '`' {
                in_template = !in_template;
            }
            // Check for ${ pattern
            if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '{' {
                if !in_template {
                    found_template_syntax = true;
                }
            }
        }

        found_template_syntax
    }
}

impl ErrorEnricher for TypeScriptEnricher {
    fn enrich(&self, error: &str, code: &str) -> EnrichedError {
        let mut enriched = EnrichedError::new(error);
        let mut line_num: Option<u32> = None;
        let mut _col_num: Option<u32> = None;

        // Extract file, line, and column - try (line,col) format first
        if let Some(caps) = self.file_line_col_re.captures(error) {
            enriched.file = Some(caps[1].to_string());
            line_num = caps[2].parse().ok();
            _col_num = caps[3].parse().ok();
            enriched.line = line_num;
        } else if let Some(caps) = self.file_line_re.captures(error) {
            enriched.file = Some(caps[1].to_string());
            line_num = caps[2].parse().ok();
            enriched.line = line_num;
        }

        // CRITICAL: Extract the actual line of code that has the error
        let faulty_line = line_num.and_then(|ln| self.get_code_line(code, ln));

        // TypeScript error codes with DETAILED handling
        if let Some(caps) = self.ts_error_re.captures(error) {
            let ts_code = &caps[1];
            match ts_code {
                // =============== SYNTAX ERRORS (TS1xxx) ===============
                "1005" | "1003" | "1109" | "1128" => {
                    enriched.problem = Some(match ts_code {
                        "1005" => {
                            "Syntax error: missing delimiter (comma, semicolon, etc.)".to_string()
                        }
                        "1003" => "Syntax error: identifier expected".to_string(),
                        "1109" => "Syntax error: expression expected".to_string(),
                        "1128" => "Syntax error: declaration or statement expected".to_string(),
                        _ => format!("Syntax error TS{}", ts_code),
                    });

                    // Check if the error is due to missing backticks in template literals
                    if let Some(ref line) = faulty_line {
                        if self.detect_missing_backticks(line) {
                            enriched.problem = Some(
                                "TEMPLATE LITERAL WITHOUT BACKTICKS! You used ${...} interpolation syntax but forgot the backticks.".to_string()
                            );

                            // Show the faulty line
                            enriched.hints.push(format!(
                                "FAULTY LINE {}: {}",
                                line_num.unwrap_or(0),
                                line.trim()
                            ));

                            // Provide the fix
                            enriched.hints.push(
                                "FIX: Wrap the string with BACKTICKS (`), not quotes (\"/')."
                                    .to_string(),
                            );

                            // Try to suggest the corrected line
                            if line.contains("Error(") || line.contains("error(") {
                                enriched.hints.push(
                                    "Example: throw new Error(`Message with ${variable}`) - note the BACKTICKS!".to_string()
                                );
                            } else {
                                enriched.hints.push(
                                    "Example: const msg = `Hello ${name}` - BACKTICKS required for ${} syntax!".to_string()
                                );
                            }
                        } else {
                            enriched.hints.push(format!(
                                "LINE {}: {}",
                                line_num.unwrap_or(0),
                                line.trim()
                            ));
                            enriched.hints.push(
                                "Check for missing/extra punctuation at this location.".to_string(),
                            );
                        }
                    }
                }

                // =============== LOOKUP ERRORS (TS2xxx) ===============
                "2304" => {
                    enriched.problem =
                        Some("Cannot find name - identifier is not defined".to_string());
                    enriched
                        .hints
                        .push("Add the import or define the variable/function.".to_string());
                }
                "2307" => {
                    enriched.problem = Some("Cannot find module".to_string());
                    enriched
                        .hints
                        .push("Check the import path or install the package.".to_string());
                }

                // =============== TYPE ERRORS (TS2xxx) ===============
                "2322" => {
                    enriched.problem = Some("Type is not assignable".to_string());
                    enriched
                        .hints
                        .push("Check that the value matches the expected type.".to_string());
                }
                "2339" => {
                    enriched.problem = Some("Property does not exist on type".to_string());
                    enriched.hints.push(
                        "Check the spelling or add the property to the type definition."
                            .to_string(),
                    );
                }
                "2345" => {
                    enriched.problem =
                        Some("Argument type is not assignable to parameter type".to_string());
                    enriched
                        .hints
                        .push("Convert the argument to the expected type.".to_string());
                }
                "2551" => {
                    enriched.problem =
                        Some("Property does not exist, did you mean...?".to_string());
                    enriched
                        .hints
                        .push("Check for typos in property name.".to_string());
                }
                "7006" => {
                    enriched.problem = Some("Parameter implicitly has 'any' type".to_string());
                    enriched
                        .hints
                        .push("Add explicit type annotation to the parameter.".to_string());
                }

                _ => {
                    enriched.problem = Some(format!("TypeScript error TS{}", ts_code));
                }
            }
        }

        // Always show the faulty line if we have it and haven't shown it yet
        if let Some(ref line) = faulty_line {
            if enriched.hints.is_empty() || !enriched.hints.iter().any(|h| h.contains("LINE")) {
                enriched.hints.insert(
                    0,
                    format!("CODE AT LINE {}: {}", line_num.unwrap_or(0), line.trim()),
                );
            }
        }

        // Jest test failures
        if error.contains("expect(") && error.contains("toBe") {
            enriched
                .hints
                .push("Jest assertion failed - check expected vs received values".to_string());
        }

        if error.contains("async") && error.contains("Promise") {
            enriched
                .hints
                .push("Ensure async functions are properly awaited".to_string());
        }

        enriched
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_python_enricher_assertion() {
        let enricher = PythonEnricher::new();
        let error = "test_main.py:453: AssertionError: assert 'OPEN' == 'HALF_OPEN'";
        let enriched = enricher.enrich(error, "");

        assert_eq!(enriched.file, Some("test_main.py".to_string()));
        assert_eq!(enriched.line, Some(453));
        assert!(!enriched.hints.is_empty());
        assert!(enriched.hints.iter().any(|h| h.contains("failure_count")));
    }

    #[test]
    fn test_rust_enricher_trait_bound() {
        let enricher = RustEnricher::new();
        let error = "error[E0277]: the trait bound `TaskError: Clone` is not satisfied";
        let enriched = enricher.enrich(error, "");

        assert!(enriched.problem.is_some());
        assert!(enriched
            .hints
            .iter()
            .any(|h| h.contains("#[derive(Clone)]")));
    }

    #[tokio::test]
    async fn test_error_enricher_validator() {
        let validator = ErrorEnricherValidator::new();

        let mut input = ValidatorInput::new("", "python");
        input.previous_results = vec![ValidatorOutput::failure(
            "sandbox",
            vec!["test_main.py:100: AssertionError: assert 'OPEN' == 'HALF_OPEN'".to_string()],
        )];

        let result = validator.validate(&input).await.unwrap();

        assert!(!result.passed);
        assert!(!result.recommendations.is_empty());
    }
}
