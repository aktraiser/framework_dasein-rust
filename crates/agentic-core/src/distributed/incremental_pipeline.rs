//! Incremental Validation Pipeline - Step-by-step code validation.
//!
//! Generic framework for validating code in stages across ALL languages:
//! 1. Types & Imports - Definitions only
//! 2. Stubs - Function signatures with stub bodies
//! 3. Logic - Full implementation
//!
//! # Supported Languages
//!
//! - **Rust**: `todo!()` stubs
//! - **TypeScript**: `throw new Error('TODO')` stubs
//! - **Python**: `raise NotImplementedError()` stubs
//! - **Go**: `panic("TODO")` stubs
//!
//! # Benefits
//!
//! - If Stage 1 fails → types/interfaces are wrong
//! - If Stage 2 fails → signatures are wrong
//! - If Stage 3 fails → implementation logic is broken
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::incremental_pipeline::{Stage, IncrementalPipeline};
//! use dasein_agentic_core::distributed::sandbox_validator::Language;
//!
//! // Works for any language!
//! let extractor = get_extractor(Language::TypeScript);
//! let types_only = extractor.extract_types(code);
//! let stubs = extractor.extract_stubs(code);
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

use super::sandbox_validator::{Language, SandboxValidator};
use dasein_agentic_sandbox::{Sandbox, SandboxError};

// ============================================================================
// TRAIT: LanguageExtractor - Interface commune pour tous les langages
// ============================================================================

/// Trait for language-specific code extraction.
///
/// Each language implements this to define how to extract types and stubs.
pub trait LanguageExtractor: Send + Sync {
    /// Language this extractor handles.
    fn language(&self) -> Language;

    /// Stub body template (e.g., "todo!()" for Rust, "throw new Error('TODO')" for TS).
    fn stub_body(&self) -> &'static str;

    /// Extract Stage 1: Types and imports only.
    fn extract_types(&self, code: &str) -> String;

    /// Extract Stage 2: Types + function/method stubs.
    fn extract_stubs(&self, code: &str) -> String;

    /// Extract Stage 3: Full code (default: no change).
    fn extract_logic(&self, code: &str) -> String {
        code.to_string()
    }

    /// Get code for a specific stage.
    fn for_stage(&self, stage: Stage, code: &str) -> String {
        match stage {
            Stage::Types => self.extract_types(code),
            Stage::Stubs => self.extract_stubs(code),
            Stage::Logic => self.extract_logic(code),
        }
    }

    /// Generate stage-specific fix prompt.
    fn generate_fix_prompt(
        &self,
        stage: Stage,
        stage_code: &str,
        errors: &[String],
        full_code: &str,
    ) -> String {
        generate_stage_fix_prompt(self.language(), stage, stage_code, errors, full_code)
    }
}

/// Get the appropriate extractor for a language.
pub fn get_extractor(lang: Language) -> Box<dyn LanguageExtractor> {
    match lang {
        Language::Rust => Box::new(RustExtractor),
        Language::TypeScript => Box::new(TypeScriptExtractor),
        Language::JavaScript => Box::new(TypeScriptExtractor), // JS uses same extractor
        Language::Python => Box::new(PythonExtractor),
        Language::Go => Box::new(GoExtractor),
        Language::Shell => Box::new(ShellExtractor), // Minimal support
    }
}

/// Validation stage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Stage {
    /// Stage 1: Imports and type definitions
    Types,
    /// Stage 2: Function signatures with stub bodies
    Stubs,
    /// Stage 3: Full logic implementation
    Logic,
}

impl Stage {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Types => "types",
            Self::Stubs => "stubs",
            Self::Logic => "logic",
        }
    }

    pub fn next(&self) -> Option<Self> {
        match self {
            Self::Types => Some(Self::Stubs),
            Self::Stubs => Some(Self::Logic),
            Self::Logic => None,
        }
    }

    pub fn all() -> Vec<Self> {
        vec![Self::Types, Self::Stubs, Self::Logic]
    }
}

/// Result of validating a stage.
#[derive(Debug, Clone)]
pub struct StageValidation {
    pub stage: Stage,
    pub passed: bool,
    pub code: String,
    pub errors: Vec<String>,
    pub duration_ms: u64,
}

// ============================================================================
// RUST EXTRACTOR
// ============================================================================

/// Rust-specific code extractor.
pub struct RustExtractor;

impl LanguageExtractor for RustExtractor {
    fn language(&self) -> Language {
        Language::Rust
    }
    fn stub_body(&self) -> &'static str {
        "todo!()"
    }

    fn extract_types(&self, code: &str) -> String {
        RustExtractor::extract_types_impl(code)
    }

    fn extract_stubs(&self, code: &str) -> String {
        RustExtractor::extract_stubs_impl(code)
    }
}

