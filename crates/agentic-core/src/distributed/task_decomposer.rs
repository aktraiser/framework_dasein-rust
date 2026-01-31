//! Task Decomposer - Breaks complex tasks into sub-tasks for multiple executors.
//!
//! For complex code generation, a single LLM call often fails.
//! This module decomposes tasks into smaller, manageable sub-tasks.

use serde::{Deserialize, Serialize};

/// A sub-task to be executed by an Executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    /// Unique ID for this sub-task
    pub id: String,
    /// Name of the sub-task
    pub name: String,
    /// The prompt for this sub-task
    pub prompt: String,
    /// IDs of sub-tasks that must complete before this one
    pub depends_on: Vec<String>,
    /// Order in the final assembled code (lower = earlier)
    pub order: u32,
}

/// Result from a sub-task execution.
#[derive(Debug, Clone)]
pub struct SubTaskResult {
    pub id: String,
    pub name: String,
    pub code: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Decomposes complex tasks into sub-tasks.
pub struct TaskDecomposer {
    /// Language for code generation
    language: String,
}

impl TaskDecomposer {
    pub fn new(language: impl Into<String>) -> Self {
        Self {
            language: language.into(),
        }
    }

    /// Decompose a complex task into sub-tasks.
    pub fn decompose(&self, task: &str) -> Vec<SubTask> {
        match self.language.as_str() {
            "rust" => self.decompose_rust(task),
            "typescript" | "ts" => self.decompose_typescript(task),
            _ => vec![SubTask {
                id: "full".to_string(),
                name: "Full Task".to_string(),
                prompt: task.to_string(),
                depends_on: vec![],
                order: 0,
            }],
        }
    }

    /// Decompose a TypeScript task into: types, class skeleton, methods, tests.
    fn decompose_typescript(&self, task: &str) -> Vec<SubTask> {
        let base_context = format!(
            "You are generating PART of a TypeScript module. \
            Return ONLY valid TypeScript code, no markdown, no explanations.\n\
            CRITICAL: For string interpolation, use BACKTICKS: `text ${{var}}` NOT quotes.\n\n\
            FULL TASK CONTEXT:\n{}\n\n",
            task
        );

        vec![
            SubTask {
                id: "types".to_string(),
                name: "Type Definitions".to_string(),
                prompt: format!(
                    "{}YOUR PART: Write ONLY the type definitions:\n\
                    - All `interface` declarations\n\
                    - All `type` aliases\n\
                    - All `enum` definitions\n\
                    - Export all types\n\n\
                    Do NOT write classes. Do NOT write functions. Do NOT write tests.\n\
                    Return ONLY type definitions.",
                    base_context
                ),
                depends_on: vec![],
                order: 0,
            },
            SubTask {
                id: "class_skeleton".to_string(),
                name: "Class Skeleton".to_string(),
                prompt: format!(
                    "{}YOUR PART: Write ONLY the class declaration with:\n\
                    - All private/public properties\n\
                    - Constructor signature and basic initialization\n\
                    - All method signatures with stub bodies: throw new Error('TODO');\n\n\
                    Assume types from previous step are already defined.\n\
                    Do NOT implement method logic yet. Do NOT write tests.\n\
                    Return ONLY the class skeleton.",
                    base_context
                ),
                depends_on: vec!["types".to_string()],
                order: 1,
            },
            SubTask {
                id: "implementation".to_string(),
                name: "Method Implementation".to_string(),
                prompt: format!(
                    "{}YOUR PART: Implement ALL methods with full logic.\n\
                    Assume types and class skeleton are already defined.\n\
                    Replace all TODO stubs with real implementations.\n\n\
                    CRITICAL SYNTAX RULES:\n\
                    - Template literals MUST use backticks: `text ${{var}}`\n\
                    - WRONG: throw new Error(\"text ${{var}}\")\n\
                    - WRONG: throw new Error(text ${{var}})\n\
                    - CORRECT: throw new Error(`text ${{var}}`)\n\n\
                    Do NOT redefine types. Do NOT redefine class structure.\n\
                    Return ONLY the implemented methods (full class with implementations).",
                    base_context
                ),
                depends_on: vec!["class_skeleton".to_string()],
                order: 2,
            },
            SubTask {
                id: "tests".to_string(),
                name: "Jest Tests".to_string(),
                prompt: format!(
                    "{}YOUR PART: Write Jest tests using describe/it blocks.\n\
                    Test all major functionality mentioned in the task.\n\
                    Assume the implementation is complete and exported.\n\n\
                    CRITICAL: Use backticks for template literals: `text ${{var}}`\n\n\
                    Return ONLY the test code (describe blocks with it statements).",
                    base_context
                ),
                depends_on: vec!["implementation".to_string()],
                order: 3,
            },
        ]
    }

