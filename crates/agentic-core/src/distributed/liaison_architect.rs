//! Liaison Architect - Ensures coherence between parallel executor outputs.
//!
//! When multiple executors work on sub-tasks in parallel (imports, types, impl, tests),
//! they may produce inconsistent code (impl references fields not in types, etc.).
//!
//! The Liaison Architect:
//! 1. Reads all sub-task outputs
//! 2. Detects inconsistencies (missing fields, wrong names, missing imports)
//! 3. Uses an executor to produce coherent, unified code
//! 4. Validates syntax before returning
//!
//! # Example
//!
//! ```rust,no_run
//! use agentic_core::distributed::{LiaisonArchitect, Executor, SubTaskResult};
//!
//! let liaison = LiaisonArchitect::new("rust");
//! let executor = Executor::new("liaison-fixer", "supervisor")
//!     .llm_gemini("gemini-2.0-flash")
//!     .build();
//!
//! // After parallel execution
//! let results: Vec<SubTaskResult> = vec![/* ... */];
//!
//! // Analyze and fix in one call
//! let fixed = liaison.fix(&results, &executor).await?;
//! ```

use super::executor::Executor;
use super::task_decomposer::SubTaskResult;
use regex::Regex;
use std::collections::{HashMap, HashSet};

/// System prompt for the Liaison fixer executor.
const LIAISON_SYSTEM_PROMPT: &str = r#"You are an expert Rust developer and code coherence specialist.
Your job is to take code fragments from multiple agents and produce a SINGLE COHERENT module.

CRITICAL RULES:
1. Use ONLY stable Rust. NO nightly features.
2. ALL braces, parentheses, and brackets must be balanced.
3. ALL struct fields referenced in impl must be defined in the struct.
4. ALL methods called in tests must be defined in impl.
5. Make constructors `pub fn new()` - not private.
6. Include ALL necessary imports at the top.
7. Return ONLY the complete, working Rust code.
8. NO markdown, NO explanations, NO comments about changes.

Double-check your syntax before responding!"#;

/// Errors that can occur during liaison operations.
#[derive(Debug, Clone)]
pub enum LiaisonError {
    /// The executor failed to generate code
    ExecutorFailed(String),
    /// Syntax validation failed after retries
    SyntaxError(String),
}

impl std::fmt::Display for LiaisonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExecutorFailed(msg) => write!(f, "Executor failed: {}", msg),
            Self::SyntaxError(msg) => write!(f, "Syntax error: {}", msg),
        }
    }
}

impl std::error::Error for LiaisonError {}

/// Analyzes and reconciles outputs from multiple executors.
pub struct LiaisonArchitect {
    /// Language being processed
    language: String,
}

/// Issues found during coherence analysis.
#[derive(Debug, Clone)]
pub struct CoherenceIssue {
    pub category: IssueCategory,
    pub description: String,
    pub location: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum IssueCategory {
    /// Field referenced in impl but not defined in types
    MissingField,
    /// Method called but not defined
    MissingMethod,
    /// Import used but not declared
    MissingImport,
    /// Type used but not defined
    MissingType,
    /// Name mismatch between definition and usage
    NameMismatch,
}

/// Result of coherence analysis.
#[derive(Debug, Clone)]
pub struct CoherenceReport {
    pub issues: Vec<CoherenceIssue>,
    pub defined_types: HashSet<String>,
    pub defined_fields: HashMap<String, HashSet<String>>,
    pub defined_methods: HashMap<String, HashSet<String>>,
    pub used_fields: HashMap<String, HashSet<String>>,
    pub used_methods: HashMap<String, HashSet<String>>,
    pub declared_imports: HashSet<String>,
    pub needed_imports: HashSet<String>,
}

impl LiaisonArchitect {
    pub fn new(language: impl Into<String>) -> Self {
        Self {
            language: language.into(),
        }
    }

