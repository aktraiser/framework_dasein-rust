//! SandboxValidator - Ground truth validation using real code execution.
//!
//! Unlike the rule-based [`Validator`], this validator actually compiles and
//! runs the code to detect real errors. This eliminates the "LLM reviewing LLM"
//! bias problem by using objective, executable feedback.
//!
//! # Why Ground Truth Validation?
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │  LLM Review (Validator)    vs    Ground Truth (Sandbox)    │
//! ├─────────────────────────────────────────────────────────────┤
//! │  "This looks correct"      vs    "error[E0308]: expected   │
//! │  Score: 8/10                      i32, found &str"         │
//! │  Subjective                       Objective                 │
//! │  Can miss bugs                    Catches all compile errs  │
//! │  Fast                             Slower but accurate       │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Supported Languages
//!
//! - **Rust**: `cargo check` + `cargo test`
//! - **Python**: `python -m pytest`
//! - **Go**: `go build` + `go test`
//! - **TypeScript**: `tsc --noEmit` + `npm test`
//!
//! # Example: Basic Validation
//!
//! ```rust,no_run
//! use dasein_agentic_core::distributed::SandboxValidator;
//! use dasein_agentic_sandbox::ProcessSandbox;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let sandbox = ProcessSandbox::new();
//! let validator = SandboxValidator::new(sandbox);
//!
//! let code = r#"
//!     pub fn add(a: i32, b: i32) -> i32 { a + b }
//!
//!     #[test]
//!     fn test_add() { assert_eq!(add(2, 3), 5); }
//! "#;
//!
//! let result = validator.validate_rust_code(code).await?;
//!
//! if result.passed {
//!     println!("Code compiles and all tests pass!");
//! } else if !result.compiles {
//!     println!("Compiler errors: {:?}", result.compiler_errors);
//! } else {
//!     println!("Test failures: {:?}", result.test_errors);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # Example: Grounded Feedback Loop
//!
//! ```rust,no_run
//! # use dasein_agentic_core::distributed::{Executor, SandboxValidator};
//! # use dasein_agentic_sandbox::ProcessSandbox;
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let executor = Executor::new("exe-001", "sup-001").build();
//! let validator = SandboxValidator::new(ProcessSandbox::new());
//!
//! let mut code = executor.execute("system", "write fibonacci").await?.content;
//!
//! for attempt in 0..5 {
//!     let result = validator.validate_rust_code(&code).await?;
//!
//!     if result.passed {
//!         println!("Success after {} attempts!", attempt + 1);
//!         break;
//!     }
//!
//!     // Feed real errors back to LLM
//!     let feedback = result.feedback.unwrap_or_default();
//!     code = executor.execute("system", &format!(
//!         "Fix this code:\n{}\n\nErrors:\n{}", code, feedback
//!     )).await?.content;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! See `examples/grounded_loop.rs` for a complete implementation.

use dasein_agentic_sandbox::{ExecutionResult as SandboxExecutionResult, Sandbox, SandboxError};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::OnceLock;
use tracing::{debug, info, instrument};

// Pre-compiled regex patterns for parsing test output (avoids regex compilation in loops)
fn passed_failed_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\d+) passed; (\d+) failed").unwrap())
}

fn passed_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\d+) passed").unwrap())
}

fn failed_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\d+) failed").unwrap())
}

fn total_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(\d+) total").unwrap())
}

/// Result of sandbox validation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxValidationResult {
    /// Overall pass/fail
    pub passed: bool,
    /// Compilation passed
    pub compiles: bool,
    /// Tests passed (if any)
    pub tests_passed: bool,
    /// Number of tests run
    pub test_count: u32,
    /// Number of tests passed
    pub tests_ok: u32,
    /// Number of tests failed
    pub tests_failed: u32,
    /// Compiler errors (if any)
    pub compiler_errors: Vec<String>,
    /// Test failures (if any)
    pub test_errors: Vec<String>,
    /// Warnings (non-blocking)
    pub warnings: Vec<String>,
    /// Structured feedback for LLM
    pub feedback: Option<String>,
    /// Total execution time in ms
    pub execution_time_ms: u64,
}