/// Legacy alias for backward compatibility.
pub type CodeExtractor = RustExtractor;

impl RustExtractor {
    /// Extract Stage 1: Types and imports only (Rust).
    pub fn extract_types(code: &str) -> String {
        Self::extract_types_impl(code)
    }

    fn extract_types_impl(code: &str) -> String {
        // Extract complete type definitions with their bodies
        Self::extract_complete_types(code)
    }

    /// Extract complete type definitions (including multi-line).
    fn extract_complete_types(code: &str) -> String {
        let mut result = String::new();
        let lines: Vec<&str> = code.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            // Imports
            if trimmed.starts_with("use ") || trimmed.starts_with("pub use ") {
                result.push_str(line);
                result.push('\n');
                i += 1;
                continue;
            }

            // Module docs
            if trimmed.starts_with("//!") {
                result.push_str(line);
                result.push('\n');
                i += 1;
                continue;
            }

            // Derive attributes - collect with following type
            if trimmed.starts_with("#[derive") || trimmed.starts_with("#[repr") {
                let block = Self::collect_block(&lines, i);
                result.push_str(&block);
                result.push_str("\n\n");
                i += block.lines().count();
                continue;
            }

            // Type definitions
            if trimmed.starts_with("pub struct ")
                || trimmed.starts_with("struct ")
                || trimmed.starts_with("pub enum ")
                || trimmed.starts_with("enum ")
                || trimmed.starts_with("pub type ")
                || trimmed.starts_with("type ")
            {
                let block = Self::collect_block(&lines, i);
                result.push_str(&block);
                result.push_str("\n\n");
                i += block.lines().count();
                continue;
            }

            i += 1;
        }

        result
    }

    /// Collect a complete block starting at line index.
    fn collect_block(lines: &[&str], start: usize) -> String {
        let mut result = String::new();
        let mut brace_depth = 0;
        let mut started = false;

        for i in start..lines.len() {
            let line = lines[i];
            result.push_str(line);
            result.push('\n');

            brace_depth += line.matches('{').count() as i32;
            brace_depth -= line.matches('}').count() as i32;

            if line.contains('{') {
                started = true;
            }

            // End of block
            if started && brace_depth == 0 {
                break;
            }

            // Single-line definition (tuple struct, type alias)
            if line.trim().ends_with(';') && !started {
                break;
            }
        }

        result.trim_end().to_string()
    }

    /// Extract Stage 2: Types + function stubs (Rust).
    pub fn extract_stubs(code: &str) -> String {
        Self::extract_stubs_impl(code)
    }

    fn extract_stubs_impl(code: &str) -> String {
        let types = Self::extract_types_impl(code);
        let mut result = types;

        let mut in_impl = false;
        let mut brace_depth = 0;
        let mut current_impl = String::new();

        for line in code.lines() {
            let trimmed = line.trim();

            if trimmed.starts_with("impl ") && !in_impl {
                in_impl = true;
                current_impl.clear();
                current_impl.push_str(line);
                current_impl.push('\n');
                brace_depth = line.matches('{').count() as i32 - line.matches('}').count() as i32;
                continue;
            }

            if in_impl {
                brace_depth += line.matches('{').count() as i32;
                brace_depth -= line.matches('}').count() as i32;
                current_impl.push_str(line);
                current_impl.push('\n');

                if brace_depth == 0 {
                    // End of impl block - convert to stubs
                    let stubbed = Self::stub_impl_block(&current_impl);
                    result.push_str(&stubbed);
                    result.push_str("\n\n");
                    in_impl = false;
                }
            }
        }

        result
    }

    /// Convert an impl block to use stub function bodies.
    fn stub_impl_block(impl_block: &str) -> String {
        let mut result = String::new();
        let mut in_fn_body = false;
        let mut fn_brace_depth = 0;

        for line in impl_block.lines() {
            let trimmed = line.trim();

            // Detect function start
            if !in_fn_body
                && (trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("pub async fn ")
                    || trimmed.starts_with("fn ")
                    || trimmed.starts_with("async fn "))
            {
                if trimmed.contains('{') {
                    // Function body starts on same line
                    in_fn_body = true;
                    fn_brace_depth = 1;

                    // Extract just the signature part before {
                    if let Some(brace_pos) = line.find('{') {
                        let sig = &line[..brace_pos];
                        result.push_str(sig);
                        result.push_str("{ todo!() }\n");
                    }
                } else {
                    result.push_str(line);
                    result.push('\n');
                }
                continue;
            }

            if in_fn_body {
                fn_brace_depth += line.matches('{').count() as i32;
                fn_brace_depth -= line.matches('}').count() as i32;

                if fn_brace_depth == 0 {
                    in_fn_body = false;
                }
                // Skip function body lines
                continue;
            }

            // Pass through other lines (impl header, closing brace, etc.)
            result.push_str(line);
            result.push('\n');
        }

        result
    }

    /// Extract Stage 3: Full code (no changes).
    pub fn extract_logic(code: &str) -> String {
        code.to_string()
    }

    /// Get code for a specific stage.
    pub fn for_stage(stage: Stage, code: &str) -> String {
        match stage {
            Stage::Types => Self::extract_types(code),
            Stage::Stubs => Self::extract_stubs(code),
            Stage::Logic => Self::extract_logic(code),
        }
    }
}