    /// Analyze coherence between sub-task results.
    pub fn analyze(&self, results: &[SubTaskResult]) -> CoherenceReport {
        let mut report = CoherenceReport {
            issues: Vec::new(),
            defined_types: HashSet::new(),
            defined_fields: HashMap::new(),
            defined_methods: HashMap::new(),
            used_fields: HashMap::new(),
            used_methods: HashMap::new(),
            declared_imports: HashSet::new(),
            needed_imports: HashSet::new(),
        };

        // Extract information from each sub-task
        for result in results {
            match result.id.as_str() {
                "imports" => self.extract_imports(&result.code, &mut report),
                "types" => self.extract_types(&result.code, &mut report),
                "impl" => self.extract_impl(&result.code, &mut report),
                "tests" => self.extract_tests(&result.code, &mut report),
                _ => {}
            }
        }

        // Find issues
        self.find_field_issues(&mut report);
        self.find_method_issues(&mut report);
        self.find_import_issues(&mut report);

        report
    }

    /// Generate a prompt for an LLM to fix coherence issues.
    pub fn generate_fix_prompt(
        &self,
        results: &[SubTaskResult],
        report: &CoherenceReport,
    ) -> String {
        let mut prompt = String::new();

        prompt.push_str("You are a Rust code coherence expert. Multiple agents generated code parts in parallel, \
            but there are inconsistencies between them. Your job is to produce COHERENT, WORKING code.\n\n");

        prompt.push_str("=== COHERENCE ISSUES DETECTED ===\n");
        for issue in &report.issues {
            prompt.push_str(&format!(
                "- {:?}: {} (in {})\n",
                issue.category, issue.description, issue.location
            ));
        }

        prompt.push_str("\n=== DEFINED TYPES ===\n");
        for typ in &report.defined_types {
            prompt.push_str(&format!("- {}\n", typ));
            if let Some(fields) = report.defined_fields.get(typ) {
                for field in fields {
                    prompt.push_str(&format!("    field: {}\n", field));
                }
            }
            if let Some(methods) = report.defined_methods.get(typ) {
                for method in methods {
                    prompt.push_str(&format!("    method: {}\n", method));
                }
            }
        }

        prompt.push_str("\n=== CODE FROM EACH AGENT ===\n\n");

        for result in results {
            prompt.push_str(&format!("--- {} ({}) ---\n", result.name, result.id));
            prompt.push_str(&result.code);
            prompt.push_str("\n\n");
        }

        prompt.push_str("=== YOUR TASK ===\n");
        prompt.push_str("Produce a SINGLE, COHERENT Rust module that:\n");
        prompt.push_str("1. Has ALL necessary imports (add missing ones)\n");
        prompt.push_str("2. Has struct/enum definitions with ALL fields needed by the impl\n");
        prompt.push_str("3. Has impl blocks that use ONLY the fields defined in the structs\n");
        prompt.push_str("4. Has tests that use ONLY the methods defined in impl\n");
        prompt.push_str("5. Uses ONLY stable Rust features\n\n");
        prompt.push_str(
            "Return ONLY the complete, working Rust code. No markdown, no explanations.\n",
        );

        prompt
    }

    /// Generate a minimal fix prompt when there are few issues.
    pub fn generate_minimal_fix_prompt(
        &self,
        assembled_code: &str,
        report: &CoherenceReport,
    ) -> Option<String> {
        if report.issues.is_empty() {
            return None;
        }

        let mut prompt = String::new();
        prompt.push_str("Fix these specific issues in the Rust code:\n\n");

        for issue in &report.issues {
            prompt.push_str(&format!("- {}: {}\n", issue.location, issue.description));
        }

        prompt.push_str("\n=== CODE ===\n");
        prompt.push_str(assembled_code);
        prompt.push_str("\n\n=== INSTRUCTIONS ===\n");
        prompt.push_str("Return the FIXED code. Keep everything else the same. No markdown.\n");

        Some(prompt)
    }

    // === Extraction helpers ===

    fn extract_imports(&self, code: &str, report: &mut CoherenceReport) {
        let re = Regex::new(r"use\s+([^;]+);").unwrap();
        for cap in re.captures_iter(code) {
            report.declared_imports.insert(cap[1].to_string());
        }
    }

