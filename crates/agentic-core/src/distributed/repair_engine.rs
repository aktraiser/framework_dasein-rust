//! Repair Engine - Framework-level error repair system.
//!
//! This module provides the missing pieces for effective LLM error correction:
//!
//! 1. **ErrorContextExtractor** - Extracts faulty code lines from error messages
//! 2. **RepairSuggestion** - Generates concrete fix suggestions based on patterns
//! 3. **FocusedPromptBuilder** - Builds targeted prompts showing only problem areas
//!
//! # Architecture
//!
//! ```text
//! Validation Error
//!       ↓
//! ErrorContextExtractor → Extracts line 34: "throw new Error(Invalid ${x})"
//!       ↓
//! RepairSuggestion     → Detects pattern, suggests: "`Invalid ${x}`"
//!       ↓
//! FocusedPromptBuilder → Builds prompt with ONLY the problem zone
//!       ↓
//! LLM sees exactly what to fix
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! use dasein_agentic_core::distributed::repair_engine::{
//!     ErrorContextExtractor, RepairSuggestion, FocusedPromptBuilder,
//! };
//!
//! let code = "...";
//! let errors = vec!["main.ts(34,5): error TS1005: ',' expected".to_string()];
//!
//! // Extract context
//! let extractor = ErrorContextExtractor::new();
//! let contexts = extractor.extract_all(code, &errors);
//!
//! // Generate repair suggestions
//! let suggestions = RepairSuggestion::from_contexts(&contexts);
//!
//! // Build focused prompt
//! let prompt = FocusedPromptBuilder::new()
//!     .with_contexts(&contexts)
//!     .with_suggestions(&suggestions)
//!     .with_code(code)
//!     .build();
//! ```

use regex::Regex;

use super::sandbox_validator::Language;

// ============================================================================
// ERROR CONTEXT EXTRACTOR
// ============================================================================

/// Extracted context for a single error.
#[derive(Debug, Clone)]
pub struct ErrorContext {
    /// Original error message
    pub error_message: String,
    /// File name (if detected)
    pub file: Option<String>,
    /// Line number where error occurred
    pub line: Option<u32>,
    /// Column number (if available)
    pub column: Option<u32>,
    /// The actual code line at that position
    pub code_line: Option<String>,
    /// Lines before (for context)
    pub lines_before: Vec<(u32, String)>,
    /// Lines after (for context)
    pub lines_after: Vec<(u32, String)>,
    /// Error code (e.g., "TS1005", "E0382")
    pub error_code: Option<String>,
    /// Detected problem pattern
    pub detected_pattern: Option<ErrorPattern>,
}

impl ErrorContext {
    /// Format the context for display in a prompt.
    pub fn format_for_prompt(&self) -> String {
        let mut output = String::new();

        // Header with location
        if let (Some(file), Some(line)) = (&self.file, self.line) {
            output.push_str(&format!(">>> ERROR at {}:{}", file, line));
            if let Some(col) = self.column {
                output.push_str(&format!(":{}", col));
            }
            output.push('\n');
        }

        // Error code and message
        if let Some(code) = &self.error_code {
            output.push_str(&format!("    Code: {}\n", code));
        }
        output.push_str(&format!(
            "    Message: {}\n",
            self.error_message
                .lines()
                .next()
                .unwrap_or(&self.error_message)
        ));
        output.push('\n');

        // Context lines before
        for (ln, content) in &self.lines_before {
            output.push_str(&format!("    {:4} │ {}\n", ln, content));
        }

        // The faulty line (highlighted)
        if let (Some(line_num), Some(code_line)) = (self.line, &self.code_line) {
            output.push_str(&format!("  → {:4} │ {}\n", line_num, code_line));

            // Show column indicator if available
            if let Some(col) = self.column {
                let spaces = " ".repeat(10 + col as usize);
                output.push_str(&format!("{}^\n", spaces));
            }
        }

        // Context lines after
        for (ln, content) in &self.lines_after {
            output.push_str(&format!("    {:4} │ {}\n", ln, content));
        }

        // Detected pattern and suggestion
        if let Some(pattern) = &self.detected_pattern {
            output.push('\n');
            output.push_str(&format!("    PROBLEM: {}\n", pattern.description));
            output.push_str(&format!("    FIX: {}\n", pattern.fix_instruction));
            if let Some(example) = &pattern.example_fix {
                output.push_str(&format!("    EXAMPLE: {}\n", example));
            }
        }

        output
    }
}

/// Detected error pattern (for pattern-based repair suggestions).
#[derive(Debug, Clone)]
pub struct ErrorPattern {
    /// Pattern identifier
    pub id: &'static str,
    /// Human-readable description
    pub description: String,
    /// Instruction for fixing
    pub fix_instruction: String,
    /// Example of correct code
    pub example_fix: Option<String>,
    /// Confidence (0.0 - 1.0)
    pub confidence: f32,
}

/// Extracts error context from code based on error messages.
pub struct ErrorContextExtractor {
    /// Pattern for TypeScript file:line:col
    ts_location_paren: Regex,
    ts_location_colon: Regex,
    /// Pattern for Rust file:line
    rust_location: Regex,
    /// Pattern for Python traceback
    python_location: Regex,
    /// Pattern for Go file:line
    go_location: Regex,
    /// Pattern for TypeScript error codes
    ts_error_code: Regex,
    /// Pattern for Rust error codes
    rust_error_code: Regex,
    /// Pattern detectors
    template_literal_pattern: Regex,
    missing_await_pattern: Regex,
}

impl Default for ErrorContextExtractor {
    fn default() -> Self {
        Self::new()
    }
}

impl ErrorContextExtractor {
    pub fn new() -> Self {
        Self {
            // TypeScript: main.ts(34,5) or main.ts:34
            ts_location_paren: Regex::new(r"(\w+\.tsx?)\((\d+),(\d+)\)").unwrap(),
            ts_location_colon: Regex::new(r"(\w+\.tsx?):(\d+)(?::(\d+))?").unwrap(),
            // Rust: src/main.rs:42
            rust_location: Regex::new(r"(\S+\.rs):(\d+)(?::(\d+))?").unwrap(),
            // Python: File "main.py", line 42
            python_location: Regex::new(r#"File "([^"]+\.py)", line (\d+)"#).unwrap(),
            // Go: main.go:42
            go_location: Regex::new(r"(\w+\.go):(\d+)(?::(\d+))?").unwrap(),
            // Error codes
            ts_error_code: Regex::new(r"TS(\d+)").unwrap(),
            rust_error_code: Regex::new(r"E(\d{4})").unwrap(),
            // Pattern detectors
            template_literal_pattern: Regex::new(r"\$\{[^}]+\}").unwrap(),
            missing_await_pattern: Regex::new(r"Promise\s*<").unwrap(),
        }
    }

    /// Extract context for all errors.
    pub fn extract_all(&self, code: &str, errors: &[String]) -> Vec<ErrorContext> {
        errors
            .iter()
            .filter_map(|err| self.extract(code, err))
            .collect()
    }

    /// Extract context for a single error.
    pub fn extract(&self, code: &str, error: &str) -> Option<ErrorContext> {
        let lines: Vec<&str> = code.lines().collect();

        // Try to extract location
        let (file, line, column) = self.extract_location(error);

        // Extract error code
        let error_code = self.extract_error_code(error);

        // Get the code line if we have a line number
        let code_line = line.and_then(|ln| lines.get((ln - 1) as usize).map(|s| (*s).to_string()));

        // Get context lines (2 before, 2 after)
        let lines_before = if let Some(ln) = line {
            self.get_context_lines(&lines, ln, -2, -1)
        } else {
            vec![]
        };

        let lines_after = if let Some(ln) = line {
            self.get_context_lines(&lines, ln, 1, 2)
        } else {
            vec![]
        };

        // Detect patterns
        let detected_pattern = code_line
            .as_ref()
            .and_then(|cl| self.detect_pattern(cl, error, &error_code));

        Some(ErrorContext {
            error_message: error.to_string(),
            file,
            line,
            column,
            code_line,
            lines_before,
            lines_after,
            error_code,
            detected_pattern,
        })
    }

    /// Extract file, line, column from error message.
    fn extract_location(&self, error: &str) -> (Option<String>, Option<u32>, Option<u32>) {
        // Try TypeScript patterns
        if let Some(caps) = self.ts_location_paren.captures(error) {
            return (
                Some(caps[1].to_string()),
                caps[2].parse().ok(),
                caps[3].parse().ok(),
            );
        }
        if let Some(caps) = self.ts_location_colon.captures(error) {
            return (
                Some(caps[1].to_string()),
                caps[2].parse().ok(),
                caps.get(3).and_then(|m| m.as_str().parse().ok()),
            );
        }

        // Try Rust pattern
        if let Some(caps) = self.rust_location.captures(error) {
            return (
                Some(caps[1].to_string()),
                caps[2].parse().ok(),
                caps.get(3).and_then(|m| m.as_str().parse().ok()),
            );
        }

        // Try Python pattern
        if let Some(caps) = self.python_location.captures(error) {
            return (Some(caps[1].to_string()), caps[2].parse().ok(), None);
        }

        // Try Go pattern
        if let Some(caps) = self.go_location.captures(error) {
            return (
                Some(caps[1].to_string()),
                caps[2].parse().ok(),
                caps.get(3).and_then(|m| m.as_str().parse().ok()),
            );
        }

        (None, None, None)
    }

    /// Extract error code from message.
    fn extract_error_code(&self, error: &str) -> Option<String> {
        // TypeScript
        if let Some(caps) = self.ts_error_code.captures(error) {
            return Some(format!("TS{}", &caps[1]));
        }

        // Rust
        if let Some(caps) = self.rust_error_code.captures(error) {
            return Some(format!("E{}", &caps[1]));
        }

        None
    }

    /// Get context lines around a target line.
    fn get_context_lines(
        &self,
        lines: &[&str],
        target_line: u32,
        from_offset: i32,
        to_offset: i32,
    ) -> Vec<(u32, String)> {
        let mut result = vec![];
        for offset in from_offset..=to_offset {
            let line_num = (target_line as i32 + offset) as u32;
            if line_num >= 1 && (line_num as usize) <= lines.len() {
                let content = lines[(line_num - 1) as usize].to_string();
                result.push((line_num, content));
            }
        }
        result
    }

    /// Detect error patterns in the code line.
    fn detect_pattern(
        &self,
        code_line: &str,
        error: &str,
        error_code: &Option<String>,
    ) -> Option<ErrorPattern> {
        let error_lower = error.to_lowercase();

        // Pattern 1: Template literal without backticks
        if self.template_literal_pattern.is_match(code_line) {
            let backtick_count = code_line.chars().filter(|&c| c == '`').count();

            // Check if ${} is outside backticks
            let has_unquoted_template = self.has_unquoted_template_literal(code_line);

            if backtick_count == 0 || has_unquoted_template {
                // This is THE common TypeScript mistake
                return Some(ErrorPattern {
                    id: "TEMPLATE_LITERAL_NO_BACKTICKS",
                    description: "Template literal ${...} used without backticks".to_string(),
                    fix_instruction: "Wrap the string with BACKTICKS (`) not quotes".to_string(),
                    example_fix: Some(self.suggest_backtick_fix(code_line)),
                    confidence: 0.95,
                });
            }
        }

        // Pattern 2: Missing await on Promise
        if error_lower.contains("promise")
            && (error_lower.contains("not assignable") || error_lower.contains("is not a function"))
        {
            if code_line.contains("Promise") || self.missing_await_pattern.is_match(code_line) {
                return Some(ErrorPattern {
                    id: "MISSING_AWAIT",
                    description: "Promise returned but not awaited".to_string(),
                    fix_instruction: "Add 'await' before the Promise-returning expression"
                        .to_string(),
                    example_fix: None,
                    confidence: 0.8,
                });
            }
        }

        // Pattern 3: TS1005 (syntax) with common causes
        if error_code.as_deref() == Some("TS1005") {
            // Could be missing comma, semicolon, or backticks
            if code_line.contains("${") && !code_line.contains('`') {
                return Some(ErrorPattern {
                    id: "TS1005_TEMPLATE_LITERAL",
                    description: "TS1005 caused by template literal without backticks".to_string(),
                    fix_instruction: "The ${} syntax requires backticks around the string"
                        .to_string(),
                    example_fix: Some(self.suggest_backtick_fix(code_line)),
                    confidence: 0.9,
                });
            }
        }

        // Pattern 4: Rust move errors
        if error_code
            .as_deref()
            .map_or(false, |c| c == "E0382" || c == "E0505")
        {
            return Some(ErrorPattern {
                id: "RUST_MOVE_ERROR",
                description: "Value moved or borrowed incorrectly".to_string(),
                fix_instruction: "Clone the value before using, or use references".to_string(),
                example_fix: Some("Use .clone() or Arc::clone(&val)".to_string()),
                confidence: 0.85,
            });
        }

        // Pattern 5: Python f-string issues
        if code_line.contains('{')
            && code_line.contains('}')
            && !code_line.trim_start().starts_with('f')
            && (code_line.contains('"') || code_line.contains('\''))
        {
            return Some(ErrorPattern {
                id: "PYTHON_MISSING_FSTRING",
                description: "String interpolation without f-prefix".to_string(),
                fix_instruction: "Add 'f' prefix before the string".to_string(),
                example_fix: Some("f\"text {variable}\"".to_string()),
                confidence: 0.75,
            });
        }

        None
    }

    /// Check if ${} appears outside of backticks.
    fn has_unquoted_template_literal(&self, line: &str) -> bool {
        let chars: Vec<char> = line.chars().collect();
        let mut in_backticks = false;
        let mut in_double_quotes = false;
        let mut in_single_quotes = false;

        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];

            // Track quote state (simplified, doesn't handle escapes perfectly)
            match c {
                '`' if !in_double_quotes && !in_single_quotes => in_backticks = !in_backticks,
                '"' if !in_backticks && !in_single_quotes => in_double_quotes = !in_double_quotes,
                '\'' if !in_backticks && !in_double_quotes => in_single_quotes = !in_single_quotes,
                _ => {}
            }

            // Check for ${ pattern
            if i + 1 < chars.len() && c == '$' && chars[i + 1] == '{' {
                // If not inside backticks, this is wrong
                if !in_backticks {
                    return true;
                }
            }

            i += 1;
        }

        false
    }

    /// Suggest a fix for missing backticks.
    fn suggest_backtick_fix(&self, code_line: &str) -> String {
        // Try to find the string that needs backticks
        // Common patterns: throw new Error(...), console.log(...), const x = ...

        let trimmed = code_line.trim();

        // Pattern: throw new Error(something ${var} something)
        if let Some(start) = trimmed.find("Error(") {
            let after_error = &trimmed[start + 6..];
            // Find the content
            if let Some(end) = after_error.rfind(')') {
                let content = &after_error[..end];
                // Check if it's not already backticked
                if !content.starts_with('`') {
                    let fixed = content.trim_matches(|c| c == '"' || c == '\'');
                    return format!("throw new Error(`{}`)", fixed);
                }
            }
        }

        // Generic suggestion
        "Wrap the string containing ${} with backticks (`), not quotes".to_string()
    }
}

