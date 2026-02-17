//! Code Assembler Executor - Merge code fragments (MAF Pattern)
//!
//! Uses dynamic dispatch with `serde_json::Value` for MAF compatibility.
//!
//! # Input Schema
//!
//! ```json
//! {
//!   "parts": [
//!     {"name": "types", "code": "struct Foo {}", "language": "rust", "section": "Types"},
//!     {"name": "impl", "code": "impl Foo {}", "language": "rust"}
//!   ],
//!   "language": "rust"
//! }
//! ```
//!
//! # Output Schema
//!
//! ```json
//! {
//!   "code": "// === TYPES ===\nstruct Foo {}\n\n// === IMPL ===\nimpl Foo {}",
//!   "language": "rust",
//!   "part_count": 2,
//!   "size_bytes": 50
//! }
//! ```
//!
//! # Example
//!
//! ```rust,ignore
//! let assembler = CodeAssemblerExecutor::new("assembler")
//!     .with_header("// Generated code - do not edit");
//!
//! // In workflow: receives parts from fan-in edge
//! registry.register(assembler);
//! ```

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::distributed::graph::{
    Executor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind,
};
use crate::distributed::CodeAssembler;

// ============================================================================
// INPUT/OUTPUT TYPES
// ============================================================================

/// A code part to be assembled (type-safe helper).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodePart {
    /// Name/identifier of this part.
    pub name: String,
    /// The code content.
    pub code: String,
    /// Language of the code.
    pub language: String,
    /// Optional section header.
    #[serde(default)]
    pub section: Option<String>,
}

impl CodePart {
    /// Create a new code part.
    pub fn new(
        name: impl Into<String>,
        code: impl Into<String>,
        language: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            code: code.into(),
            language: language.into(),
            section: None,
        }
    }

    /// Add a section header.
    pub fn with_section(mut self, section: impl Into<String>) -> Self {
        self.section = Some(section.into());
        self
    }
}

/// Input to the assembler (type-safe helper).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssemblyInput {
    /// Parts to assemble.
    pub parts: Vec<CodePart>,
    /// Target language.
    pub language: String,
}

impl AssemblyInput {
    /// Create new assembly input.
    pub fn new(language: impl Into<String>) -> Self {
        Self {
            parts: vec![],
            language: language.into(),
        }
    }

    /// Add a code part.
    pub fn add_part(mut self, part: CodePart) -> Self {
        self.parts.push(part);
        self
    }

    /// Convert to JSON Value.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap()
    }
}

/// Output from the assembler.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssembledCode {
    /// The final assembled code.
    pub code: String,
    /// Language of the code.
    pub language: String,
    /// Number of parts assembled.
    pub part_count: usize,
    /// Total size in bytes.
    pub size_bytes: usize,
}

impl AssembledCode {
    /// Create new assembled code.
    pub fn new(code: impl Into<String>, language: impl Into<String>, part_count: usize) -> Self {
        let code = code.into();
        let size_bytes = code.len();
        Self {
            code,
            language: language.into(),
            part_count,
            size_bytes,
        }
    }

    /// Convert to JSON Value.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap()
    }
}

// ============================================================================
// CODE ASSEMBLER EXECUTOR
// ============================================================================

/// Graph Executor that merges code fragments.
///
/// Uses MAF-style dynamic dispatch with `serde_json::Value`.
pub struct CodeAssemblerExecutor {
    id: ExecutorId,
    /// The underlying assembler.
    assembler: CodeAssembler,
    /// Optional header to prepend.
    header: Option<String>,
    /// Section separator style.
    separator_style: SeparatorStyle,
}

/// Style for section separators.
#[derive(Debug, Clone, Copy)]
pub enum SeparatorStyle {
    /// No separators.
    None,
    /// Simple comment line.
    Simple,
    /// Box-style comment.
    Box,
}

impl CodeAssemblerExecutor {
    /// Create a new code assembler executor.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: ExecutorId::new(id),
            assembler: CodeAssembler::new(),
            header: None,
            separator_style: SeparatorStyle::Box,
        }
    }

    /// Set a header to prepend to the output.
    pub fn with_header(mut self, header: impl Into<String>) -> Self {
        self.header = Some(header.into());
        self
    }

    /// Set the separator style.
    pub fn with_separator_style(mut self, style: SeparatorStyle) -> Self {
        self.separator_style = style;
        self
    }

    /// Format a section header.
    fn format_section(&self, name: &str, language: &str) -> String {
        match self.separator_style {
            SeparatorStyle::None => String::new(),
            SeparatorStyle::Simple => {
                let comment = comment_prefix(language);
                format!("{} === {} ===\n\n", comment, name.to_uppercase())
            }
            SeparatorStyle::Box => {
                let comment = comment_prefix(language);
                let line = format!("{} {}", comment, "=".repeat(70));
                format!(
                    "{}\n{} {}\n{}\n\n",
                    line,
                    comment,
                    name.to_uppercase(),
                    line
                )
            }
        }
    }

    /// Assemble all parts into a single code string.
    fn assemble_parts(&self, input: &AssemblyInput) -> String {
        let mut output = String::new();

        // Add header if present
        if let Some(ref header) = self.header {
            output.push_str(header);
            output.push_str("\n\n");
        }

        // Collect unique imports (for languages that need it)
        let mut imports = vec![];
        for part in &input.parts {
            imports.extend(extract_imports(&part.code, &input.language));
        }
        imports.sort();
        imports.dedup();

        // Add imports first
        if !imports.is_empty() {
            for imp in &imports {
                output.push_str(imp);
                output.push('\n');
            }
            output.push('\n');
        }

        // Add each part
        for part in &input.parts {
            // Add section header
            let section_name = part.section.as_deref().unwrap_or(&part.name);
            output.push_str(&self.format_section(section_name, &input.language));

            // Add code (without imports, they're at the top)
            let code = remove_imports(&part.code, &input.language);
            let cleaned = self.assembler.clean_for_validation(&code);
            output.push_str(&cleaned);
            output.push_str("\n\n");
        }

        output.trim_end().to_string()
    }
}