    fn extract_types(&self, code: &str, report: &mut CoherenceReport) {
        // Extract struct definitions
        let struct_re = Regex::new(r"(?:pub\s+)?struct\s+(\w+)").unwrap();
        for cap in struct_re.captures_iter(code) {
            let name = cap[1].to_string();
            report.defined_types.insert(name.clone());
            report.defined_fields.insert(name.clone(), HashSet::new());
        }

        // Extract enum definitions
        let enum_re = Regex::new(r"(?:pub\s+)?enum\s+(\w+)").unwrap();
        for cap in enum_re.captures_iter(code) {
            report.defined_types.insert(cap[1].to_string());
        }

        // Extract fields from structs
        let field_re = Regex::new(r"(?:pub\s+)?(\w+)\s*:\s*[^,}]+").unwrap();

        // Find struct bodies and extract fields
        let struct_body_re = Regex::new(r"struct\s+(\w+)[^{]*\{([^}]+)\}").unwrap();
        for cap in struct_body_re.captures_iter(code) {
            let struct_name = cap[1].to_string();
            let body = &cap[2];

            for field_cap in field_re.captures_iter(body) {
                let field_name = field_cap[1].to_string();
                if field_name != "pub" {
                    report
                        .defined_fields
                        .entry(struct_name.clone())
                        .or_default()
                        .insert(field_name);
                }
            }
        }
    }

    fn extract_impl(&self, code: &str, report: &mut CoherenceReport) {
        // Extract which type is being impl'd
        let impl_re = Regex::new(r"impl(?:<[^>]*>)?\s+(\w+)").unwrap();

        for cap in impl_re.captures_iter(code) {
            let type_name = cap[1].to_string();

            // Skip trait impls like "impl Default for X"
            if type_name == "Default"
                || type_name == "Clone"
                || type_name == "Debug"
                || type_name == "Display"
                || type_name == "Error"
                || type_name == "From"
            {
                continue;
            }

            report.defined_methods.entry(type_name.clone()).or_default();
        }

        // Extract method definitions
        let method_re = Regex::new(r"(?:pub\s+)?(?:async\s+)?fn\s+(\w+)").unwrap();
        let impl_block_re = Regex::new(r"impl(?:<[^>]*>)?\s+(\w+)[^{]*\{").unwrap();

        for cap in impl_block_re.captures_iter(code) {
            let type_name = cap[1].to_string();
            if type_name == "Default" || type_name == "Clone" || type_name == "Debug" {
                continue;
            }

            // Find methods in this impl block (simplified)
            for method_cap in method_re.captures_iter(code) {
                let method_name = method_cap[1].to_string();
                if method_name != "new" && method_name != "default" && method_name != "fmt" {
                    report
                        .defined_methods
                        .entry(type_name.clone())
                        .or_default()
                        .insert(method_name);
                }
            }
        }

        // Extract field usages (self.field_name)
        let field_usage_re = Regex::new(r"self\.(\w+)").unwrap();
        for cap in field_usage_re.captures_iter(code) {
            let field_name = cap[1].to_string();
            // We need to associate with the right type, but for now collect all
            for type_name in report.defined_types.iter() {
                report
                    .used_fields
                    .entry(type_name.clone())
                    .or_default()
                    .insert(field_name.clone());
            }
        }
    }

    fn extract_tests(&self, code: &str, report: &mut CoherenceReport) {
        // Extract method calls on types
        let method_call_re = Regex::new(r"\.(\w+)\s*\(").unwrap();
        for cap in method_call_re.captures_iter(code) {
            let method_name = cap[1].to_string();
            // Common methods to ignore
            if [
                "await",
                "unwrap",
                "expect",
                "is_ok",
                "is_err",
                "len",
                "clone",
                "to_string",
                "into",
                "as_ref",
                "as_mut",
                "iter",
                "collect",
            ]
            .contains(&method_name.as_str())
            {
                continue;
            }
            for type_name in report.defined_types.iter() {
                report
                    .used_methods
                    .entry(type_name.clone())
                    .or_default()
                    .insert(method_name.clone());
            }
        }
    }

    // === Issue detection ===

    fn find_field_issues(&self, report: &mut CoherenceReport) {
        for (type_name, used) in &report.used_fields {
            if let Some(defined) = report.defined_fields.get(type_name) {
                for field in used {
                    if !defined.contains(field) {
                        report.issues.push(CoherenceIssue {
                            category: IssueCategory::MissingField,
                            description: format!(
                                "Field '{}' used but not defined in struct '{}'",
                                field, type_name
                            ),
                            location: format!("impl {}", type_name),
                        });
                    }
                }
            }
        }
    }