// ============================================================================
// REPAIR SUGGESTION
// ============================================================================

/// A concrete repair suggestion.
#[derive(Debug, Clone)]
pub struct RepairSuggestion {
    /// Which line to fix
    pub line: Option<u32>,
    /// Original code
    pub original: String,
    /// Suggested replacement
    pub suggested: String,
    /// Explanation
    pub explanation: String,
    /// Confidence (0.0 - 1.0)
    pub confidence: f32,
}

impl RepairSuggestion {
    /// Generate suggestions from error contexts.
    pub fn from_contexts(contexts: &[ErrorContext]) -> Vec<Self> {
        contexts
            .iter()
            .filter_map(Self::from_context)
            .collect()
    }

    /// Check if this suggestion can be applied automatically (high confidence).
    pub fn can_auto_apply(&self) -> bool {
        self.confidence >= 0.9 && self.line.is_some() && !self.suggested.is_empty()
    }

    /// Generate a suggestion from a single context.
    pub fn from_context(ctx: &ErrorContext) -> Option<Self> {
        let pattern = ctx.detected_pattern.as_ref()?;
        let code_line = ctx.code_line.as_ref()?;

        match pattern.id {
            "TEMPLATE_LITERAL_NO_BACKTICKS" | "TS1005_TEMPLATE_LITERAL" => {
                let suggested = Self::fix_template_literal(code_line);
                Some(RepairSuggestion {
                    line: ctx.line,
                    original: code_line.clone(),
                    suggested,
                    explanation: "Template literals with ${} require backticks, not quotes"
                        .to_string(),
                    confidence: pattern.confidence,
                })
            }
            "MISSING_AWAIT" => {
                let suggested = Self::add_await(code_line);
                Some(RepairSuggestion {
                    line: ctx.line,
                    original: code_line.clone(),
                    suggested,
                    explanation: "Promise-returning functions must be awaited".to_string(),
                    confidence: pattern.confidence,
                })
            }
            _ => None,
        }
    }