// ============================================================================
// TYPESCRIPT EXTRACTOR
// ============================================================================

/// TypeScript/JavaScript code extractor.
pub struct TypeScriptExtractor;

impl LanguageExtractor for TypeScriptExtractor {
    fn language(&self) -> Language {
        Language::TypeScript
    }
    fn stub_body(&self) -> &'static str {
        "throw new Error('TODO');"
    }

    fn extract_types(&self, code: &str) -> String {
        TypeScriptExtractor::extract_types_impl(code)
    }

    fn extract_stubs(&self, code: &str) -> String {
        TypeScriptExtractor::extract_stubs_impl(code)
    }
}

impl TypeScriptExtractor {
    /// Extract Stage 1: Types and imports only (TypeScript).
    fn extract_types_impl(code: &str) -> String {
        let mut result = String::new();
        let lines: Vec<&str> = code.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            // Import statements (handle multi-line)
            if trimmed.starts_with("import ") {
                let block = Self::collect_until_semicolon(&lines, i);
                result.push_str(&block);
                result.push('\n');
                i += block.matches('\n').count().max(1);
                continue;
            }

            // Interface definitions
            if trimmed.starts_with("interface ") || trimmed.starts_with("export interface ") {
                let block = Self::collect_brace_block(&lines, i);
                result.push_str(&block);
                result.push_str("\n\n");
                i += block.matches('\n').count().max(1);
                continue;
            }

            // Type aliases
            if trimmed.starts_with("type ") || trimmed.starts_with("export type ") {
                let block = Self::collect_until_semicolon(&lines, i);
                result.push_str(&block);
                result.push_str("\n\n");
                i += block.matches('\n').count().max(1);
                continue;
            }

            // Enum definitions
            if trimmed.starts_with("enum ") || trimmed.starts_with("export enum ") {
                let block = Self::collect_brace_block(&lines, i);
                result.push_str(&block);
                result.push_str("\n\n");
                i += block.matches('\n').count().max(1);
                continue;
            }

            // Const type declarations (as const)
            if (trimmed.starts_with("const ") || trimmed.starts_with("export const "))
                && trimmed.contains(" as const")
            {
                let block = Self::collect_until_semicolon(&lines, i);
                result.push_str(&block);
                result.push_str("\n\n");
                i += block.matches('\n').count().max(1);
                continue;
            }

            i += 1;
        }

