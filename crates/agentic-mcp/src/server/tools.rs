//! MCP Tools for code validation and analysis.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

use agentic_sandbox::Sandbox;

/// MCP Server exposing validation/audit tools.
pub struct ValidatorMCPServer<S: Sandbox> {
    sandbox: Arc<S>,
    workspace: std::path::PathBuf,
}

/// Tool definition for MCP protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MCPToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Tool call result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MCPToolResult {
    pub content: Vec<MCPContent>,
    #[serde(default)]
    pub is_error: bool,
}

/// Content in tool result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum MCPContent {
    Text { text: String },
}

impl<S: Sandbox + Send + Sync + 'static> ValidatorMCPServer<S> {
    /// Create new MCP server with sandbox.
    pub fn new(sandbox: S, workspace: std::path::PathBuf) -> Self {
        Self {
            sandbox: Arc::new(sandbox),
            workspace,
        }
    }

    /// List available tools.
    pub fn list_tools(&self) -> Vec<MCPToolDef> {
        vec![
            MCPToolDef {
                name: "validate-code".to_string(),
                description: "Validate code by compiling and running tests in a sandbox. Returns compilation errors or test results.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "code": {
                            "type": "string",
                            "description": "The source code to validate"
                        },
                        "language": {
                            "type": "string",
                            "enum": ["rust", "python", "typescript"],
                            "description": "Programming language"
                        },
                        "run_tests": {
                            "type": "boolean",
                            "default": true,
                            "description": "Whether to run tests after compilation"
                        }
                    },
                    "required": ["code", "language"]
                }),
            },
            MCPToolDef {
                name: "analyze-code".to_string(),
                description: "Static analysis of code without execution. Checks for common issues, style, and potential bugs.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "code": {
                            "type": "string",
                            "description": "The source code to analyze"
                        },
                        "language": {
                            "type": "string",
                            "enum": ["rust", "python", "typescript"],
                            "description": "Programming language"
                        },
                        "checks": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Specific checks to run (clippy, pylint, eslint, etc.)"
                        }
                    },
                    "required": ["code", "language"]
                }),
            },
            MCPToolDef {
                name: "audit-security".to_string(),
                description: "Security audit of code. Checks for vulnerabilities, unsafe patterns, and security best practices.".to_string(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "code": {
                            "type": "string",
                            "description": "The source code to audit"
                        },
                        "language": {
                            "type": "string",
                            "enum": ["rust", "python", "typescript"],
                            "description": "Programming language"
                        },
                        "severity_threshold": {
                            "type": "string",
                            "enum": ["low", "medium", "high", "critical"],
                            "default": "medium",
                            "description": "Minimum severity to report"
                        }
                    },
                    "required": ["code", "language"]
                }),
            },
        ]
    }

    /// Call a tool by name.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> MCPToolResult {
        match name {
            "validate-code" => self.validate_code(arguments).await,
            "analyze-code" => self.analyze_code(arguments).await,
            "audit-security" => self.audit_security(arguments).await,
            _ => MCPToolResult {
                content: vec![MCPContent::Text {
                    text: format!("Unknown tool: {}", name),
                }],
                is_error: true,
            },
        }
    }

    /// Validate code tool implementation.
    async fn validate_code(&self, args: Value) -> MCPToolResult {
        let code = args["code"].as_str().unwrap_or("");
        let language = args["language"].as_str().unwrap_or("rust");
        let run_tests = args["run_tests"].as_bool().unwrap_or(true);

        // Create temp project
        let project_id = uuid::Uuid::new_v4();
        let project_dir = self.workspace.join(format!("validate-{}", project_id));

        let result = match language {
            "rust" => self.validate_rust(code, &project_dir, run_tests).await,
            "python" => self.validate_python(code, &project_dir, run_tests).await,
            _ => Err(format!("Unsupported language: {}", language)),
        };

        // Cleanup
        let _ = std::fs::remove_dir_all(&project_dir);

        match result {
            Ok(output) => MCPToolResult {
                content: vec![MCPContent::Text { text: output }],
                is_error: false,
            },
            Err(e) => MCPToolResult {
                content: vec![MCPContent::Text { text: e }],
                is_error: true,
            },
        }
    }

    /// Validate Rust code.
    async fn validate_rust(
        &self,
        code: &str,
        project_dir: &std::path::Path,
        run_tests: bool,
    ) -> Result<String, String> {
        std::fs::create_dir_all(project_dir).map_err(|e| e.to_string())?;

        // Create Cargo.toml
        let cargo_toml = r#"[package]
name = "validate"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
"#;
        std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).map_err(|e| e.to_string())?;

        std::fs::create_dir_all(project_dir.join("src")).map_err(|e| e.to_string())?;
        std::fs::write(project_dir.join("src/lib.rs"), code).map_err(|e| e.to_string())?;

        // Compile
        let compile_cmd = format!("cd {} && cargo check 2>&1", project_dir.display());
        let compile_result = self
            .sandbox
            .execute(&compile_cmd)
            .await
            .map_err(|e| e.to_string())?;

        if !compile_result.is_success() {
            return Ok(format!(
                "COMPILATION FAILED:\n{}{}",
                compile_result.stdout, compile_result.stderr
            ));
        }

        if !run_tests {
            return Ok("COMPILATION OK".to_string());
        }

        // Run tests
        let test_cmd = format!("cd {} && cargo test 2>&1", project_dir.display());
        let test_result = self
            .sandbox
            .execute(&test_cmd)
            .await
            .map_err(|e| e.to_string())?;

        if test_result.is_success() {
            Ok(format!("ALL TESTS PASSED:\n{}", test_result.stdout))
        } else {
            Ok(format!(
                "TESTS FAILED:\n{}{}",
                test_result.stdout, test_result.stderr
            ))
        }
    }

    /// Validate Python code.
    async fn validate_python(
        &self,
        code: &str,
        project_dir: &std::path::Path,
        run_tests: bool,
    ) -> Result<String, String> {
        std::fs::create_dir_all(project_dir).map_err(|e| e.to_string())?;
        std::fs::write(project_dir.join("code.py"), code).map_err(|e| e.to_string())?;

        // Syntax check
        let check_cmd = format!(
            "cd {} && python3 -m py_compile code.py 2>&1",
            project_dir.display()
        );
        let check_result = self
            .sandbox
            .execute(&check_cmd)
            .await
            .map_err(|e| e.to_string())?;

        if !check_result.is_success() {
            return Ok(format!(
                "SYNTAX ERROR:\n{}{}",
                check_result.stdout, check_result.stderr
            ));
        }

        if !run_tests {
            return Ok("SYNTAX OK".to_string());
        }

        // Run with pytest if available
        let test_cmd = format!(
            "cd {} && python3 -m pytest code.py -v 2>&1 || python3 code.py 2>&1",
            project_dir.display()
        );
        let test_result = self
            .sandbox
            .execute(&test_cmd)
            .await
            .map_err(|e| e.to_string())?;

        Ok(format!(
            "EXECUTION RESULT:\n{}{}",
            test_result.stdout, test_result.stderr
        ))
    }

    /// Analyze code tool implementation.
    async fn analyze_code(&self, args: Value) -> MCPToolResult {
        let code = args["code"].as_str().unwrap_or("");
        let language = args["language"].as_str().unwrap_or("rust");

        let analysis = match language {
            "rust" => self.analyze_rust(code).await,
            "python" => self.analyze_python(code).await,
            _ => Ok(format!("Analysis not available for: {}", language)),
        };

        match analysis {
            Ok(output) => MCPToolResult {
                content: vec![MCPContent::Text { text: output }],
                is_error: false,
            },
            Err(e) => MCPToolResult {
                content: vec![MCPContent::Text { text: e }],
                is_error: true,
            },
        }
    }

    /// Analyze Rust code with clippy.
    async fn analyze_rust(&self, code: &str) -> Result<String, String> {
        let project_dir = self
            .workspace
            .join(format!("analyze-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&project_dir).map_err(|e| e.to_string())?;

        let cargo_toml = r#"[package]
name = "analyze"
version = "0.1.0"
edition = "2021"
"#;
        std::fs::write(project_dir.join("Cargo.toml"), cargo_toml).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(project_dir.join("src")).map_err(|e| e.to_string())?;
        std::fs::write(project_dir.join("src/lib.rs"), code).map_err(|e| e.to_string())?;

        let cmd = format!(
            "cd {} && cargo clippy --all-targets -- -W clippy::all 2>&1",
            project_dir.display()
        );
        let result = self
            .sandbox
            .execute(&cmd)
            .await
            .map_err(|e| e.to_string())?;

        let _ = std::fs::remove_dir_all(&project_dir);

        Ok(format!(
            "CLIPPY ANALYSIS:\n{}{}",
            result.stdout, result.stderr
        ))
    }

    /// Analyze Python code with pylint/flake8.
    async fn analyze_python(&self, code: &str) -> Result<String, String> {
        let project_dir = self
            .workspace
            .join(format!("analyze-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&project_dir).map_err(|e| e.to_string())?;
        std::fs::write(project_dir.join("code.py"), code).map_err(|e| e.to_string())?;

        let cmd = format!(
            "cd {} && (python3 -m flake8 code.py 2>&1 || python3 -m pylint code.py 2>&1 || echo 'No linter available')",
            project_dir.display()
        );
        let result = self
            .sandbox
            .execute(&cmd)
            .await
            .map_err(|e| e.to_string())?;

        let _ = std::fs::remove_dir_all(&project_dir);

        Ok(format!(
            "PYTHON ANALYSIS:\n{}{}",
            result.stdout, result.stderr
        ))
    }

    /// Security audit tool implementation.
    async fn audit_security(&self, args: Value) -> MCPToolResult {
        let code = args["code"].as_str().unwrap_or("");
        let language = args["language"].as_str().unwrap_or("rust");
        let threshold = args["severity_threshold"].as_str().unwrap_or("medium");

        let mut findings = Vec::new();

        // Common security patterns to check
        let security_patterns: HashMap<&str, Vec<(&str, &str, &str)>> = HashMap::from([
            (
                "rust",
                vec![
                    ("unsafe", "high", "Unsafe code block detected"),
                    (
                        "std::process::Command",
                        "medium",
                        "Command execution - check for injection",
                    ),
                    (
                        "std::fs::write",
                        "low",
                        "File write - verify path sanitization",
                    ),
                    (
                        "unwrap()",
                        "low",
                        "Panic on error - consider proper error handling",
                    ),
                    ("panic!", "medium", "Explicit panic - may cause DoS"),
                ],
            ),
            (
                "python",
                vec![
                    (
                        "eval(",
                        "critical",
                        "eval() is dangerous - code injection risk",
                    ),
                    (
                        "exec(",
                        "critical",
                        "exec() is dangerous - code injection risk",
                    ),
                    (
                        "subprocess",
                        "high",
                        "Subprocess execution - check for injection",
                    ),
                    ("os.system", "high", "OS command execution - injection risk"),
                    (
                        "pickle.load",
                        "high",
                        "Pickle deserialization - arbitrary code execution",
                    ),
                    ("__import__", "medium", "Dynamic import - check source"),
                ],
            ),
            (
                "typescript",
                vec![
                    (
                        "eval(",
                        "critical",
                        "eval() is dangerous - code injection risk",
                    ),
                    ("dangerouslySetInnerHTML", "high", "XSS vulnerability risk"),
                    ("innerHTML", "high", "XSS vulnerability risk"),
                    (
                        "child_process",
                        "high",
                        "Command execution - check for injection",
                    ),
                    ("fs.writeFile", "medium", "File write - verify path"),
                ],
            ),
        ]);

        let severity_order = ["low", "medium", "high", "critical"];
        let threshold_idx = severity_order
            .iter()
            .position(|&s| s == threshold)
            .unwrap_or(1);

        if let Some(patterns) = security_patterns.get(language) {
            for (pattern, severity, description) in patterns {
                if code.contains(pattern) {
                    let sev_idx = severity_order
                        .iter()
                        .position(|&s| s == *severity)
                        .unwrap_or(0);
                    if sev_idx >= threshold_idx {
                        findings.push(format!(
                            "[{}] {}: {}",
                            severity.to_uppercase(),
                            pattern,
                            description
                        ));
                    }
                }
            }
        }

        let output = if findings.is_empty() {
            format!(
                "SECURITY AUDIT ({}):\nNo issues found above {} severity.",
                language, threshold
            )
        } else {
            format!(
                "SECURITY AUDIT ({}):\nFound {} issues:\n\n{}",
                language,
                findings.len(),
                findings.join("\n")
            )
        };

        MCPToolResult {
            content: vec![MCPContent::Text { text: output }],
            is_error: false,
        }
    }

    /// Handle JSON-RPC request.
    pub async fn handle_request(&self, request: Value) -> Value {
        let method = request["method"].as_str().unwrap_or("");
        let id = request.get("id").cloned().unwrap_or(Value::Null);

        let result = match method {
            "tools/list" => {
                json!({
                    "tools": self.list_tools()
                })
            }
            "tools/call" => {
                let name = request["params"]["name"].as_str().unwrap_or("");
                let args = request["params"]["arguments"].clone();
                let result = self.call_tool(name, args).await;
                serde_json::to_value(result).unwrap_or(Value::Null)
            }
            _ => {
                return json!({
                    "jsonrpc": "2.0",
                    "error": {
                        "code": -32601,
                        "message": format!("Method not found: {}", method)
                    },
                    "id": id
                });
            }
        };

        json!({
            "jsonrpc": "2.0",
            "result": result,
            "id": id
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentic_sandbox::ProcessSandbox;

    #[tokio::test]
    async fn test_list_tools() {
        let sandbox = ProcessSandbox::new();
        let server = ValidatorMCPServer::new(sandbox, std::path::PathBuf::from("/tmp"));
        let tools = server.list_tools();
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].name, "validate-code");
    }

    #[test]
    fn test_security_audit_finds_issues() {
        let code = r#"
fn main() {
    let cmd = std::process::Command::new("ls").output().unwrap();
    unsafe { do_something() };
}
"#;
        let args = json!({
            "code": code,
            "language": "rust",
            "severity_threshold": "medium"
        });

        let sandbox = ProcessSandbox::new();
        let server = ValidatorMCPServer::new(sandbox, std::path::PathBuf::from("/tmp"));

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(server.audit_security(args));

        assert!(!result.is_error);
        let MCPContent::Text { text } = &result.content[0];
        assert!(text.contains("unsafe"));
        assert!(text.contains("Command"));
    }
}