    /// Fix template literal by adding backticks.
    fn fix_template_literal(line: &str) -> String {
        // Find patterns like: Error(text ${var} more) and convert to Error(`text ${var} more`)
        let mut result = line.to_string();

        // Pattern 1: Error(unquoted text)
        let re = Regex::new(r#"Error\(([^`"'][^)]*\$\{[^}]+\}[^)]*)\)"#).unwrap();
        if let Some(caps) = re.captures(line) {
            let content = &caps[1];
            result = line.replace(
                &format!("Error({})", content),
                &format!("Error(`{}`)", content),
            );
        }

        // Pattern 2: Error("text ${var}") - quotes instead of backticks
        let re2 = Regex::new(r#"Error\("([^"]*\$\{[^}]+\}[^"]*)"\)"#).unwrap();
        if let Some(caps) = re2.captures(line) {
            let content = &caps[1];
            result = line.replace(
                &format!("Error(\"{}\")", content),
                &format!("Error(`{}`)", content),
            );
        }

        // Pattern 3: Error('text ${var}') - single quotes
        let re3 = Regex::new(r"Error\('([^']*\$\{[^}]+\}[^']*)'\)").unwrap();
        if let Some(caps) = re3.captures(line) {
            let content = &caps[1];
            result = line.replace(
                &format!("Error('{}')", content),
                &format!("Error(`{}`)", content),
            );
        }

        result
    }

    /// Add await to an expression.
    fn add_await(line: &str) -> String {
        // Simple heuristic: add await before method calls that return Promise
        let trimmed = line.trim();

        // Pattern: const x = foo.bar() -> const x = await foo.bar()
        if trimmed.starts_with("const ") || trimmed.starts_with("let ") {
            if let Some(eq_pos) = trimmed.find(" = ") {
                let before = &trimmed[..eq_pos + 3];
                let after = &trimmed[eq_pos + 3..];
                if !after.trim_start().starts_with("await ") {
                    return format!("{}await {}", before, after);
                }
            }
        }

        // Pattern: return foo.bar() -> return await foo.bar()
        if trimmed.starts_with("return ") {
            let after = &trimmed[7..];
            if !after.trim_start().starts_with("await ") {
                return format!("return await {}", after);
            }
        }

        line.to_string()
    }
}

// ============================================================================
// SURGICAL REPAIR - Apply fixes directly without LLM
// ============================================================================

/// Applies high-confidence repairs directly to code without LLM intervention.
///
/// When we have 90%+ confidence in a fix (like adding backticks to template literals),
/// we apply it directly instead of asking the LLM to regenerate.
pub struct SurgicalRepair;

impl SurgicalRepair {
    /// Apply high-confidence repairs directly to the code.
    ///
    /// Returns Some(fixed_code) if any fixes were applied, None otherwise.
    pub fn apply(code: &str, suggestions: &[RepairSuggestion]) -> Option<String> {
        let auto_apply: Vec<_> = suggestions.iter().filter(|s| s.can_auto_apply()).collect();

        if auto_apply.is_empty() {
            return None;
        }

        let mut lines: Vec<String> = code.lines().map(std::string::ToString::to_string).collect();
        let mut applied_count = 0;

        for suggestion in auto_apply {
            if let Some(line_num) = suggestion.line {
                let idx = (line_num - 1) as usize;
                if idx < lines.len() {
                    let current_line = &lines[idx];

                    // Only apply if the original matches (avoid double-application)
                    if current_line.trim() == suggestion.original.trim() {
                        // Preserve indentation
                        let indent = current_line.len() - current_line.trim_start().len();
                        let indent_str: String = " ".repeat(indent);
                        lines[idx] = format!("{}{}", indent_str, suggestion.suggested.trim());
                        applied_count += 1;
                    }
                }
            }
        }

        if applied_count > 0 {
            Some(lines.join("\n"))
        } else {
            None
        }
    }

    /// Apply template literal fixes specifically.
    ///
    /// This is a targeted fix for the common pattern:
    /// `throw new Error(text ${var})` -> `throw new Error(\`text ${var}\`)`
    pub fn fix_template_literals(code: &str) -> (String, usize) {
        let mut result = String::new();
        let mut fix_count = 0;

        for line in code.lines() {
            let fixed_line = Self::fix_template_literal_line(line);
            if fixed_line != line {
                fix_count += 1;
            }
            result.push_str(&fixed_line);
            result.push('\n');
        }

        // Remove trailing newline if original didn't have one
        if !code.ends_with('\n') && result.ends_with('\n') {
            result.pop();
        }

        (result, fix_count)
    }

    /// Fix a single line with template literal issues.
    fn fix_template_literal_line(line: &str) -> String {
        let trimmed = line.trim();

        // Skip if already using backticks correctly
        if !trimmed.contains("${") {
            return line.to_string();
        }

        // Skip if already has backticks around the ${} part
        // Simple check: count backticks - if we have matched pairs around ${}, it's OK
        let has_proper_backticks = Self::has_proper_template_backticks(trimmed);
        if has_proper_backticks {
            return line.to_string();
        }

        // Get indentation
        let indent = line.len() - trimmed.len();
        let indent_str: String = " ".repeat(indent);

        // Try to fix common patterns

        // Pattern 1: throw new Error(text ${var} more text)
        // -> throw new Error(`text ${var} more text`)
        if let Some(fixed) = Self::fix_error_pattern(trimmed) {
            return format!("{}{}", indent_str, fixed);
        }

        // Pattern 2: console.log(text ${var})
        // -> console.log(`text ${var}`)
        if let Some(fixed) = Self::fix_function_call_pattern(trimmed, "console.log") {
            return format!("{}{}", indent_str, fixed);
        }

        // Pattern 3: const x = text ${var}
        // -> const x = `text ${var}`
        if let Some(fixed) = Self::fix_assignment_pattern(trimmed) {
            return format!("{}{}", indent_str, fixed);
        }

        // Pattern 4: "text ${var}" or 'text ${var}' -> `text ${var}`
        if let Some(fixed) = Self::fix_quoted_template(trimmed) {
            return format!("{}{}", indent_str, fixed);
        }

        // Couldn't fix automatically
        line.to_string()
    }

    /// Check if the line has proper backticks around ${} expressions.
    fn has_proper_template_backticks(line: &str) -> bool {
        let chars: Vec<char> = line.chars().collect();
        let mut in_backticks = false;

        for i in 0..chars.len() {
            if chars[i] == '`' {
                in_backticks = !in_backticks;
            }
            // Check for ${ pattern
            if i + 1 < chars.len() && chars[i] == '$' && chars[i + 1] == '{' {
                if !in_backticks {
                    return false; // Found ${} outside backticks
                }
            }
        }

        true
    }

    /// Fix: throw new Error(text ${var}) -> throw new Error(`text ${var}`)
    fn fix_error_pattern(line: &str) -> Option<String> {
        // Match patterns like: throw new Error(something ${var} something)
        // where the content is NOT wrapped in quotes or backticks

        let error_patterns = ["new Error(", "Error(", "throw new Error("];

        for pattern in error_patterns {
            if let Some(start_idx) = line.find(pattern) {
                let after_pattern = &line[start_idx + pattern.len()..];

                // Find matching closing paren
                if let Some(end_idx) = Self::find_matching_paren(after_pattern) {
                    let content = &after_pattern[..end_idx];

                    // Check if content contains ${} and is not already backticked
                    if content.contains("${") && !content.starts_with('`') {
                        // Remove existing quotes if present
                        let clean_content = content
                            .trim_start_matches('"')
                            .trim_start_matches('\'')
                            .trim_end_matches('"')
                            .trim_end_matches('\'');

                        let before = &line[..start_idx + pattern.len()];
                        let after = &after_pattern[end_idx..];

                        return Some(format!("{}`{}`{}", before, clean_content, after));
                    }
                }
            }
        }

        None
    }

    /// Fix function call patterns like console.log(text ${var})
    fn fix_function_call_pattern(line: &str, func_name: &str) -> Option<String> {
        let pattern = format!("{}(", func_name);
        if let Some(start_idx) = line.find(&pattern) {
            let after_pattern = &line[start_idx + pattern.len()..];

            if let Some(end_idx) = Self::find_matching_paren(after_pattern) {
                let content = &after_pattern[..end_idx];

                if content.contains("${") && !content.starts_with('`') {
                    let clean_content = content
                        .trim_start_matches('"')
                        .trim_start_matches('\'')
                        .trim_end_matches('"')
                        .trim_end_matches('\'');

                    let before = &line[..start_idx + pattern.len()];
                    let after = &after_pattern[end_idx..];

                    return Some(format!("{}`{}`{}", before, clean_content, after));
                }
            }
        }

        None
    }

    /// Fix: const x = text ${var} -> const x = `text ${var}`
    fn fix_assignment_pattern(line: &str) -> Option<String> {
        if let Some(eq_idx) = line.find(" = ") {
            let after_eq = line[eq_idx + 3..].trim();

            // Check if it's an unquoted template literal
            if after_eq.contains("${")
                && !after_eq.starts_with('`')
                && !after_eq.starts_with('"')
                && !after_eq.starts_with('\'')
            {
                // Find where the expression ends (semicolon, comma, or end of line)
                let end_chars = [';', ','];
                let expr_end = after_eq
                    .find(|c| end_chars.contains(&c))
                    .unwrap_or(after_eq.len());

                let expr = &after_eq[..expr_end];
                let suffix = &after_eq[expr_end..];

                let before = &line[..eq_idx + 3];
                return Some(format!("{}`{}`{}", before, expr.trim(), suffix));
            }
        }

        None
    }

    /// Fix: "text ${var}" or 'text ${var}' -> `text ${var}`
    fn fix_quoted_template(line: &str) -> Option<String> {
        // Look for patterns like "...${...}..." or '...${...}...'
        let mut result = line.to_string();
        let mut changed = false;

        // Fix double-quoted templates
        let re_double = regex::Regex::new(r#""([^"]*\$\{[^}]+\}[^"]*)""#).ok()?;
        if let Some(caps) = re_double.captures(&result) {
            let full_match = caps.get(0)?;
            let content = caps.get(1)?;
            let replacement = format!("`{}`", content.as_str());
            result = result.replacen(full_match.as_str(), &replacement, 1);
            changed = true;
        }

        // Fix single-quoted templates
        let re_single = regex::Regex::new(r"'([^']*\$\{[^}]+\}[^']*)'").ok()?;
        if let Some(caps) = re_single.captures(&result) {
            let full_match = caps.get(0)?;
            let content = caps.get(1)?;
            let replacement = format!("`{}`", content.as_str());
            result = result.replacen(full_match.as_str(), &replacement, 1);
            changed = true;
        }

        if changed {
            Some(result)
        } else {
            None
        }
    }

    /// Find the index of the matching closing parenthesis.
    fn find_matching_paren(s: &str) -> Option<usize> {
        let mut depth = 1;
        for (i, c) in s.chars().enumerate() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i);
                    }
                }
                _ => {}
            }
        }
        None
    }
}

