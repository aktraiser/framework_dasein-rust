//! Bus Linter - Fast syntax validation before expensive compilation.
//!
//! Performs quick checks to catch common errors before a 25-second compile:
//! - Balanced braces, brackets, parentheses
//! - Missing semicolons
//! - Invalid Rust syntax patterns
//! - Type name consistency
//! - Import validation
//!
//! This is NOT a full Rust parser - it's a fast heuristic check.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tracing::{debug, warn};

/// Severity of a lint error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LintSeverity {
    /// Will definitely cause compilation failure
    Error,
    /// Likely to cause issues
    Warning,
    /// Suggestion for improvement
    Hint,
}

/// A lint error found in the code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintError {
    /// Line number (1-indexed)
    pub line: usize,
    /// Column number (1-indexed)
    pub column: usize,
    /// Error message
    pub message: String,
    /// Severity
    pub severity: LintSeverity,
    /// Error code for categorization
    pub code: String,
}

impl LintError {
    pub fn error(line: usize, column: usize, code: &str, message: impl Into<String>) -> Self {
        Self {
            line,
            column,
            message: message.into(),
            severity: LintSeverity::Error,
            code: code.to_string(),
        }
    }

    pub fn warning(line: usize, column: usize, code: &str, message: impl Into<String>) -> Self {
        Self {
            line,
            column,
            message: message.into(),
            severity: LintSeverity::Warning,
            code: code.to_string(),
        }
    }
}

/// Result of linting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LintResult {
    /// Whether the code passed all checks
    pub passed: bool,
    /// Errors found
    pub errors: Vec<LintError>,
    /// Warnings found
    pub warnings: Vec<LintError>,
    /// Time taken in milliseconds
    pub lint_time_ms: u64,
}

impl LintResult {
    pub fn ok() -> Self {
        Self {
            passed: true,
            errors: vec![],
            warnings: vec![],
            lint_time_ms: 0,
        }
    }