        result
    }

    /// Extract Stage 2: Types + function/class stubs (TypeScript).
    fn extract_stubs_impl(code: &str) -> String {
        let types = Self::extract_types_impl(code);
        let mut result = types;
        let lines: Vec<&str> = code.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            // Class definitions
            if trimmed.starts_with("class ")
                || trimmed.starts_with("export class ")
                || trimmed.starts_with("abstract class ")
                || trimmed.starts_with("export abstract class ")
            {
                let block = Self::collect_brace_block(&lines, i);
                let stubbed = Self::stub_class(&block);
                result.push_str(&stubbed);
                result.push_str("\n\n");
                i += block.matches('\n').count().max(1);
                continue;
            }

            // Standalone functions
            if trimmed.starts_with("function ")
                || trimmed.starts_with("export function ")
                || trimmed.starts_with("async function ")
                || trimmed.starts_with("export async function ")
            {
                let block = Self::collect_brace_block(&lines, i);
                let stubbed = Self::stub_function(&block);
                result.push_str(&stubbed);
                result.push_str("\n\n");
                i += block.matches('\n').count().max(1);
                continue;
            }

            // Arrow function exports
            if (trimmed.starts_with("export const ") || trimmed.starts_with("const "))
                && !trimmed.contains(" as const")
            {
                // Check if it's an arrow function
                let block = Self::collect_brace_block(&lines, i);
                if block.contains("=>") {
                    let stubbed = Self::stub_arrow_function(&block);
                    result.push_str(&stubbed);
                    result.push_str("\n\n");
                    i += block.matches('\n').count().max(1);
                    continue;
                }
            }

            i += 1;
        }

        result
    }

    fn collect_until_semicolon(lines: &[&str], start: usize) -> String {
        let mut result = String::new();
        for i in start..lines.len() {
            result.push_str(lines[i]);
            result.push('\n');
            if lines[i].trim().ends_with(';') {
                break;
            }
        }
        result.trim_end().to_string()
    }

    fn collect_brace_block(lines: &[&str], start: usize) -> String {
        let mut result = String::new();
        let mut brace_depth = 0;
        let mut started = false;

        for i in start..lines.len() {
            let line = lines[i];
            result.push_str(line);
            result.push('\n');

            brace_depth += line.matches('{').count() as i32;
            brace_depth -= line.matches('}').count() as i32;

            if line.contains('{') {
                started = true;
            }

            if started && brace_depth == 0 {
                break;
            }

            // Single-line without braces
            if line.trim().ends_with(';') && !started {
                break;
            }
        }
        result.trim_end().to_string()
    }

    fn stub_class(class_block: &str) -> String {
        let mut result = String::new();
        let mut in_method_body = false;
        let mut method_brace_depth = 0;

        for line in class_block.lines() {
            let trimmed = line.trim();

            // Detect method start
            let is_method = !in_method_body
                && trimmed.contains('(')
                && (trimmed.starts_with("async ")
                    || trimmed.starts_with("public ")
                    || trimmed.starts_with("private ")
                    || trimmed.starts_with("protected ")
                    || trimmed.starts_with("static ")
                    || trimmed.starts_with("constructor")
                    || trimmed.chars().next().map_or(false, char::is_alphabetic));

            if is_method && !trimmed.ends_with(',') && !trimmed.contains(':') {
                if trimmed.contains('{') {
                    in_method_body = true;
                    method_brace_depth = 1;
                    // Extract signature, add stub
                    if let Some(brace_pos) = line.find('{') {
                        result.push_str(&line[..brace_pos]);
                        result.push_str("{ throw new Error('TODO'); }\n");
                    }
                    continue;
                }
            }

            if in_method_body {
                method_brace_depth += line.matches('{').count() as i32;
                method_brace_depth -= line.matches('}').count() as i32;
                if method_brace_depth == 0 {
                    in_method_body = false;
                }
                continue;
            }

            result.push_str(line);
            result.push('\n');
        }
        result.trim_end().to_string()
    }

    fn stub_function(fn_block: &str) -> String {
        if let Some(brace_pos) = fn_block.find('{') {
            let signature = &fn_block[..brace_pos];
            format!("{}{{ throw new Error('TODO'); }}", signature)
        } else {
            fn_block.to_string()
        }
    }

    fn stub_arrow_function(arrow_block: &str) -> String {
        if let Some(arrow_pos) = arrow_block.find("=>") {
            let before_arrow = &arrow_block[..arrow_pos + 2];
            format!("{} {{ throw new Error('TODO'); }};", before_arrow)
        } else {
            arrow_block.to_string()
        }
    }
}

// ============================================================================
// PYTHON EXTRACTOR
// ============================================================================

/// Python code extractor.
pub struct PythonExtractor;

impl LanguageExtractor for PythonExtractor {
    fn language(&self) -> Language {
        Language::Python
    }
    fn stub_body(&self) -> &'static str {
        "raise NotImplementedError()"
    }

    fn extract_types(&self, code: &str) -> String {
        PythonExtractor::extract_types_impl(code)
    }

    fn extract_stubs(&self, code: &str) -> String {
        PythonExtractor::extract_stubs_impl(code)
    }
}

impl PythonExtractor {
    /// Extract Stage 1: Imports and type definitions (Python).
    fn extract_types_impl(code: &str) -> String {
        let mut result = String::new();

        for line in code.lines() {
            let trimmed = line.trim();

            // Import statements
            if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
                result.push_str(line);
                result.push('\n');
                continue;
            }

            // Type aliases (Python 3.9+: TypeAlias, 3.12+: type)
            if trimmed.starts_with("type ") || trimmed.contains(": TypeAlias") {
                result.push_str(line);
                result.push('\n');
                continue;
            }

            // TypedDict, Protocol, dataclass definitions (just the decorator/class line)
            if trimmed.starts_with("@dataclass") || trimmed.starts_with("@dataclasses.dataclass") {
                result.push_str(line);
                result.push('\n');
                continue;
            }

            // Collect class definitions that are type-like (TypedDict, Protocol, Enum)
            if trimmed.starts_with("class ") {
                let is_type_class = trimmed.contains("TypedDict")
                    || trimmed.contains("Protocol")
                    || trimmed.contains("Enum")
                    || trimmed.contains("NamedTuple");
                if is_type_class {
                    result.push_str(line);
                    result.push('\n');
                    // TODO: collect full class body for these type classes
                }
            }
        }