// ============================================================================
// FOCUSED PROMPT BUILDER
// ============================================================================

/// Builds focused prompts that show only the problem areas.
pub struct FocusedPromptBuilder {
    contexts: Vec<ErrorContext>,
    suggestions: Vec<RepairSuggestion>,
    full_code: Option<String>,
    language: Language,
    task_description: Option<String>,
    _max_context_lines: usize,
}

impl Default for FocusedPromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusedPromptBuilder {
    pub fn new() -> Self {
        Self {
            contexts: vec![],
            suggestions: vec![],
            full_code: None,
            language: Language::TypeScript,
            task_description: None,
            _max_context_lines: 5,
        }
    }

    /// Set error contexts.
    pub fn with_contexts(mut self, contexts: &[ErrorContext]) -> Self {
        self.contexts = contexts.to_vec();
        self
    }

    /// Set repair suggestions.
    pub fn with_suggestions(mut self, suggestions: &[RepairSuggestion]) -> Self {
        self.suggestions = suggestions.to_vec();
        self
    }

    /// Set the full code (for reference sections).
    pub fn with_code(mut self, code: &str) -> Self {
        self.full_code = Some(code.to_string());
        self
    }

    /// Set the language.
    pub fn with_language(mut self, lang: Language) -> Self {
        self.language = lang;
        self
    }