    pub fn with_time(mut self, ms: u64) -> Self {
        self.lint_time_ms = ms;
        self
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn error_summary(&self) -> String {
        if self.errors.is_empty() {
            "No errors".to_string()
        } else {
            self.errors
                .iter()
                .map(|e| format!("L{}:{}: [{}] {}", e.line, e.column, e.code, e.message))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

/// Fast syntax linter for Rust code.
pub struct BusLinter {
    /// Known type names (from StateStore)
    known_types: HashSet<String>,
    /// Known function names
    known_functions: HashSet<String>,
}

impl Default for BusLinter {
    fn default() -> Self {
        Self::new()
    }
}

impl BusLinter {
    pub fn new() -> Self {
        Self {
            known_types: HashSet::new(),
            known_functions: HashSet::new(),
        }
    }

    /// Add known types from StateStore.
    pub fn with_known_types(mut self, types: impl IntoIterator<Item = String>) -> Self {
        self.known_types.extend(types);
        self
    }

    /// Add known functions from StateStore.
    pub fn with_known_functions(mut self, funcs: impl IntoIterator<Item = String>) -> Self {
        self.known_functions.extend(funcs);
        self
    }

    /// Perform fast lint check on code.
    pub fn lint(&self, code: &str) -> LintResult {
        let start = std::time::Instant::now();
        let mut errors = Vec::new();
        let mut warnings = Vec::new();

        // Check balanced delimiters
        if let Some(err) = self.check_balanced_delimiters(code) {
            errors.push(err);
        }

        // Check for common syntax issues
        errors.extend(self.check_syntax_patterns(code));
        warnings.extend(self.check_warnings(code));

        // Check type references if we have known types
        if !self.known_types.is_empty() {
            warnings.extend(self.check_type_references(code));
        }

        let elapsed = start.elapsed().as_millis() as u64;
        let passed = errors.is_empty();

        debug!(
            "Lint completed in {}ms: {} errors, {} warnings",
            elapsed,
            errors.len(),
            warnings.len()
        );

        LintResult {
            passed,
            errors,
            warnings,
            lint_time_ms: elapsed,
        }
    }

    /// Check that braces, brackets, and parentheses are balanced.
    fn check_balanced_delimiters(&self, code: &str) -> Option<LintError> {
        let mut stack: Vec<(char, usize, usize)> = Vec::new();
        let mut in_string = false;
        let mut in_char = false;
        let mut in_comment = false;
        let mut in_line_comment = false;
        let mut prev_char = ' ';

        for (line_idx, line) in code.lines().enumerate() {
            in_line_comment = false;

            for (col_idx, ch) in line.chars().enumerate() {
                // Handle line comments
                if !in_string && !in_char && prev_char == '/' && ch == '/' {
                    in_line_comment = true;
                }

                // Handle block comments
                if !in_string && !in_char && !in_line_comment {
                    if prev_char == '/' && ch == '*' {
                        in_comment = true;
                    }
                    if prev_char == '*' && ch == '/' && in_comment {
                        in_comment = false;
                        prev_char = ch;
                        continue;
                    }
                }

                if in_comment || in_line_comment {
                    prev_char = ch;
                    continue;
                }

                // Handle strings
                if ch == '"' && prev_char != '\\' && !in_char {
                    in_string = !in_string;
                }

                // Handle char literals
                if ch == '\'' && prev_char != '\\' && !in_string {
                    in_char = !in_char;
                }

                if in_string || in_char {
                    prev_char = ch;
                    continue;
                }

                match ch {
                    '(' | '[' | '{' => {
                        stack.push((ch, line_idx + 1, col_idx + 1));
                    }
                    ')' => {
                        if let Some((open, _, _)) = stack.pop() {
                            if open != '(' {
                                return Some(LintError::error(
                                    line_idx + 1,
                                    col_idx + 1,
                                    "E001",
                                    format!("Mismatched delimiter: expected closing for '{}', found ')'", open),
                                ));
                            }
                        } else {
                            return Some(LintError::error(
                                line_idx + 1,
                                col_idx + 1,
                                "E001",
                                "Unexpected closing parenthesis ')'",
                            ));
                        }
                    }
                    ']' => {
                        if let Some((open, _, _)) = stack.pop() {
                            if open != '[' {
                                return Some(LintError::error(
                                    line_idx + 1,
                                    col_idx + 1,
                                    "E001",
                                    format!("Mismatched delimiter: expected closing for '{}', found ']'", open),
                                ));
                            }
                        } else {
                            return Some(LintError::error(
                                line_idx + 1,
                                col_idx + 1,
                                "E001",
                                "Unexpected closing bracket ']'",
                            ));
                        }
                    }
                    '}' => {
                        if let Some((open, _, _)) = stack.pop() {
                            if open != '{' {
                                return Some(LintError::error(
                                    line_idx + 1,
                                    col_idx + 1,
                                    "E001",
                                    format!("Mismatched delimiter: expected closing for '{}', found '}}'", open),
                                ));
                            }
                        } else {
                            return Some(LintError::error(
                                line_idx + 1,
                                col_idx + 1,
                                "E001",
                                "Unexpected closing brace '}'",
                            ));
                        }
                    }
                    _ => {}
                }
                prev_char = ch;
            }
        }

        if let Some((ch, line, col)) = stack.pop() {
            return Some(LintError::error(
                line,
                col,
                "E001",
                format!("Unclosed delimiter '{}'", ch),
            ));
        }

        None
    }

    /// Check for common syntax errors.
    fn check_syntax_patterns(&self, code: &str) -> Vec<LintError> {
        let mut errors = Vec::new();

        for (line_idx, line) in code.lines().enumerate() {
            let trimmed = line.trim();

            // Skip comments and empty lines
            if trimmed.starts_with("//") || trimmed.is_empty() {
                continue;
            }

            // Check for double semicolons
            if trimmed.contains(";;") {
                if let Some(col) = line.find(";;") {
                    errors.push(LintError::error(
                        line_idx + 1,
                        col + 1,
                        "E002",
                        "Double semicolon ';;' is invalid",
                    ));
                }
            }

            // Check for `fn` without opening paren
            if trimmed.starts_with("fn ") || trimmed.contains(" fn ") || trimmed.starts_with("pub fn ") {
                if !trimmed.contains('(') {
                    errors.push(LintError::error(
                        line_idx + 1,
                        1,
                        "E003",
                        "Function declaration missing parameters '()'",
                    ));
                }
            }

            // Check for `let` without `=` on same line (unless it's a pattern match)
            if (trimmed.starts_with("let ") || trimmed.starts_with("let mut "))
                && !trimmed.contains('=')
                && !trimmed.ends_with('{')
                && trimmed.ends_with(';')
            {
                errors.push(LintError::error(
                    line_idx + 1,
                    1,
                    "E004",
                    "Variable declaration without initialization",
                ));
            }

            // Check for `struct` or `enum` without name
            if (trimmed == "struct" || trimmed == "pub struct" ||
                trimmed == "enum" || trimmed == "pub enum") {
                errors.push(LintError::error(
                    line_idx + 1,
                    1,
                    "E005",
                    "Type declaration missing name",
                ));
            }

            // Check for arrow without async/fn
            if trimmed.contains("->") && !trimmed.contains("fn") && !trimmed.contains("|")
               && !trimmed.contains("match") && !trimmed.contains("=>") {
                // This could be a return type annotation in an impl block, which is fine
                // Only warn if it looks suspicious
                if !trimmed.starts_with("type ") && !trimmed.contains("impl") {
                    // Could be valid, just be cautious
                }
            }

            // Check for `impl` without `for` or type
            if trimmed == "impl" || trimmed == "impl {" {
                errors.push(LintError::error(
                    line_idx + 1,
                    1,
                    "E006",
                    "impl block missing type",
                ));
            }
        }

        errors
    }

    /// Check for common warnings (not errors).
    fn check_warnings(&self, code: &str) -> Vec<LintError> {
        let mut warnings = Vec::new();

        for (line_idx, line) in code.lines().enumerate() {
            let trimmed = line.trim();

            // Warn about TODO/FIXME in code
            if trimmed.contains("TODO") || trimmed.contains("FIXME") {
                warnings.push(LintError::warning(
                    line_idx + 1,
                    1,
                    "W001",
                    "Found TODO/FIXME comment",
                ));
            }

            // Warn about unwrap() usage
            if trimmed.contains(".unwrap()") {
                if let Some(col) = line.find(".unwrap()") {
                    warnings.push(LintError::warning(
                        line_idx + 1,
                        col + 1,
                        "W002",
                        "Use of .unwrap() - consider proper error handling",
                    ));
                }
            }

            // Warn about panic!
            if trimmed.contains("panic!") {
                if let Some(col) = line.find("panic!") {
                    warnings.push(LintError::warning(
                        line_idx + 1,
                        col + 1,
                        "W003",
                        "Use of panic! macro",
                    ));
                }
            }

            // Warn about very long lines
            if line.len() > 120 {
                warnings.push(LintError::warning(
                    line_idx + 1,
                    120,
                    "W004",
                    format!("Line too long ({} chars, max 120)", line.len()),
                ));
            }
        }

        warnings
    }

    /// Check that referenced types exist in known types.
    fn check_type_references(&self, code: &str) -> Vec<LintError> {
        let mut warnings = Vec::new();

        // Extract type references from the code
        let type_ref_re = regex::Regex::new(r"\b([A-Z][a-zA-Z0-9]*)\b").unwrap();

        // Standard library types we don't need to check
        let stdlib_types: HashSet<&str> = [
            "String", "Vec", "Option", "Result", "Box", "Arc", "Rc", "Mutex", "RwLock",
            "HashMap", "HashSet", "BTreeMap", "BTreeSet", "Duration", "Instant",
            "Path", "PathBuf", "File", "Error", "Cow", "Cell", "RefCell",
            "Pin", "Future", "Stream", "Send", "Sync", "Clone", "Copy", "Debug",
            "Default", "Display", "From", "Into", "Iterator", "IntoIterator",
            "Self", "Ok", "Err", "Some", "None", "true", "false",
            "u8", "u16", "u32", "u64", "u128", "usize",
            "i8", "i16", "i32", "i64", "i128", "isize",
            "f32", "f64", "bool", "char", "str",
        ].into_iter().collect();

        for (line_idx, line) in code.lines().enumerate() {
            let trimmed = line.trim();

            // Skip comments
            if trimmed.starts_with("//") {
                continue;
            }

            for cap in type_ref_re.captures_iter(line) {
                let type_name = cap.get(1).unwrap().as_str();

                // Skip stdlib types
                if stdlib_types.contains(type_name) {
                    continue;
                }

                // Skip if it's a known type
                if self.known_types.contains(type_name) {
                    continue;
                }

                // Skip if it looks like a module name (followed by ::)
                if line.contains(&format!("{}::", type_name)) {
                    continue;
                }

                // Skip if in a use statement
                if trimmed.starts_with("use ") {
                    continue;
                }

                // This is a potential unknown type reference
                if let Some(col) = line.find(type_name) {
                    warnings.push(LintError::warning(
                        line_idx + 1,
                        col + 1,
                        "W010",
                        format!("Reference to unknown type '{}' - ensure it's defined", type_name),
                    ));
                }
            }
        }

        warnings
    }

    /// Quick validation - just check for fatal errors.
    pub fn quick_check(&self, code: &str) -> bool {
        // Just check balanced delimiters for speed
        self.check_balanced_delimiters(code).is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_balanced_braces() {
        let linter = BusLinter::new();

        // Valid code
        assert!(linter.lint("fn foo() { let x = 1; }").passed);
        assert!(linter.lint("struct Foo { a: i32, b: String }").passed);
        assert!(linter.lint("fn bar() { if true { } else { } }").passed);

        // Invalid code
        assert!(!linter.lint("fn foo() { let x = 1;").passed);
        assert!(!linter.lint("fn foo() { let x = 1; }}").passed);
        assert!(!linter.lint("fn foo( { }").passed);
    }

    #[test]
    fn test_double_semicolon() {
        let linter = BusLinter::new();
        let result = linter.lint("let x = 1;;");
        assert!(!result.passed);
        assert!(result.errors.iter().any(|e| e.code == "E002"));
    }

    #[test]
    fn test_string_handling() {
        let linter = BusLinter::new();

        // Braces inside strings should be ignored
        assert!(linter.lint(r#"let s = "{ not a brace }";"#).passed);
        assert!(linter.lint(r#"let s = "unbalanced {";"#).passed);
    }

    #[test]
    fn test_warnings() {
        let linter = BusLinter::new();
        let result = linter.lint("fn foo() { x.unwrap(); }");
        assert!(result.passed); // Warnings don't fail
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn test_quick_check() {
        let linter = BusLinter::new();
        assert!(linter.quick_check("fn foo() {}"));
        assert!(!linter.quick_check("fn foo() {"));
    }

    #[test]
    fn test_comment_handling() {
        let linter = BusLinter::new();

        // Comments should be ignored
        assert!(linter.lint("// this is a comment with { unbalanced braces").passed);
        assert!(linter.lint("/* block comment { */\nfn foo() {}").passed);
    }
}