#[async_trait]
impl Executor for CodeAssemblerExecutor {
    // MAF Pattern: All types are Value for dynamic dispatch
    type Input = Value;
    type Message = Value;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    fn name(&self) -> &str {
        "Code Assembler"
    }

    fn description(&self) -> Option<&str> {
        Some("Merges multiple code fragments into a single output")
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        // Deserialize input dynamically
        let typed_input: AssemblyInput = serde_json::from_value(input).map_err(|e| {
            ExecutorError::new(
                self.id.clone(),
                format!("Invalid input (expected AssemblyInput): {}", e),
            )
        })?;

        let part_count = typed_input.parts.len();
        let language = typed_input.language.clone();

        // Assemble the code
        let code = self.assemble_parts(&typed_input);

        // Log event
        ctx.add_event(
            "assembly",
            serde_json::json!({
                "part_count": part_count,
                "output_size": code.len(),
                "language": &language,
            }),
        )
        .await?;

        // Build output
        let assembled = AssembledCode::new(&code, &language, part_count);

        // Send as Value to next executors
        ctx.send_message(assembled.to_value()).await?;

        // Yield the final code as output
        ctx.yield_output(code).await?;

        Ok(())
    }
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Get the comment prefix for a language.
fn comment_prefix(language: &str) -> &'static str {
    match language.to_lowercase().as_str() {
        "rust" | "go" | "java" | "javascript" | "typescript" | "c" | "cpp" => "//",
        "python" | "ruby" | "bash" | "sh" => "#",
        "html" | "xml" => "<!--",
        _ => "//",
    }
}

/// Extract import statements from code.
fn extract_imports(code: &str, language: &str) -> Vec<String> {
    let mut imports = vec![];

    for line in code.lines() {
        let trimmed = line.trim();
        let is_import = match language.to_lowercase().as_str() {
            "rust" => trimmed.starts_with("use ") || trimmed.starts_with("extern crate"),
            "python" => trimmed.starts_with("import ") || trimmed.starts_with("from "),
            "javascript" | "typescript" => {
                trimmed.starts_with("import ")
                    || trimmed.starts_with("const ") && trimmed.contains("require(")
            }
            "go" => trimmed.starts_with("import "),
            "java" => trimmed.starts_with("import "),
            _ => false,
        };

        if is_import {
            imports.push(line.to_string());
        }
    }

    imports
}

/// Remove import statements from code (they'll be collected at top).
fn remove_imports(code: &str, language: &str) -> String {
    code.lines()
        .filter(|line| {
            let trimmed = line.trim();
            let is_import = match language.to_lowercase().as_str() {
                "rust" => trimmed.starts_with("use ") || trimmed.starts_with("extern crate"),
                "python" => trimmed.starts_with("import ") || trimmed.starts_with("from "),
                "javascript" | "typescript" => {
                    trimmed.starts_with("import ")
                        || (trimmed.starts_with("const ") && trimmed.contains("require("))
                }
                "go" => trimmed.starts_with("import "),
                "java" => trimmed.starts_with("import "),
                _ => false,
            };
            !is_import
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_start()
        .to_string()
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_part() {
        let part = CodePart::new("types", "struct Foo {}", "rust").with_section("Type Definitions");

        assert_eq!(part.name, "types");
        assert_eq!(part.section, Some("Type Definitions".into()));
    }

    #[test]
    fn test_assembly_input() {
        let input = AssemblyInput::new("rust")
            .add_part(CodePart::new("a", "struct A {}", "rust"))
            .add_part(CodePart::new("b", "struct B {}", "rust"));

        assert_eq!(input.parts.len(), 2);
        assert_eq!(input.language, "rust");
    }

    #[test]
    fn test_assembly_input_to_value() {
        let input = AssemblyInput::new("rust").add_part(CodePart::new("a", "struct A {}", "rust"));

        let value = input.to_value();
        assert_eq!(value["language"], "rust");
        assert_eq!(value["parts"][0]["name"], "a");
    }

    #[test]
    fn test_assembled_code_to_value() {
        let assembled = AssembledCode::new("struct Foo {}", "rust", 1);
        let value = assembled.to_value();

        assert_eq!(value["language"], "rust");
        assert_eq!(value["part_count"], 1);
    }

    #[test]
    fn test_extract_imports_rust() {
        let code = "use std::io;\nuse std::fs;\n\nfn main() {}";
        let imports = extract_imports(code, "rust");
        assert_eq!(imports.len(), 2);
    }

    #[test]
    fn test_comment_prefix() {
        assert_eq!(comment_prefix("rust"), "//");
        assert_eq!(comment_prefix("python"), "#");
    }

    #[test]
    fn test_deserialize_from_value() {
        let value = serde_json::json!({
            "parts": [
                {"name": "a", "code": "struct A {}", "language": "rust"}
            ],
            "language": "rust"
        });

        let input: AssemblyInput = serde_json::from_value(value).unwrap();
        assert_eq!(input.language, "rust");
        assert_eq!(input.parts.len(), 1);
    }
}