    /// Set the original task description.
    pub fn with_task(mut self, task: &str) -> Self {
        self.task_description = Some(task.to_string());
        self
    }

    /// Build the focused prompt.
    pub fn build(&self) -> String {
        let mut prompt = String::new();

        // Header
        prompt.push_str("=== CODE REPAIR REQUIRED ===\n\n");

        // Show task if available (brief)
        if let Some(task) = &self.task_description {
            let brief = task.lines().take(5).collect::<Vec<_>>().join("\n");
            prompt.push_str(&format!("TASK: {}\n\n", brief));
        }

        // Show each error context with its suggestion
        prompt.push_str("=== ERRORS TO FIX ===\n\n");

        for (i, ctx) in self.contexts.iter().enumerate() {
            prompt.push_str(&format!("--- Error {} ---\n", i + 1));
            prompt.push_str(&ctx.format_for_prompt());

            // Add repair suggestion if available
            if let Some(suggestion) = self.suggestions.iter().find(|s| s.line == ctx.line) {
                prompt.push('\n');
                prompt.push_str("    REPAIR SUGGESTION:\n");
                prompt.push_str(&format!("    Current:  {}\n", suggestion.original.trim()));
                prompt.push_str(&format!("    Fix to:   {}\n", suggestion.suggested.trim()));
                prompt.push_str(&format!("    Why: {}\n", suggestion.explanation));
            }

            prompt.push_str("\n\n");
        }

        // Language-specific critical rules
        prompt.push_str("=== SYNTAX RULES ===\n\n");
        match self.language {
            Language::TypeScript | Language::JavaScript => {
                prompt.push_str("TypeScript/JavaScript:\n");
                prompt.push_str("  - Template literals: `text ${var}` (BACKTICKS required)\n");
                prompt.push_str(
                    "  - WRONG: \"text ${var}\" or 'text ${var}' or Error(text ${var})\n",
                );
                prompt.push_str("  - RIGHT: `text ${var}` or Error(`text ${var}`)\n");
            }
            Language::Python => {
                prompt.push_str("Python:\n");
                prompt.push_str("  - f-strings: f\"text {var}\" (f prefix required)\n");
                prompt.push_str("  - WRONG: \"text {var}\"\n");
                prompt.push_str("  - RIGHT: f\"text {var}\"\n");
            }
            Language::Rust => {
                prompt.push_str("Rust:\n");
                prompt.push_str("  - format!: format!(\"text {}\", var)\n");
                prompt.push_str("  - Clone before move: val.clone()\n");
                prompt.push_str("  - Arc clone: Arc::clone(&val)\n");
            }
            Language::Go => {
                prompt.push_str("Go:\n");
                prompt.push_str("  - fmt.Sprintf: fmt.Sprintf(\"text %v\", var)\n");
            }
            _ => {}
        }

        prompt.push_str("\n=== INSTRUCTIONS ===\n\n");
        prompt.push_str("1. Fix ONLY the errors shown above\n");
        prompt.push_str("2. Apply the repair suggestions where provided\n");
        prompt.push_str("3. Return the COMPLETE corrected code\n");
        prompt.push_str("4. NO markdown, NO explanations\n");

        // Add the full code at the end (for context)
        if let Some(code) = &self.full_code {
            prompt.push_str("\n=== YOUR CODE (fix and return complete) ===\n\n");
            prompt.push_str(code);
        }

        prompt
    }