        result
    }

    /// Extract Stage 2: Types + function/class stubs (Python).
    fn extract_stubs_impl(code: &str) -> String {
        let types = Self::extract_types_impl(code);
        let mut result = types;
        let lines: Vec<&str> = code.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            // Function definitions
            if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
                // Get indentation
                let indent = line.len() - line.trim_start().len();
                let indent_str: String = " ".repeat(indent);

                // Collect signature (may span multiple lines)
                let mut sig = line.to_string();
                let mut j = i + 1;
                while !sig.contains(':') && j < lines.len() {
                    sig.push_str(lines[j]);
                    j += 1;
                }

                // Add signature with stub body
                result.push_str(&sig);
                result.push('\n');
                result.push_str(&format!(
                    "{}    raise NotImplementedError()\n\n",
                    indent_str
                ));

                // Skip original function body
                let body_indent = indent + 4;
                while j < lines.len() {
                    let next_line = lines[j];
                    let next_indent = next_line.len() - next_line.trim_start().len();
                    if !next_line.trim().is_empty() && next_indent < body_indent {
                        break;
                    }
                    j += 1;
                }
                i = j;
                continue;
            }

            // Class definitions (non-type classes)
            if trimmed.starts_with("class ")
                && !trimmed.contains("TypedDict")
                && !trimmed.contains("Protocol")
                && !trimmed.contains("Enum")
            {
                result.push_str(line);
                result.push('\n');
                // Continue to collect methods inside
            }

            i += 1;
        }

        result
    }
}

// ============================================================================
// GO EXTRACTOR
// ============================================================================

/// Go code extractor.
pub struct GoExtractor;

impl LanguageExtractor for GoExtractor {
    fn language(&self) -> Language {
        Language::Go
    }
    fn stub_body(&self) -> &'static str {
        "panic(\"TODO\")"
    }

    fn extract_types(&self, code: &str) -> String {
        GoExtractor::extract_types_impl(code)
    }

    fn extract_stubs(&self, code: &str) -> String {
        GoExtractor::extract_stubs_impl(code)
    }
}

impl GoExtractor {
    /// Extract Stage 1: Package, imports, and type definitions (Go).
    fn extract_types_impl(code: &str) -> String {
        let mut result = String::new();
        let lines: Vec<&str> = code.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            // Package declaration
            if trimmed.starts_with("package ") {
                result.push_str(line);
                result.push_str("\n\n");
                i += 1;
                continue;
            }

            // Import statements
            if trimmed.starts_with("import ") {
                if trimmed.contains('(') {
                    // Multi-line import
                    let block = RustExtractor::collect_block(&lines, i);
                    result.push_str(&block);
                    result.push_str("\n\n");
                    i += block.matches('\n').count().max(1);
                } else {
                    result.push_str(line);
                    result.push('\n');
                    i += 1;
                }
                continue;
            }

            // Type definitions
            if trimmed.starts_with("type ") {
                let block = RustExtractor::collect_block(&lines, i);
                result.push_str(&block);
                result.push_str("\n\n");
                i += block.matches('\n').count().max(1);
                continue;
            }

            // Const declarations
            if trimmed.starts_with("const ") {
                if trimmed.contains('(') {
                    let block = RustExtractor::collect_block(&lines, i);
                    result.push_str(&block);
                    result.push_str("\n\n");
                    i += block.matches('\n').count().max(1);
                } else {
                    result.push_str(line);
                    result.push('\n');
                    i += 1;
                }
                continue;
            }

            // Var declarations
            if trimmed.starts_with("var ") {
                if trimmed.contains('(') {
                    let block = RustExtractor::collect_block(&lines, i);
                    result.push_str(&block);
                    result.push_str("\n\n");
                    i += block.matches('\n').count().max(1);
                } else {
                    result.push_str(line);
                    result.push('\n');
                    i += 1;
                }
                continue;
            }

            i += 1;
        }

        result
    }

    /// Extract Stage 2: Types + function stubs (Go).
    fn extract_stubs_impl(code: &str) -> String {
        let types = Self::extract_types_impl(code);
        let mut result = types;
        let lines: Vec<&str> = code.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            let line = lines[i];
            let trimmed = line.trim();

            // Function definitions
            if trimmed.starts_with("func ") {
                // Find the function signature
                let block = RustExtractor::collect_block(&lines, i);

                // Extract signature (everything before first {)
                if let Some(brace_pos) = block.find('{') {
                    let signature = &block[..brace_pos];
                    result.push_str(signature);
                    result.push_str("{\n\tpanic(\"TODO\")\n}\n\n");
                } else {
                    result.push_str(&block);
                    result.push_str("\n\n");
                }

                i += block.matches('\n').count().max(1);
                continue;
            }

            i += 1;
        }

        result
    }
}

// ============================================================================
// SHELL EXTRACTOR (Minimal)
// ============================================================================

/// Shell script extractor (minimal support).
pub struct ShellExtractor;