impl SandboxValidationResult {
    /// Create a successful result.
    pub fn success(test_count: u32, execution_time_ms: u64) -> Self {
        Self {
            passed: true,
            compiles: true,
            tests_passed: true,
            test_count,
            tests_ok: test_count,
            tests_failed: 0,
            compiler_errors: vec![],
            test_errors: vec![],
            warnings: vec![],
            feedback: None,
            execution_time_ms,
        }
    }

    /// Create a compilation failure result.
    pub fn compile_error(errors: Vec<String>, execution_time_ms: u64) -> Self {
        // SAFEGUARD: Never have passed=false with empty errors
        // This prevents confusing "0 errors but failed" scenarios
        let errors = if errors.is_empty() {
            vec![
                "Compilation failed but no specific error was captured. Check command output."
                    .to_string(),
            ]
        } else {
            errors
        };
        let feedback = format!(
            "COMPILATION FAILED:\n\n{}\n\nFix these errors and try again.",
            errors.join("\n")
        );
        Self {
            passed: false,
            compiles: false,
            tests_passed: false,
            test_count: 0,
            tests_ok: 0,
            tests_failed: 0,
            compiler_errors: errors,
            test_errors: vec![],
            warnings: vec![],
            feedback: Some(feedback),
            execution_time_ms,
        }
    }

    /// Create a test failure result.
    pub fn test_error(
        test_count: u32,
        tests_ok: u32,
        errors: Vec<String>,
        execution_time_ms: u64,
    ) -> Self {
        // SAFEGUARD: Never have passed=false with empty errors
        let errors = if errors.is_empty() {
            vec!["Tests failed but no specific error was captured. Check test output.".to_string()]
        } else {
            errors
        };
        let feedback = format!(
            "TESTS FAILED: {}/{} passed\n\nFailures:\n{}\n\nFix these test failures.",
            tests_ok,
            test_count,
            errors.join("\n")
        );
        Self {
            passed: false,
            compiles: true,
            tests_passed: false,
            test_count,
            tests_ok,
            tests_failed: test_count - tests_ok,
            compiler_errors: vec![],
            test_errors: errors,
            warnings: vec![],
            feedback: Some(feedback),
            execution_time_ms,
        }
    }

    /// Create a lint failure result (clippy, flake8, etc.).
    /// Code compiles but has anti-patterns or code quality issues.
    pub fn lint_error(errors: Vec<String>, execution_time_ms: u64) -> Self {
        // SAFEGUARD: Never have passed=false with empty errors
        let errors = if errors.is_empty() {
            vec![
                "Lint check failed but no specific error was captured. Check linter output."
                    .to_string(),
            ]
        } else {
            errors
        };
        let feedback = format!(
            "LINT ERRORS (code compiles but has issues):\n\n{}\n\nFix these anti-patterns.",
            errors.join("\n")
        );
        Self {
            passed: false,
            compiles: true, // It compiles, but has lint issues
            tests_passed: false,
            test_count: 0,
            tests_ok: 0,
            tests_failed: 0,
            compiler_errors: vec![],
            test_errors: errors, // Use test_errors to store lint errors
            warnings: vec![],
            feedback: Some(feedback),
            execution_time_ms,
        }
    }
}

/// Language for code validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    Shell,
}

impl Language {
    /// Get file extension for this language.
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Rust => "rs",
            Self::Python => "py",
            Self::JavaScript => "js",
            Self::TypeScript => "ts",
            Self::Go => "go",
            Self::Shell => "sh",
        }
    }

    /// Get compile command (if applicable).
    pub fn compile_command(&self, project_dir: &str) -> Option<String> {
        match self {
            Self::Rust => Some(format!("cd {} && cargo check 2>&1", project_dir)),
            Self::Go => Some(format!("cd {} && go build ./... 2>&1", project_dir)),
            // Use tsc directly (globally installed)
            Self::TypeScript => Some(format!("cd {} && tsc --noEmit 2>&1", project_dir)),
            _ => None,
        }
    }

    /// Get lint command (optional static analysis).
    /// Returns None if no linter is available for this language.
    pub fn lint_command(&self, project_dir: &str) -> Option<String> {
        match self {
            // Clippy catches: .forget() misuse, unused vars, anti-patterns
            Self::Rust => Some(format!(
                "cd {} && cargo clippy --all-targets -- -D warnings 2>&1",
                project_dir
            )),
            // Pylint/flake8 for Python
            Self::Python => Some(format!(
                "cd {} && python3 -m flake8 --max-line-length=120 2>&1 || true",
                project_dir
            )),
            // ESLint for JS/TS
            Self::JavaScript | Self::TypeScript => {
                Some(format!("cd {} && npx eslint . 2>&1 || true", project_dir))
            }
            // golint for Go
            Self::Go => Some(format!("cd {} && go vet ./... 2>&1", project_dir)),
            _ => None,
        }
    }

    /// Get test command.
    pub fn test_command(&self, project_dir: &str) -> String {
        match self {
            Self::Rust => format!("cd {} && cargo test 2>&1", project_dir),
            Self::Python => format!("cd {} && python3 -m pytest -v 2>&1", project_dir),
            Self::JavaScript => format!("cd {} && npm test 2>&1", project_dir),
            // Use jest directly with NODE_PATH for global packages
            Self::TypeScript => format!(
                "cd {} && NODE_PATH=$(npm root -g) jest --config jest.config.js 2>&1",
                project_dir
            ),
            Self::Go => format!("cd {} && go test ./... -v 2>&1", project_dir),
            Self::Shell => format!("cd {} && bash -n *.sh 2>&1", project_dir),
        }
    }
}