    /// Build a minimal prompt focusing on just the fixes needed.
    pub fn build_minimal(&self) -> String {
        let mut prompt = String::new();

        prompt.push_str("FIX THESE SPECIFIC ISSUES:\n\n");

        for suggestion in &self.suggestions {
            if let Some(line) = suggestion.line {
                prompt.push_str(&format!("Line {}: Change\n", line));
                prompt.push_str(&format!("  FROM: {}\n", suggestion.original.trim()));
                prompt.push_str(&format!("  TO:   {}\n", suggestion.suggested.trim()));
                prompt.push_str(&format!("  ({}))\n\n", suggestion.explanation));
            }
        }

        prompt.push_str("Apply these fixes and return the complete corrected code.\n");

        prompt
    }
}

// ============================================================================
// REPAIR PIPELINE INTEGRATION
// ============================================================================

/// Result of the repair engine analysis.
#[derive(Debug, Clone)]
pub struct RepairAnalysis {
    /// Extracted error contexts
    pub contexts: Vec<ErrorContext>,
    /// Generated repair suggestions
    pub suggestions: Vec<RepairSuggestion>,
    /// Built prompt for the LLM
    pub prompt: String,
    /// Confidence score (average of suggestions)
    pub confidence: f32,
    /// Whether repairs could be automatically suggested
    pub has_suggestions: bool,
    /// Surgically repaired code (if high-confidence fixes were applied)
    pub surgical_fix: Option<String>,
    /// Number of lines fixed surgically
    pub surgical_fix_count: usize,
}