impl LanguageExtractor for ShellExtractor {
    fn language(&self) -> Language {
        Language::Shell
    }
    fn stub_body(&self) -> &'static str {
        "echo 'TODO' && exit 1"
    }

    fn extract_types(&self, code: &str) -> String {
        // Shell has no types - just return shebang and comments
        let mut result = String::new();
        for line in code.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("#!") || trimmed.starts_with("# ") || trimmed.is_empty() {
                result.push_str(line);
                result.push('\n');
            } else {
                break; // Stop at first non-comment line
            }
        }
        result
    }

    fn extract_stubs(&self, code: &str) -> String {
        // For shell, just return function signatures
        let mut result = self.extract_types(code);

        for line in code.lines() {
            let trimmed = line.trim();
            // Shell functions: function_name() { or function function_name {
            if (trimmed.contains("()") && trimmed.ends_with('{'))
                || trimmed.starts_with("function ")
            {
                if let Some(brace_pos) = line.find('{') {
                    result.push_str(&line[..brace_pos]);
                    result.push_str("{ echo 'TODO' && exit 1; }\n");
                }
            }
        }
        result
    }
}

/// Result of incremental validation.
#[derive(Debug, Clone)]
pub struct IncrementalResult {
    /// Which stage failed (if any)
    pub failed_stage: Option<Stage>,
    /// Last stage that passed
    pub last_passed_stage: Option<Stage>,
    /// Validated code at each passing stage
    pub validated_code: HashMap<Stage, String>,
    /// All stage results
    pub stages: Vec<StageValidation>,
    /// Overall success
    pub passed: bool,
    /// Total duration
    pub total_duration_ms: u64,
}

impl IncrementalResult {
    /// Get a summary string.
    pub fn summary(&self) -> String {
        let stages_str: Vec<String> = self
            .stages
            .iter()
            .map(|s| format!("{}: {}", s.stage.name(), if s.passed { "✓" } else { "✗" }))
            .collect();

        format!(
            "[{}] {}",
            stages_str.join(" → "),
            if self.passed {
                "ALL PASSED".to_string()
            } else {
                format!(
                    "FAILED at {}",
                    self.failed_stage.map(|s| s.name()).unwrap_or("?")
                )
            }
        )
    }

    /// Get errors from failed stage.
    pub fn errors(&self) -> Vec<String> {
        self.stages
            .iter()
            .find(|s| !s.passed)
            .map(|s| s.errors.clone())
            .unwrap_or_default()
    }

    /// Get the code that needs fixing (from failed stage).
    pub fn code_to_fix(&self) -> Option<&str> {
        self.stages
            .iter()
            .find(|s| !s.passed)
            .map(|s| s.code.as_str())
    }
}

/// Incremental validation pipeline for Rust code.
///
/// Uses SandboxValidator to validate code in stages.
/// Generic incremental validation pipeline for ANY language.
pub struct IncrementalPipeline<S: Sandbox> {
    validator: SandboxValidator<S>,
    language: Language,
}

impl<S: Sandbox> IncrementalPipeline<S> {
    /// Create a new incremental pipeline for a specific language.
    pub fn new(validator: SandboxValidator<S>, language: Language) -> Self {
        Self {
            validator,
            language,
        }
    }

    /// Create a Rust-specific pipeline (backward compatibility).
    pub fn new_rust(validator: SandboxValidator<S>) -> Self {
        Self::new(validator, Language::Rust)
    }

    /// Get the language this pipeline validates.
    pub fn language(&self) -> Language {
        self.language
    }