/// Sandbox-based code validator.
///
/// Uses real compilation and test execution for ground truth validation.
pub struct SandboxValidator<S: Sandbox> {
    sandbox: S,
    workspace: PathBuf,
    run_tests: bool,
}

impl<S: Sandbox> SandboxValidator<S> {
    /// Create a new sandbox validator.
    pub fn new(sandbox: S) -> Self {
        Self {
            sandbox,
            workspace: PathBuf::from("/tmp/agentic-validation"),
            run_tests: true,
        }
    }

    /// Set the workspace directory for validation files.
    pub fn workspace(mut self, path: impl Into<PathBuf>) -> Self {
        self.workspace = path.into();
        self
    }

    /// Enable/disable test execution.
    pub fn run_tests(mut self, run: bool) -> Self {
        self.run_tests = run;
        self
    }

    /// Validate Rust code with real compilation and tests.
    #[instrument(skip(self, code), fields(lang = "rust"))]
    pub async fn validate_rust_code(
        &self,
        code: &str,
    ) -> Result<SandboxValidationResult, SandboxError> {
        self.validate_code(code, Language::Rust).await
    }

    /// Validate code in any supported language.
    #[instrument(skip(self, code))]
    pub async fn validate_code(
        &self,
        code: &str,
        language: Language,
    ) -> Result<SandboxValidationResult, SandboxError> {
        let start = std::time::Instant::now();
        let project_id = uuid::Uuid::new_v4().to_string();
        let project_dir = self.workspace.join(&project_id);

        info!(
            "Validating {:?} code in {}",
            language,
            project_dir.display()
        );

        // Setup project structure based on language
        let setup_result = match language {
            Language::Rust => self.setup_rust_project(&project_dir, code).await,
            Language::Python => self.setup_python_project(&project_dir, code).await,
            Language::TypeScript => self.setup_typescript_project(&project_dir, code).await,
            Language::Go => self.setup_go_project(&project_dir, code).await,
            _ => {
                self.setup_generic_project(&project_dir, code, language)
                    .await
            }
        }?;

        if !setup_result.is_success() {
            return Ok(SandboxValidationResult::compile_error(
                vec![format!("Project setup failed: {}", setup_result.stderr)],
                start.elapsed().as_millis() as u64,
            ));
        }

        // Compile if language requires it
        if let Some(compile_cmd) = language.compile_command(project_dir.to_str().unwrap_or("")) {
            debug!("Running compile: {}", compile_cmd);
            let compile_result = self.sandbox.execute(&compile_cmd).await?;

            if !compile_result.is_success() {
                let errors =
                    Self::parse_compiler_errors(&compile_result.stdout, &compile_result.stderr);
                // Cleanup
                let _ = self
                    .sandbox
                    .execute(&format!("rm -rf {}", project_dir.display()))
                    .await;
                return Ok(SandboxValidationResult::compile_error(
                    errors,
                    start.elapsed().as_millis() as u64,
                ));
            }

            // Extract warnings
            let warnings = Self::extract_warnings(&compile_result.stdout, &compile_result.stderr);
            if !warnings.is_empty() {
                debug!("Warnings: {:?}", warnings);
            }
        }

        // Run linter (clippy for Rust) to catch anti-patterns
        if let Some(lint_cmd) = language.lint_command(project_dir.to_str().unwrap_or("")) {
            debug!("Running linter: {}", lint_cmd);
            let lint_result = self.sandbox.execute(&lint_cmd).await?;

            // Clippy errors are treated as failures (we use -D warnings)
            if !lint_result.is_success() {
                let errors = Self::parse_clippy_errors(&lint_result.stdout, &lint_result.stderr);
                if !errors.is_empty() {
                    // Cleanup
                    let _ = self
                        .sandbox
                        .execute(&format!("rm -rf {}", project_dir.display()))
                        .await;
                    return Ok(SandboxValidationResult::lint_error(
                        errors,
                        start.elapsed().as_millis() as u64,
                    ));
                }
            }
        }

        // Run tests if enabled
        if self.run_tests {
            let test_cmd = language.test_command(project_dir.to_str().unwrap_or(""));
            debug!("Running tests: {}", test_cmd);
            let test_result = self.sandbox.execute(&test_cmd).await?;

            // Cleanup
            let _ = self
                .sandbox
                .execute(&format!("rm -rf {}", project_dir.display()))
                .await;

            // Parse test results
            let (total, passed, failed, errors) =
                Self::parse_test_results(&test_result.stdout, &test_result.stderr, language);

            if failed > 0 || !test_result.is_success() {
                // If no specific errors captured but test failed, include raw output
                let final_errors = if errors.is_empty() {
                    let combined = format!("{}\n{}", test_result.stdout, test_result.stderr);

                    // Always include raw output - don't filter too aggressively
                    let output_lines: Vec<&str> = combined.lines().collect();
                    let truncated = if output_lines.len() > 100 {
                        // Too long, take last 80 lines
                        output_lines[output_lines.len() - 80..].join("\n")
                    } else {
                        combined.clone()
                    };

                    // If still empty, note that
                    let final_output = if truncated.trim().is_empty() {
                        format!(
                            "Test failed with exit code {} but no output captured. \
                            Check for panics, timeouts, or infinite loops.",
                            test_result.exit_code
                        )
                    } else {
                        format!(
                            "Test failed (exit code {}):\n{}",
                            test_result.exit_code, truncated
                        )
                    };

                    vec![final_output]
                } else {
                    errors
                };

                return Ok(SandboxValidationResult::test_error(
                    total,
                    passed,
                    final_errors,
                    start.elapsed().as_millis() as u64,
                ));
            }

            Ok(SandboxValidationResult::success(
                total,
                start.elapsed().as_millis() as u64,
            ))
        } else {
            // Cleanup
            let _ = self
                .sandbox
                .execute(&format!("rm -rf {}", project_dir.display()))
                .await;
            Ok(SandboxValidationResult::success(
                0,
                start.elapsed().as_millis() as u64,
            ))
        }
    }