    /// Decompose a Rust task into: imports, types, impl, tests.
    fn decompose_rust(&self, task: &str) -> Vec<SubTask> {
        let base_context = format!(
            "You are generating PART of a Rust module. \
            Use ONLY stable Rust features. \
            Return ONLY the code for your part, no markdown.\n\n\
            FULL TASK CONTEXT:\n{}\n\n",
            task
        );

        vec![
            SubTask {
                id: "imports".to_string(),
                name: "Imports & Dependencies".to_string(),
                prompt: format!(
                    "{}YOUR PART: Write ALL the `use` statements that might be needed. \
                    Include these common imports:\n\
                    - use std::collections::{{HashMap, HashSet, VecDeque}};\n\
                    - use std::sync::Arc;\n\
                    - use std::time::{{Duration, Instant}};\n\
                    - use std::pin::Pin;\n\
                    - use std::future::Future;\n\
                    - use tokio::sync::{{Mutex, RwLock, Semaphore}};\n\
                    - use tokio::time::{{sleep, timeout}};\n\
                    - use futures::future::BoxFuture;\n\
                    Return ONLY the use statements, nothing else.",
                    base_context
                ),
                depends_on: vec![],
                order: 0,
            },
            SubTask {
                id: "types".to_string(),
                name: "Type Definitions".to_string(),
                prompt: format!(
                    "{}YOUR PART: Write ONLY the struct and enum definitions. \
                    Include all fields and derive macros. \
                    Do NOT write impl blocks. Do NOT write use statements. \
                    Return ONLY struct/enum definitions.",
                    base_context
                ),
                depends_on: vec!["imports".to_string()],
                order: 1,
            },
            SubTask {
                id: "impl".to_string(),
                name: "Implementation".to_string(),
                prompt: format!(
                    "{}YOUR PART: Write ONLY the impl blocks with all methods. \
                    Assume types are already defined. \
                    Do NOT write struct/enum definitions. Do NOT write use statements. \
                    Do NOT write tests. Return ONLY impl blocks.",
                    base_context
                ),
                depends_on: vec!["types".to_string()],
                order: 2,
            },
            SubTask {
                id: "tests".to_string(),
                name: "Tests".to_string(),
                prompt: format!(
                    "{}YOUR PART: Write ONLY the #[cfg(test)] module with tests. \
                    Use #[tokio::test] for async tests. \
                    Assume all types and impls are already defined. \
                    Do NOT write use statements outside the test module. \
                    Return ONLY the test module.",
                    base_context
                ),
                depends_on: vec!["impl".to_string()],
                order: 3,
            },
        ]
    }

    /// Create a more granular decomposition for very complex tasks.
    pub fn decompose_granular(&self, task: &str, sections: Vec<(&str, &str)>) -> Vec<SubTask> {
        let base_context = format!(
            "You are generating PART of a Rust module. \
            Use ONLY stable Rust features. \
            Return ONLY the code for your part, no markdown.\n\n\
            FULL TASK CONTEXT:\n{}\n\n",
            task
        );

        sections
            .into_iter()
            .enumerate()
            .map(|(i, (name, instruction))| SubTask {
                id: format!("part_{}", i),
                name: name.to_string(),
                prompt: format!("{}YOUR PART: {}", base_context, instruction),
                depends_on: if i > 0 {
                    vec![format!("part_{}", i - 1)]
                } else {
                    vec![]
                },
                order: i as u32,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decompose_rust() {
        let decomposer = TaskDecomposer::new("rust");
        let subtasks = decomposer.decompose("Build an async cache");

        assert_eq!(subtasks.len(), 4);
        assert_eq!(subtasks[0].id, "imports");
        assert_eq!(subtasks[1].id, "types");
        assert_eq!(subtasks[2].id, "impl");
        assert_eq!(subtasks[3].id, "tests");
    }

    #[test]
    fn test_dependencies() {
        let decomposer = TaskDecomposer::new("rust");
        let subtasks = decomposer.decompose("Build something");

        assert!(subtasks[0].depends_on.is_empty());
        assert_eq!(subtasks[1].depends_on, vec!["imports"]);
        assert_eq!(subtasks[2].depends_on, vec!["types"]);
        assert_eq!(subtasks[3].depends_on, vec!["impl"]);
    }
}