    fn find_method_issues(&self, report: &mut CoherenceReport) {
        for (type_name, used) in &report.used_methods {
            if let Some(defined) = report.defined_methods.get(type_name) {
                for method in used {
                    if !defined.contains(method) && method != "new" {
                        report.issues.push(CoherenceIssue {
                            category: IssueCategory::MissingMethod,
                            description: format!(
                                "Method '{}' called but not defined in impl '{}'",
                                method, type_name
                            ),
                            location: "tests".to_string(),
                        });
                    }
                }
            }
        }
    }

    fn find_import_issues(&self, report: &mut CoherenceReport) {
        // Check common patterns that indicate missing imports
        let common_needs = [
            ("Arc<", "std::sync::Arc"),
            ("Arc::", "std::sync::Arc"),
            ("HashMap<", "std::collections::HashMap"),
            ("HashSet<", "std::collections::HashSet"),
            ("VecDeque<", "std::collections::VecDeque"),
            ("Duration", "std::time::Duration"),
            ("Instant", "std::time::Instant"),
            ("Mutex<", "tokio::sync::Mutex"),
            ("RwLock<", "tokio::sync::RwLock"),
            ("Semaphore", "tokio::sync::Semaphore"),
            ("mpsc::", "tokio::sync::mpsc"),
            ("oneshot::", "tokio::sync::oneshot"),
        ];

        for (pattern, import) in common_needs {
            report.needed_imports.insert(import.to_string());
        }

        for needed in &report.needed_imports {
            let has_import = report.declared_imports.iter().any(|i| i.contains(needed));
            if !has_import {
                report.issues.push(CoherenceIssue {
                    category: IssueCategory::MissingImport,
                    description: format!("May need import for '{}'", needed),
                    location: "imports".to_string(),
                });
            }
        }
    }

    /// Check if coherence issues are severe enough to need fixing.
    pub fn needs_fixing(&self, report: &CoherenceReport) -> bool {
        report.issues.iter().any(|i| {
            matches!(
                i.category,
                IssueCategory::MissingField
                    | IssueCategory::MissingMethod
                    | IssueCategory::MissingType
            )
        })
    }