    /// Setup a Rust project for validation.
    async fn setup_rust_project(
        &self,
        project_dir: &PathBuf,
        code: &str,
    ) -> Result<SandboxExecutionResult, SandboxError> {
        // Use base64 encoding to avoid all shell escaping issues
        let encoded = base64_encode(code);

        let setup_script = format!(
            r#"
mkdir -p {dir}/src && \
cat > {dir}/Cargo.toml << 'CARGO_EOF'
[package]
name = "validation_project"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = {{ version = "1", features = ["full"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
thiserror = "2"
anyhow = "1"
async-trait = "0.1"
futures = "0.3"
reqwest = {{ version = "0.12", features = ["json"] }}

[lints.clippy]
# Allow common false positives - focus on real errors, not style
new_without_default = "allow"
must_use_candidate = "allow"
missing_errors_doc = "allow"
missing_panics_doc = "allow"
module_name_repetitions = "allow"
CARGO_EOF
echo '{encoded}' | base64 -d > {dir}/src/lib.rs
"#,
            dir = project_dir.display(),
            encoded = encoded
        );

        self.sandbox.execute(&setup_script).await
    }

    /// Setup a Python project for validation.
    async fn setup_python_project(
        &self,
        project_dir: &PathBuf,
        code: &str,
    ) -> Result<SandboxExecutionResult, SandboxError> {
        let encoded = base64_encode(code);

        // Check if code already contains tests
        let has_tests = code.contains("def test_") || code.contains("import pytest");
        // Check if code uses async tests (pytest.mark.asyncio or async def test_)
        let has_async_tests =
            code.contains("@pytest.mark.asyncio") || code.contains("async def test_");

        let setup_script = if has_tests {
            if has_async_tests {
                // Async tests need pytest-asyncio + conftest.py
                format!(
                    r#"
pip3 install pytest-asyncio -q 2>/dev/null || true && \
mkdir -p {dir} && \
echo '{encoded}' | base64 -d > {dir}/test_main.py && \
cat > {dir}/conftest.py << 'CONFTEST_EOF'
import pytest
pytest_plugins = ('pytest_asyncio',)
CONFTEST_EOF
"#,
                    dir = project_dir.display(),
                    encoded = encoded
                )
            } else {
                // Sync tests - just the test file
                format!(
                    r"
mkdir -p {dir} && \
echo '{encoded}' | base64 -d > {dir}/test_main.py
",
                    dir = project_dir.display(),
                    encoded = encoded
                )
            }
        } else {
            // No tests - create main.py and a stub test file
            format!(
                r"
mkdir -p {dir} && \
echo '{encoded}' | base64 -d > {dir}/main.py && \
cat > {dir}/test_main.py << 'TEST_EOF'
import pytest
from main import *

# Auto-generated test stub
def test_placeholder():
    pass
TEST_EOF
",
                dir = project_dir.display(),
                encoded = encoded
            )
        };

        self.sandbox.execute(&setup_script).await
    }

    /// Setup a TypeScript project for validation.
    /// Uses globally installed packages (typescript, jest, ts-jest) to avoid slow npm install.
    async fn setup_typescript_project(
        &self,
        project_dir: &PathBuf,
        code: &str,
    ) -> Result<SandboxExecutionResult, SandboxError> {
        let encoded = base64_encode(code);

        // Check if code contains tests (Jest)
        let has_tests =
            code.contains("describe(") || code.contains("it(") || code.contains("test(");
        let filename = if has_tests { "main.test.ts" } else { "main.ts" };

        // Generate appropriate tsconfig based on whether we have tests
        // Without tests: don't require @types/jest (avoids TS2688 error)
        // With tests: include jest types for describe/it/expect
        let tsconfig_types = if has_tests {
            r#""types": ["jest", "node"]"#
        } else {
            // No types field = TypeScript auto-detects, no jest required
            r#""types": []"#
        };

        // Use globally installed packages - no npm install required
        // Requires: npm install -g typescript jest ts-jest @types/jest @types/node
        // NOTE: typeRoots points to global npm types location so tsc can find @types/jest etc.
        let setup_script = format!(
            r#"
mkdir -p {dir} && \
echo '{encoded}' | base64 -d > {dir}/{filename} && \
cat > {dir}/package.json << 'PKG_EOF'
{{
  "name": "validation",
  "version": "1.0.0",
  "scripts": {{
    "test": "jest --config jest.config.js"
  }}
}}
PKG_EOF
cat > {dir}/tsconfig.json << TS_EOF
{{
  "compilerOptions": {{
    "target": "ES2020",
    "module": "commonjs",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "outDir": "./dist",
    "typeRoots": ["/usr/lib/node_modules/@types", "./node_modules/@types"],
    {tsconfig_types}
  }},
  "include": ["*.ts"]
}}
TS_EOF
cat > {dir}/jest.config.js << 'JEST_EOF'
module.exports = {{
  transform: {{
    '^.+\\.tsx?$': ['ts-jest', {{ tsconfig: 'tsconfig.json' }}]
  }},
  testEnvironment: 'node',
  testMatch: ['**/*.test.ts'],
  moduleFileExtensions: ['ts', 'tsx', 'js', 'jsx', 'json', 'node'],
}};
JEST_EOF
echo "TypeScript project setup complete (using global packages)"
"#,
            dir = project_dir.display(),
            encoded = encoded,
            filename = filename
        );

        self.sandbox.execute(&setup_script).await
    }

    /// Setup a Go project for validation.
    async fn setup_go_project(
        &self,
        project_dir: &PathBuf,
        code: &str,
    ) -> Result<SandboxExecutionResult, SandboxError> {
        let encoded = base64_encode(code);

        // Check if code contains tests
        let has_tests = code.contains("func Test") || code.contains("testing.T");
        let filename = if has_tests { "main_test.go" } else { "main.go" };

        let setup_script = format!(
            r"
mkdir -p {dir} && \
echo '{encoded}' | base64 -d > {dir}/{filename} && \
cd {dir} && go mod init validation 2>&1
",
            dir = project_dir.display(),
            encoded = encoded,
            filename = filename
        );

        self.sandbox.execute(&setup_script).await
    }

    /// Setup a generic project.
    async fn setup_generic_project(
        &self,
        project_dir: &PathBuf,
        code: &str,
        language: Language,
    ) -> Result<SandboxExecutionResult, SandboxError> {
        let filename = format!("main.{}", language.extension());
        let encoded = base64_encode(code);
        let setup_script = format!(
            r"
mkdir -p {dir} && \
echo '{encoded}' | base64 -d > {dir}/{filename}
",
            dir = project_dir.display(),
            filename = filename,
            encoded = encoded
        );

        self.sandbox.execute(&setup_script).await
    }

    /// Parse compiler error output.
    fn parse_compiler_errors(stdout: &str, stderr: &str) -> Vec<String> {
        let combined = format!("{}\n{}", stdout, stderr);
        let mut errors = Vec::new();

        for line in combined.lines() {
            // Check if line matches any error pattern
            let is_rust_error = line.contains("error[E") || line.starts_with("error:");
            let is_ts_error = line.contains("): error TS") || line.contains(": error TS");
            let is_go_error = line.contains(".go:")
                && (line.contains("undefined")
                    || line.contains("cannot")
                    || line.contains("expected")
                    || line.contains("invalid"));

            if is_rust_error || is_ts_error || is_go_error {
                errors.push(line.to_string());
            }

            // Also capture the context lines (up to 5 after error)
            if line.contains(" --> ") || line.contains(" | ") {
                if let Some(last) = errors.last_mut() {
                    last.push('\n');
                    last.push_str(line);
                }
            }
        }

        // Fallback: if no structured errors, return combined output
        // NOTE: Many commands use 2>&1 which redirects stderr to stdout,
        // so we must check combined output, not just stderr
        if errors.is_empty() {
            let combined_trimmed = combined.trim();
            if !combined_trimmed.is_empty() {
                errors.push(combined_trimmed.to_string());
            }
        }

        errors
    }

    /// Extract warnings from output.
    fn extract_warnings(stdout: &str, stderr: &str) -> Vec<String> {
        let combined = format!("{}\n{}", stdout, stderr);
        combined
            .lines()
            .filter(|line| line.contains("warning:") || line.contains("warn["))
            .map(String::from)
            .collect()
    }

    /// Parse clippy/lint error output.
    /// Clippy output looks like:
    /// "warning: ... --> src/lib.rs:10:5 ... help: consider using ..."
    /// With -D warnings, these become errors.
    fn parse_clippy_errors(stdout: &str, stderr: &str) -> Vec<String> {
        let combined = format!("{}\n{}", stdout, stderr);
        let mut errors = Vec::new();
        let mut current_error = String::new();

        for line in combined.lines() {
            // Clippy error/warning pattern
            if line.starts_with("error:") || line.starts_with("warning:") {
                // Save previous error if any
                if !current_error.is_empty() {
                    errors.push(current_error.clone());
                }
                current_error = line.to_string();
            }
            // Context lines (location, help suggestions)
            // Context lines and help suggestions
            else if line.contains(" --> ")
                || line.contains(" | ")
                || line.trim().starts_with("= help:")
                || line.trim().starts_with("help:")
            {
                if !current_error.is_empty() {
                    current_error.push('\n');
                    current_error.push_str(line);
                }
            }
        }

        // Don't forget the last error
        if !current_error.is_empty() {
            errors.push(current_error);
        }

        // Filter out non-actionable items (aborting due to errors, etc.)
        errors
            .into_iter()
            .filter(|e| !e.contains("aborting due to") && !e.contains("could not compile"))
            .collect()
    }

    /// Parse test results.
    fn parse_test_results(
        stdout: &str,
        stderr: &str,
        language: Language,
    ) -> (u32, u32, u32, Vec<String>) {
        let combined = format!("{}\n{}", stdout, stderr);

        match language {
            Language::Rust => Self::parse_rust_test_output(&combined),
            Language::Python => Self::parse_pytest_output(&combined),
            Language::TypeScript | Language::JavaScript => Self::parse_jest_output(&combined),
            Language::Go => Self::parse_go_test_output(&combined),
            _ => (0, 0, 0, vec![combined]),
        }
    }

    /// Parse Rust test output (cargo test).
    fn parse_rust_test_output(output: &str) -> (u32, u32, u32, Vec<String>) {
        let mut total = 0u32;
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut errors = Vec::new();
        let lines: Vec<&str> = output.lines().collect();

        for (i, &line) in lines.iter().enumerate() {
            // Parse summary line: "test result: ok. 5 passed; 0 failed; 0 ignored"
            // or "test result: FAILED. X passed; Y failed"
            if line.starts_with("test result:") {
                if let Some(caps) = passed_failed_regex().captures(line) {
                    passed = caps
                        .get(1)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(0);
                    failed = caps
                        .get(2)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(0);
                    total = passed + failed;
                }
            }

            // Capture failed test names (e.g., "test tests::test_cycle ... FAILED")
            if line.contains("FAILED") && !line.starts_with("test result:") {
                errors.push(line.to_string());
            }

            // Capture assertion failures with context (next 5 lines)
            if line.contains("assertion `left == right` failed")
                || line.contains("assertion failed")
                || line.contains("panicked at")
                || line.contains("thread 'main' panicked")
                || line.contains("thread '") && line.contains("' panicked")
            {
                let mut context = line.to_string();
                // Add following lines for context (left/right values, etc.)
                for j in 1..=5 {
                    if i + j < lines.len() {
                        let next_line = lines[i + j].trim();
                        if !next_line.is_empty() && !next_line.starts_with("note:") {
                            context.push('\n');
                            context.push_str(next_line);
                        }
                    }
                }
                errors.push(context);
            }

            // Capture "failures:" section header and subsequent test names
            if line.trim() == "failures:" {
                // Collect the failure list
                for j in 1..=20 {
                    if i + j < lines.len() {
                        let next_line = lines[i + j].trim();
                        if next_line.is_empty() || next_line.starts_with("test result:") {
                            break;
                        }
                        if !next_line.starts_with("----") {
                            errors.push(format!("Failed: {}", next_line));
                        }
                    }
                }
            }
        }

        // IMPORTANT: If we detected failures but no specific errors, include raw output
        if failed > 0 && errors.is_empty() {
            // Include the most relevant parts of the output
            let truncated: String = output
                .lines()
                .filter(|l| {
                    l.contains("FAILED")
                        || l.contains("error")
                        || l.contains("panicked")
                        || l.contains("assertion")
                        || l.starts_with("test ")
                })
                .take(20)
                .collect::<Vec<_>>()
                .join("\n");

            if !truncated.is_empty() {
                errors.push(truncated);
            } else {
                // Last resort: include last 30 lines
                let last_lines: Vec<&str> = output.lines().collect();
                let start = last_lines.len().saturating_sub(30);
                errors.push(format!(
                    "Test output (last 30 lines):\n{}",
                    last_lines[start..].join("\n")
                ));
            }
        }

        (total, passed, failed, errors)
    }

    /// Parse pytest output.
    fn parse_pytest_output(output: &str) -> (u32, u32, u32, Vec<String>) {
        let mut total = 0u32;
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut errors = Vec::new();

        for line in output.lines() {
            // Parse summary: "5 passed, 2 failed"
            if line.contains("passed") || line.contains("failed") {
                if let Some(caps) = passed_regex().captures(line) {
                    passed = caps
                        .get(1)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(0);
                }
                if let Some(caps) = failed_regex().captures(line) {
                    failed = caps
                        .get(1)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(0);
                }
                total = passed + failed;
            }

            // Capture FAILED tests
            if line.contains("FAILED") || line.contains("AssertionError") {
                errors.push(line.to_string());
            }
        }

        (total, passed, failed, errors)
    }

    /// Parse Jest output (TypeScript/JavaScript).
    fn parse_jest_output(output: &str) -> (u32, u32, u32, Vec<String>) {
        let mut total = 0u32;
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut errors = Vec::new();

        for line in output.lines() {
            // Parse summary: "Tests:       1 passed, 1 total" or "Tests:       1 failed, 2 passed, 3 total"
            if line.contains("Tests:") && line.contains("total") {
                if let Some(caps) = passed_regex().captures(line) {
                    passed = caps
                        .get(1)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(0);
                }
                if let Some(caps) = failed_regex().captures(line) {
                    failed = caps
                        .get(1)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(0);
                }
                if let Some(caps) = total_regex().captures(line) {
                    total = caps
                        .get(1)
                        .and_then(|m| m.as_str().parse().ok())
                        .unwrap_or(0);
                }
            }

            // Capture FAIL lines
            if line.contains("FAIL ") || line.contains("✕") || line.contains("● ") {
                errors.push(line.to_string());
            }
            // Capture error details
            if line.contains("Error:") || line.contains("expect(") || line.contains("toBe(") {
                errors.push(line.to_string());
            }
        }

        (total, passed, failed, errors)
    }

    /// Parse Go test output.
    fn parse_go_test_output(output: &str) -> (u32, u32, u32, Vec<String>) {
        let mut total = 0u32;
        let mut passed = 0u32;
        let mut failed = 0u32;
        let mut errors = Vec::new();

        for line in output.lines() {
            // Count tests: "--- PASS:" or "--- FAIL:"
            if line.contains("--- PASS:") {
                passed += 1;
                total += 1;
            } else if line.contains("--- FAIL:") {
                failed += 1;
                total += 1;
                errors.push(line.to_string());
            }
            // Also capture error messages
            if line.contains("Error Trace:") || line.contains("Error:") || line.contains("FAIL\t") {
                errors.push(line.to_string());
            }
            // Go compile errors
            if line.contains(".go:")
                && (line.contains("undefined")
                    || line.contains("cannot")
                    || line.contains("expected"))
            {
                errors.push(line.to_string());
            }
        }

        (total, passed, failed, errors)
    }
}

/// Base64 encode a string (simple implementation without external deps).
fn base64_encode(input: &str) -> String {
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::new();

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;

        let n = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARSET[((n >> 18) & 0x3F) as usize] as char);
        result.push(CHARSET[((n >> 12) & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            result.push(CHARSET[((n >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(CHARSET[(n & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use dasein_agentic_sandbox::ProcessSandbox;

    #[tokio::test]
    async fn test_valid_rust_code() {
        let sandbox = ProcessSandbox::new().with_timeout(60000);
        let validator = SandboxValidator::new(sandbox).run_tests(false);

        let code = r#"
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add() {
        assert_eq!(add(2, 3), 5);
    }
}
"#;

        let result = validator.validate_rust_code(code).await.unwrap();
        assert!(
            result.compiles,
            "Code should compile: {:?}",
            result.compiler_errors
        );
    }

    #[tokio::test]
    async fn test_invalid_rust_code() {
        let sandbox = ProcessSandbox::new().with_timeout(60000);
        let validator = SandboxValidator::new(sandbox);

        let code = r#"
pub fn broken() -> i32 {
    let x = "not an integer";
    x  // Type error: expected i32, found &str
}
"#;

        let result = validator.validate_rust_code(code).await.unwrap();
        assert!(!result.compiles, "Code should not compile");
        assert!(!result.compiler_errors.is_empty(), "Should have errors");
        assert!(result.feedback.is_some(), "Should have feedback");
    }

    #[test]
    fn test_parse_rust_errors() {
        let stderr = r#"
error[E0308]: mismatched types
 --> src/lib.rs:4:5
  |
3 | pub fn broken() -> i32 {
  |                    --- expected `i32` because of return type
4 |     "hello"
  |     ^^^^^^^ expected `i32`, found `&str`
"#;

        let errors = SandboxValidator::<ProcessSandbox>::parse_compiler_errors("", stderr);
        assert!(!errors.is_empty());
        assert!(errors[0].contains("E0308"));
    }

    #[test]
    fn test_parse_rust_test_results() {
        let output = r#"
running 3 tests
test tests::test_one ... ok
test tests::test_two ... FAILED
test tests::test_three ... ok

failures:

---- tests::test_two stdout ----
thread 'tests::test_two' panicked at src/lib.rs:15:9:
assertion `left == right` failed
  left: 1
 right: 2

test result: FAILED. 2 passed; 1 failed; 0 ignored
"#;

        let (total, passed, failed, errors) =
            SandboxValidator::<ProcessSandbox>::parse_rust_test_output(output);

        assert_eq!(total, 3);
        assert_eq!(passed, 2);
        assert_eq!(failed, 1);
        assert!(!errors.is_empty());
    }
}