    /// Validate code incrementally through all stages.
    ///
    /// Uses the appropriate extractor for the configured language.
    /// Stops at the first failing stage and returns detailed info about what failed.
    pub async fn validate(&self, code: &str) -> Result<IncrementalResult, SandboxError> {
        let extractor = get_extractor(self.language);
        let start = std::time::Instant::now();
        let mut stages = Vec::new();
        let mut validated_code = HashMap::new();
        let mut last_passed = None;
        let mut failed_stage = None;

        for stage in Stage::all() {
            let stage_code = extractor.for_stage(stage, code);
            let stage_start = std::time::Instant::now();

            info!(
                "Validating {} stage: {} ({} chars)",
                self.language.extension(),
                stage.name(),
                stage_code.len()
            );
            debug!(
                "Stage code preview:\n{}",
                &stage_code[..stage_code.len().min(500)]
            );

            // Use the appropriate validator for the language
            let result = self
                .validator
                .validate_code(&stage_code, self.language)
                .await?;

            let errors: Vec<String> = if !result.compiles {
                result.compiler_errors.clone()
            } else {
                result.test_errors.clone()
            };

            let stage_result = StageValidation {
                stage,
                passed: result.passed,
                code: stage_code.clone(),
                errors: errors.clone(),
                duration_ms: stage_start.elapsed().as_millis() as u64,
            };

            if result.passed {
                validated_code.insert(stage, stage_code);
                last_passed = Some(stage);
                info!(
                    "✓ Stage {} passed in {}ms",
                    stage.name(),
                    stage_result.duration_ms
                );
            } else {
                failed_stage = Some(stage);
                warn!(
                    "✗ Stage {} failed with {} errors",
                    stage.name(),
                    errors.len()
                );
                stages.push(stage_result);
                break; // Stop at first failure
            }

            stages.push(stage_result);
        }

        Ok(IncrementalResult {
            failed_stage,
            last_passed_stage: last_passed,
            validated_code,
            stages,
            passed: failed_stage.is_none(),
            total_duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Validate only a specific stage.
    /// Validate only a specific stage.
    pub async fn validate_stage(
        &self,
        stage: Stage,
        code: &str,
    ) -> Result<StageValidation, SandboxError> {
        let extractor = get_extractor(self.language);
        let stage_code = extractor.for_stage(stage, code);
        let start = std::time::Instant::now();

        let result = self
            .validator
            .validate_code(&stage_code, self.language)
            .await?;

        let errors = if !result.compiles {
            result.compiler_errors
        } else {
            result.test_errors
        };

        Ok(StageValidation {
            stage,
            passed: result.passed,
            code: stage_code,
            errors,
            duration_ms: start.elapsed().as_millis() as u64,
        })
    }

    /// Get the underlying validator.
    pub fn validator(&self) -> &SandboxValidator<S> {
        &self.validator
    }
}

/// Validate code incrementally without creating a full pipeline.
///
/// This is a convenience function for quick validation.
/// Validate code incrementally without creating a full pipeline.
///
/// This is a convenience function for quick validation of any language.
pub async fn validate_incrementally<S: Sandbox>(
    validator: &SandboxValidator<S>,
    code: &str,
    language: Language,
) -> Result<IncrementalResult, SandboxError> {
    let extractor = get_extractor(language);
    let start = std::time::Instant::now();
    let mut stages = Vec::new();
    let mut validated_code = HashMap::new();
    let mut last_passed = None;
    let mut failed_stage = None;

    for stage in Stage::all() {
        let stage_code = extractor.for_stage(stage, code);
        let stage_start = std::time::Instant::now();

        info!(
            "Validating {} stage: {} ({} chars)",
            language.extension(),
            stage.name(),
            stage_code.len()
        );

        let result = validator.validate_code(&stage_code, language).await?;

        let errors: Vec<String> = if !result.compiles {
            result.compiler_errors.clone()
        } else {
            result.test_errors.clone()
        };

        let stage_result = StageValidation {
            stage,
            passed: result.passed,
            code: stage_code.clone(),
            errors: errors.clone(),
            duration_ms: stage_start.elapsed().as_millis() as u64,
        };

        if result.passed {
            validated_code.insert(stage, stage_code);
            last_passed = Some(stage);
        } else {
            failed_stage = Some(stage);
            stages.push(stage_result);
            break;
        }

        stages.push(stage_result);
    }

    Ok(IncrementalResult {
        failed_stage,
        last_passed_stage: last_passed,
        validated_code,
        stages,
        passed: failed_stage.is_none(),
        total_duration_ms: start.elapsed().as_millis() as u64,
    })
}

/// Generate a targeted fix prompt for a failed stage.
/// Generate a targeted fix prompt for a failed stage (language-aware).
pub fn generate_stage_fix_prompt(
    language: Language,
    stage: Stage,
    stage_code: &str,
    errors: &[String],
    full_code: &str,
) -> String {
    let mut prompt = String::new();
    let lang_name = match language {
        Language::Rust => "RUST",
        Language::TypeScript | Language::JavaScript => "TYPESCRIPT",
        Language::Python => "PYTHON",
        Language::Go => "GO",
        Language::Shell => "SHELL",
    };

    prompt.push_str(&format!(
        "=== {} FAILED AT STAGE: {} ===\n\n",
        lang_name,
        stage.name().to_uppercase()
    ));

    match stage {
        Stage::Types => {
            prompt.push_str("The TYPE DEFINITIONS have errors.\n");
            match language {
                Language::Rust => {
                    prompt.push_str("Fix ONLY the struct/enum/type definitions.\n\n");
                }
                Language::TypeScript | Language::JavaScript => {
                    prompt.push_str("Fix ONLY the interface/type/enum definitions.\n");
                    prompt.push_str("RULES:\n");
                    prompt.push_str("- Interfaces use { } not = { }\n");
                    prompt.push_str("- Type aliases use = syntax\n\n");
                }
                Language::Python => {
                    prompt.push_str("Fix ONLY the TypedDict/Protocol/dataclass definitions.\n\n");
                }
                Language::Go => {
                    prompt.push_str("Fix ONLY the struct/interface type definitions.\n\n");
                }
                _ => {
                    prompt.push_str("Fix ONLY the type definitions.\n\n");
                }
            }
        }
        Stage::Stubs => {
            prompt.push_str("The FUNCTION SIGNATURES have errors.\n");
            prompt.push_str("Fix ONLY the function signatures (parameters, return types).\n");
            prompt.push_str("Do NOT change the type definitions - they are correct.\n\n");
        }
        Stage::Logic => {
            prompt.push_str("The LOGIC IMPLEMENTATION has errors.\n");
            prompt.push_str("Fix ONLY the function bodies.\n");
            prompt.push_str("Do NOT change type definitions or function signatures.\n\n");

            // Language-specific critical rules
            match language {
                Language::TypeScript | Language::JavaScript => {
                    prompt.push_str("CRITICAL SYNTAX RULES (READ CAREFULLY!):\n\n");
                    prompt.push_str("STRING INTERPOLATION REQUIRES BACKTICKS:\n");
                    prompt.push_str(
                        "  WRONG:   throw new Error(Invalid: ${var})     // NO QUOTES = ERROR!\n",
                    );
                    prompt.push_str(
                        "  WRONG:   throw new Error(\"Invalid: ${var}\")   // QUOTES DON'T WORK!\n",
                    );
                    prompt.push_str("  CORRECT: throw new Error(`Invalid: ${var}`)   // BACKTICKS REQUIRED!\n\n");
                    prompt.push_str("EXAMPLES OF CORRECT USAGE:\n");
                    prompt.push_str("  throw new Error(`State ${state} is invalid`);\n");
                    prompt.push_str("  console.log(`Value: ${value}`);\n");
                    prompt.push_str("  const msg = `Hello ${name}`;\n\n");
                }
                Language::Python => {
                    prompt.push_str("CRITICAL RULES:\n");
                    prompt.push_str("- Use f-strings for interpolation: f\"text {var}\"\n");
                    prompt.push_str("- NEVER use: \"text {var}\" without f prefix\n\n");
                }
                _ => {}
            }
        }
    }

    prompt.push_str("=== ERRORS ===\n");
    for err in errors.iter().take(5) {
        prompt.push_str(&format!("{}\n", err.lines().next().unwrap_or(err)));
    }

    prompt.push_str("\n=== CODE TO FIX ===\n");
    prompt.push_str(stage_code);

    if stage != Stage::Types {
        prompt.push_str("\n\n=== FULL CONTEXT (for reference) ===\n");
        prompt.push_str(&full_code[..full_code.len().min(2000)]);
        if full_code.len() > 2000 {
            prompt.push_str("\n... (truncated)");
        }
    }

    prompt.push_str("\n\n=== INSTRUCTIONS ===\n");
    prompt.push_str("Return the COMPLETE fixed code (all stages combined).\n");
    prompt.push_str("NO markdown. NO explanations.\n");

    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_types() {
        let code = r#"
use std::collections::HashMap;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct TaskId(pub u64);

pub enum TaskError {
    Timeout,
    Failed,
}

impl TaskId {
    pub fn new(id: u64) -> Self {
        Self(id)
    }

    pub fn value(&self) -> u64 {
        self.0
    }
}

pub struct Scheduler {
    tasks: HashMap<TaskId, String>,
}
"#;

        let types = CodeExtractor::extract_types(code);

        assert!(types.contains("use std::collections::HashMap"));
        assert!(types.contains("pub struct TaskId"));
        assert!(types.contains("pub enum TaskError"));
        assert!(types.contains("pub struct Scheduler"));
        // Should NOT contain impl block
        assert!(!types.contains("pub fn new"));
        assert!(!types.contains("pub fn value"));
    }

    #[test]
    fn test_extract_stubs() {
        let code = r#"
use std::time::Duration;

pub struct Scheduler {
    max: usize,
}

impl Scheduler {
    pub fn new(max: usize) -> Self {
        Self { max }
    }

    pub async fn run(&self) -> Vec<u64> {
        let mut results = Vec::new();
        for i in 0..self.max {
            results.push(i as u64);
        }
        results
    }
}
"#;

        let stubs = CodeExtractor::extract_stubs(code);

        assert!(stubs.contains("pub struct Scheduler"));
        assert!(stubs.contains("pub fn new"));
        assert!(stubs.contains("pub async fn run"));
        assert!(stubs.contains("todo!()"));
        // Should NOT contain actual implementation
        assert!(!stubs.contains("for i in 0..self.max"));
    }

    #[test]
    fn test_stage_progression() {
        assert_eq!(Stage::Types.next(), Some(Stage::Stubs));
        assert_eq!(Stage::Stubs.next(), Some(Stage::Logic));
        assert_eq!(Stage::Logic.next(), None);
    }
}