/// Main entry point for the repair engine.
pub struct RepairEngine {
    extractor: ErrorContextExtractor,
}

impl Default for RepairEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RepairEngine {
    pub fn new() -> Self {
        Self {
            extractor: ErrorContextExtractor::new(),
        }
    }

    /// Analyze errors and generate repair suggestions.
    pub fn analyze(
        &self,
        code: &str,
        errors: &[String],
        language: Language,
        task: Option<&str>,
    ) -> RepairAnalysis {
        // Extract contexts
        let contexts = self.extractor.extract_all(code, errors);

        // Generate suggestions
        let suggestions = RepairSuggestion::from_contexts(&contexts);

        // Build focused prompt
        let mut builder = FocusedPromptBuilder::new()
            .with_contexts(&contexts)
            .with_suggestions(&suggestions)
            .with_code(code)
            .with_language(language);

        if let Some(t) = task {
            builder = builder.with_task(t);
        }

        let prompt = builder.build();

        // Calculate confidence
        let confidence = if suggestions.is_empty() {
            0.0
        } else {
            suggestions.iter().map(|s| s.confidence).sum::<f32>() / suggestions.len() as f32
        };

        // Try surgical repair for high-confidence fixes
        let (surgical_fix, surgical_fix_count) = if confidence >= 0.9 && !code.is_empty() {
            // First try applying suggestions directly
            if let Some(fixed) = SurgicalRepair::apply(code, &suggestions) {
                let count = suggestions.iter().filter(|s| s.can_auto_apply()).count();
                (Some(fixed), count)
            } else {
                // Fallback to template literal specific fix
                match language {
                    Language::TypeScript | Language::JavaScript => {
                        let (fixed, count) = SurgicalRepair::fix_template_literals(code);
                        if count > 0 {
                            (Some(fixed), count)
                        } else {
                            (None, 0)
                        }
                    }
                    _ => (None, 0),
                }
            }
        } else {
            (None, 0)
        };

        RepairAnalysis {
            contexts,
            suggestions: suggestions.clone(),
            prompt,
            confidence,
            has_suggestions: !suggestions.is_empty(),
            surgical_fix,
            surgical_fix_count,
        }
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_typescript_location() {
        let extractor = ErrorContextExtractor::new();

        let code = r#"function test() {
    const x = 1;
    throw new Error(Invalid value ${x});
    return x;
}"#;

        let error = "main.ts(3,22): error TS1005: ',' expected";
        let ctx = extractor.extract(code, error).unwrap();

        assert_eq!(ctx.file, Some("main.ts".to_string()));
        assert_eq!(ctx.line, Some(3));
        assert_eq!(ctx.column, Some(22));
        assert!(ctx.code_line.as_ref().unwrap().contains("Error"));
        assert_eq!(ctx.error_code, Some("TS1005".to_string()));
    }

