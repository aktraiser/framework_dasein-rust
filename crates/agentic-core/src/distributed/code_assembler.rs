//! Code Assembler - Combines code from multiple executors into a single module.

use super::task_decomposer::{SubTask, SubTaskResult};

/// Assembles code fragments from multiple executors.
pub struct CodeAssembler;

impl CodeAssembler {
    pub fn new() -> Self {
        Self
    }

    /// Assemble results into a single code string.
    pub fn assemble(&self, results: &[SubTaskResult]) -> Result<String, AssemblyError> {
        // Check all succeeded
        let failures: Vec<_> = results.iter().filter(|r| !r.success).collect();
        if !failures.is_empty() {
            return Err(AssemblyError::SubTasksFailed(
                failures.iter().map(|r| r.name.clone()).collect(),
            ));
        }

        // Sort by order (we need the original subtasks for order, but we can infer from id)
        let mut sorted: Vec<_> = results.iter().collect();
        sorted.sort_by_key(|r| match r.id.as_str() {
            "imports" => 0,
            "types" => 1,
            "impl" => 2,
            "tests" => 3,
            id if id.starts_with("part_") => id
                .strip_prefix("part_")
                .and_then(|n| n.parse().ok())
                .unwrap_or(99),
            _ => 99,
        });

        // Combine code
        let mut assembled = String::new();

        for result in sorted {
            let clean_code = self.clean_fragment(&result.code);
            if !clean_code.is_empty() {
                assembled.push_str(&clean_code);
                assembled.push_str("\n\n");
            }
        }

        // Post-process: remove duplicate imports, fix common issues
        let final_code = self.post_process(&assembled);

        Ok(final_code)
    }

    /// Clean a code fragment (remove markdown, extra whitespace, invalid chars).
    fn clean_fragment(&self, code: &str) -> String {
        let mut clean = code.to_string();

        // Remove markdown code blocks - check language-specific markers first
        let markers = [
            "```rust",
            "```rs", // Rust
            "```python",
            "```py", // Python
            "```javascript",
            "```js", // JavaScript
            "```typescript",
            "```ts", // TypeScript
            "```go",
            "```golang", // Go
            "```shell",
            "```bash",
            "```sh", // Shell
            "```",   // Generic (must be last)
        ];

        for marker in markers {
            if clean.contains(marker) {
                clean = clean
                    .split(marker)
                    .nth(1)
                    .unwrap_or(&clean)
                    .split("```")
                    .next()
                    .unwrap_or(&clean)
                    .to_string();
                break;
            }
        }

        // If the first line is just a language identifier (leftover from partial markdown),
        // remove it
        let language_identifiers = [
            "rust",
            "python",
            "py",
            "javascript",
            "js",
            "typescript",
            "ts",
            "go",
            "bash",
            "sh",
            "shell",
        ];
        let lines: Vec<&str> = clean.lines().collect();
        if !lines.is_empty() {
            let first_line = lines[0].trim().to_lowercase();
            if language_identifiers.contains(&first_line.as_str()) {
                clean = lines[1..].join("\n");
            }
        }

        // Remove any remaining backticks (common LLM pollution)
        clean = clean.replace('`', "");

        // Remove smart quotes (LLM sometimes generates these)
        // Using Unicode escapes to avoid source file encoding issues
        clean = clean.replace('\u{201C}', "\"").replace('\u{201D}', "\""); // " and "
        clean = clean.replace('\u{2018}', "'").replace('\u{2019}', "'"); // ' and '

        clean.trim().to_string()
    }

    /// Clean code for validation - call this before sending to compiler.
    pub fn clean_for_validation(&self, code: &str) -> String {
        self.clean_fragment(code)
    }

    /// Fix missing imports by scanning code for common types.
    fn fix_missing_imports(&self, code: &str) -> String {
        let mut missing_imports = Vec::new();

        // Check for commonly missing types
        let checks = [
            ("Arc<", "use std::sync::Arc;"),
            ("Arc::", "use std::sync::Arc;"),
            ("HashMap<", "use std::collections::HashMap;"),
            ("HashMap::", "use std::collections::HashMap;"),
            ("HashSet<", "use std::collections::HashSet;"),
            ("VecDeque<", "use std::collections::VecDeque;"),
            ("Duration", "use std::time::Duration;"),
            ("Instant", "use std::time::Instant;"),
            ("Pin<", "use std::pin::Pin;"),
            ("BoxFuture", "use futures::future::BoxFuture;"),
            ("Mutex<", "use tokio::sync::Mutex;"),
            ("RwLock<", "use tokio::sync::RwLock;"),
            ("Semaphore", "use tokio::sync::Semaphore;"),
            ("sleep(", "use tokio::time::sleep;"),
            ("timeout(", "use tokio::time::timeout;"),
        ];

        for (pattern, import) in checks {
            if code.contains(pattern) && !code.contains(import) {
                // Check if any variant of this import exists
                let base = import
                    .split("::")
                    .last()
                    .unwrap_or("")
                    .trim_end_matches(';');
                if !code.contains(&format!("use "))
                    || !code
                        .lines()
                        .any(|l| l.contains(base) && l.trim().starts_with("use "))
                {
                    if !missing_imports.contains(&import.to_string()) {
                        missing_imports.push(import.to_string());
                    }
                }
            }
        }

        if missing_imports.is_empty() {
            return code.to_string();
        }

        // Insert missing imports at the top
        let mut result = missing_imports.join("\n");
        result.push_str("\n\n");
        result.push_str(code);
        result
    }