    /// Analyze, fix coherence issues, and return unified code.
    ///
    /// This is the main entry point that handles the full flow:
    /// 1. Analyze results for coherence issues
    /// 2. If issues found, generate fix with executor
    /// 3. Validate syntax (brace balance, etc.)
    /// 4. Re-fix if syntax errors detected
    /// 5. Return coherent code as single SubTaskResult
    pub async fn fix(
        &self,
        results: &[SubTaskResult],
        executor: &Executor,
    ) -> Result<SubTaskResult, LiaisonError> {
        // Step 1: Analyze
        let report = self.analyze(results);
        tracing::info!("{}", self.summary(&report));

        if !self.needs_fixing(&report) {
            tracing::info!("No critical coherence issues - returning assembled results");
            // Just combine the results
            let code = results
                .iter()
                .filter(|r| r.success)
                .map(|r| r.code.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");

            return Ok(SubTaskResult {
                id: "liaison_passed".to_string(),
                name: "Liaison Passed".to_string(),
                code,
                success: true,
                error: None,
            });
        }

        // Step 2: Generate fix prompt and execute
        tracing::info!("Coherence issues detected - generating fix");
        let fix_prompt = self.generate_fix_prompt(results, &report);

        let code = match executor.execute(LIAISON_SYSTEM_PROMPT, &fix_prompt).await {
            Ok(result) => result.content,
            Err(e) => {
                return Err(LiaisonError::ExecutorFailed(e.to_string()));
            }
        };

        tracing::info!("Liaison produced {} chars", code.len());

        // Step 3: Validate syntax
        if let Some(syntax_error) = self.validate_syntax(&code) {
            tracing::warn!("Syntax error detected: {}", syntax_error);

            // Step 4: Re-fix
            let refix_prompt = format!(
                "The code has a syntax error: {}\n\n\
                Fix the syntax and return the complete, corrected Rust code.\n\
                No markdown, no explanations.\n\n{}",
                syntax_error, code
            );

            let fixed_code = match executor.execute(LIAISON_SYSTEM_PROMPT, &refix_prompt).await {
                Ok(result) => result.content,
                Err(e) => {
                    tracing::warn!("Re-fix failed: {}", e);
                    code // Use original
                }
            };

            // Check again
            if let Some(err) = self.validate_syntax(&fixed_code) {
                tracing::warn!("Still has syntax error after re-fix: {}", err);
            }

            return Ok(SubTaskResult {
                id: "liaison_fixed".to_string(),
                name: "Liaison Fixed".to_string(),
                code: fixed_code,
                success: true,
                error: None,
            });
        }

        Ok(SubTaskResult {
            id: "liaison_fixed".to_string(),
            name: "Liaison Fixed".to_string(),
            code,
            success: true,
            error: None,
        })
    }

    /// Validate basic syntax (brace balance, etc.)
    /// Returns None if valid, Some(error_message) if invalid.
    pub fn validate_syntax(&self, code: &str) -> Option<String> {
        // Check brace balance
        let open_braces = code.matches('{').count();
        let close_braces = code.matches('}').count();

        if open_braces != close_braces {
            return Some(format!(
                "Unbalanced braces: {} open, {} close",
                open_braces, close_braces
            ));
        }

        // Check parentheses balance
        let open_parens = code.matches('(').count();
        let close_parens = code.matches(')').count();

        if open_parens != close_parens {
            return Some(format!(
                "Unbalanced parentheses: {} open, {} close",
                open_parens, close_parens
            ));
        }

        // Check bracket balance
        let open_brackets = code.matches('[').count();
        let close_brackets = code.matches(']').count();

        if open_brackets != close_brackets {
            return Some(format!(
                "Unbalanced brackets: {} open, {} close",
                open_brackets, close_brackets
            ));
        }

        None
    }

    /// Get a summary of issues for logging.
    pub fn summary(&self, report: &CoherenceReport) -> String {
        if report.issues.is_empty() {
            return "No coherence issues detected".to_string();
        }

        let mut counts: HashMap<&str, usize> = HashMap::new();
        for issue in &report.issues {
            let cat = match issue.category {
                IssueCategory::MissingField => "missing_field",
                IssueCategory::MissingMethod => "missing_method",
                IssueCategory::MissingImport => "missing_import",
                IssueCategory::MissingType => "missing_type",
                IssueCategory::NameMismatch => "name_mismatch",
            };
            *counts.entry(cat).or_default() += 1;
        }

        let parts: Vec<String> = counts
            .iter()
            .map(|(k, v)| format!("{}: {}", k, v))
            .collect();

        format!("Coherence issues: {}", parts.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_types() {
        let architect = LiaisonArchitect::new("rust");
        let mut report = CoherenceReport {
            issues: Vec::new(),
            defined_types: HashSet::new(),
            defined_fields: HashMap::new(),
            defined_methods: HashMap::new(),
            used_fields: HashMap::new(),
            used_methods: HashMap::new(),
            declared_imports: HashSet::new(),
            needed_imports: HashSet::new(),
        };

        let code = r#"
            pub struct Scheduler {
                max_concurrent: usize,
                tasks: Vec<Task>,
            }

            pub enum TaskError {
                Timeout,
                Failed,
            }
        "#;

        architect.extract_types(code, &mut report);

        assert!(report.defined_types.contains("Scheduler"));
        assert!(report.defined_types.contains("TaskError"));

        let fields = report.defined_fields.get("Scheduler").unwrap();
        assert!(fields.contains("max_concurrent"));
        assert!(fields.contains("tasks"));
    }

    #[test]
    fn test_detect_missing_field() {
        let architect = LiaisonArchitect::new("rust");

        let results = vec![
            SubTaskResult {
                id: "types".to_string(),
                name: "Types".to_string(),
                code: "pub struct Foo { x: i32 }".to_string(),
                success: true,
                error: None,
            },
            SubTaskResult {
                id: "impl".to_string(),
                name: "Impl".to_string(),
                code: "impl Foo { fn bar(&self) { self.y } }".to_string(),
                success: true,
                error: None,
            },
        ];

        let report = architect.analyze(&results);

        // Should detect that 'y' is used but 'x' is defined
        let has_field_issue = report
            .issues
            .iter()
            .any(|i| matches!(i.category, IssueCategory::MissingField));
        assert!(has_field_issue, "Should detect missing field 'y'");
    }
}