    #[test]
    fn test_detect_template_literal_pattern() {
        let extractor = ErrorContextExtractor::new();

        let code = r#"throw new Error(Invalid state ${state});"#;
        let error = "main.ts(1,20): error TS1005: ',' expected";

        let ctx = extractor.extract(code, error).unwrap();
        assert!(ctx.detected_pattern.is_some());
        assert_eq!(
            ctx.detected_pattern.as_ref().unwrap().id,
            "TEMPLATE_LITERAL_NO_BACKTICKS"
        );
    }

    #[test]
    fn test_repair_suggestion() {
        let extractor = ErrorContextExtractor::new();

        let code = r#"throw new Error(Invalid: ${x});"#;
        let error = "main.ts(1,20): error TS1005: ',' expected";

        let ctx = extractor.extract(code, error).unwrap();
        let suggestions = RepairSuggestion::from_contexts(&[ctx]);

        assert!(!suggestions.is_empty());
        assert!(suggestions[0].suggested.contains('`'));
    }

    #[test]
    fn test_focused_prompt_builder() {
        let contexts = vec![ErrorContext {
            error_message: "TS1005: ',' expected".to_string(),
            file: Some("main.ts".to_string()),
            line: Some(5),
            column: Some(20),
            code_line: Some("throw new Error(test ${x});".to_string()),
            lines_before: vec![(4, "const x = 1;".to_string())],
            lines_after: vec![(6, "return x;".to_string())],
            error_code: Some("TS1005".to_string()),
            detected_pattern: Some(ErrorPattern {
                id: "TEMPLATE_LITERAL_NO_BACKTICKS",
                description: "Missing backticks".to_string(),
                fix_instruction: "Add backticks".to_string(),
                example_fix: None,
                confidence: 0.9,
            }),
        }];

        let prompt = FocusedPromptBuilder::new()
            .with_contexts(&contexts)
            .with_language(Language::TypeScript)
            .build();

        assert!(prompt.contains("Error 1"));
        assert!(prompt.contains("main.ts:5"));
        assert!(prompt.contains("BACKTICKS"));
    }

    #[test]
    fn test_repair_engine() {
        let engine = RepairEngine::new();

        let code = r#"function test() {
    throw new Error(Bad value ${x});
}"#;
        let errors = vec!["main.ts(2,22): error TS1005: ',' expected".to_string()];

        let analysis = engine.analyze(code, &errors, Language::TypeScript, None);

        assert!(analysis.has_suggestions);
        assert!(analysis.confidence > 0.5);
        assert!(analysis.prompt.contains("BACKTICKS"));
    }
}