    /// Post-process assembled code.
    fn post_process(&self, code: &str) -> String {
        // First, add any missing common imports
        let code = self.fix_missing_imports(code);

        let lines: Vec<&str> = code.lines().collect();
        let mut result = Vec::new();
        let mut seen_imports = std::collections::HashSet::new();

        for line in lines {
            let trimmed = line.trim();

            // Deduplicate use statements
            if trimmed.starts_with("use ") {
                if seen_imports.contains(trimmed) {
                    continue;
                }
                seen_imports.insert(trimmed.to_string());
            }

            result.push(line);
        }

        // Remove excessive blank lines
        let mut final_lines = Vec::new();
        let mut blank_count = 0;

        for line in result {
            if line.trim().is_empty() {
                blank_count += 1;
                if blank_count <= 2 {
                    final_lines.push(line);
                }
            } else {
                blank_count = 0;
                final_lines.push(line);
            }
        }

        final_lines.join("\n")
    }

    /// Assemble with explicit ordering.
    pub fn assemble_ordered(
        &self,
        mut results: Vec<SubTaskResult>,
        tasks: &[SubTask],
    ) -> Result<String, AssemblyError> {
        // Sort results by task order
        results.sort_by_key(|r| {
            tasks
                .iter()
                .find(|t| t.id == r.id)
                .map(|t| t.order)
                .unwrap_or(999)
        });

        self.assemble(&results)
    }
}

impl Default for CodeAssembler {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors during assembly.
#[derive(Debug, Clone)]
pub enum AssemblyError {
    /// Some sub-tasks failed
    SubTasksFailed(Vec<String>),
    /// Code validation failed after assembly
    ValidationFailed(String),
}

impl std::fmt::Display for AssemblyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SubTasksFailed(names) => {
                write!(f, "Sub-tasks failed: {}", names.join(", "))
            }
            Self::ValidationFailed(msg) => {
                write!(f, "Validation failed: {}", msg)
            }
        }
    }
}

impl std::error::Error for AssemblyError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assemble_basic() {
        let assembler = CodeAssembler::new();

        let results = vec![
            SubTaskResult {
                id: "imports".to_string(),
                name: "Imports".to_string(),
                code: "use std::collections::HashMap;".to_string(),
                success: true,
                error: None,
            },
            SubTaskResult {
                id: "types".to_string(),
                name: "Types".to_string(),
                code: "pub struct Foo { x: i32 }".to_string(),
                success: true,
                error: None,
            },
        ];

        let assembled = assembler.assemble(&results).unwrap();
        assert!(assembled.contains("use std::collections::HashMap"));
        assert!(assembled.contains("pub struct Foo"));
    }

    #[test]
    fn test_deduplicate_imports() {
        let assembler = CodeAssembler::new();

        let results = vec![
            SubTaskResult {
                id: "imports".to_string(),
                name: "Imports".to_string(),
                code: "use std::sync::Arc;\nuse tokio::sync::Mutex;".to_string(),
                success: true,
                error: None,
            },
            SubTaskResult {
                id: "types".to_string(),
                name: "Types".to_string(),
                code: "use std::sync::Arc;\npub struct Foo {}".to_string(),
                success: true,
                error: None,
            },
        ];

        let assembled = assembler.assemble(&results).unwrap();
        let arc_count = assembled.matches("use std::sync::Arc").count();
        assert_eq!(arc_count, 1, "Should deduplicate imports");
    }

    #[test]
    fn test_clean_markdown() {
        let assembler = CodeAssembler::new();

        let results = vec![SubTaskResult {
            id: "imports".to_string(),
            name: "Imports".to_string(),
            code: "```rust\nuse std::io;\n```".to_string(),
            success: true,
            error: None,
        }];

        let assembled = assembler.assemble(&results).unwrap();
        assert!(!assembled.contains("```"));
        assert!(assembled.contains("use std::io"));
    }

    #[test]
    fn test_failure_handling() {
        let assembler = CodeAssembler::new();

        let results = vec![SubTaskResult {
            id: "imports".to_string(),
            name: "Imports".to_string(),
            code: "".to_string(),
            success: false,
            error: Some("Failed".to_string()),
        }];

        let result = assembler.assemble(&results);
        assert!(result.is_err());
    }
}
