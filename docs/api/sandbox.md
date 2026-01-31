# agentic-sandbox

Sandboxed code execution environments.

## Installation

```toml
[dependencies]
agentic-sandbox = "0.1"
```

## Sandbox Implementations

### ProcessSandbox

Local process-based execution. Fast but **no isolation** - for development only.

```rust
use agentic_sandbox::ProcessSandbox;

let sandbox = ProcessSandbox::new()
    .with_timeout(30000)      // 30 seconds
    .with_shell("bash");      // or "sh", "zsh"

let result = sandbox.execute("echo 'Hello World'").await?;
println!("Output: {}", result.stdout);
```

### DockerSandbox

Container-based execution with full isolation. For production use.

```rust
use agentic_sandbox::DockerSandbox;

let sandbox = DockerSandbox::new("python:3.11-slim")
    .with_timeout(60000)
    .with_memory_limit("256m")
    .with_cpu_limit(1.0)
    .with_network(false);  // No network access

let result = sandbox.execute("print('Hello from container')").await?;
```

## Trait

### Sandbox

```rust
#[async_trait]
pub trait Sandbox: Send + Sync {
    async fn execute(&self, code: &str) -> Result<ExecutionResult, SandboxError>;
    async fn is_ready(&self) -> Result<bool, SandboxError>;
    async fn stop(&self) -> Result<(), SandboxError>;
}
```

## Types

### ExecutionResult

```rust
pub struct ExecutionResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub execution_time_ms: u64,
    pub artifacts: Vec<SandboxArtifact>,
}

impl ExecutionResult {
    pub fn is_success(&self) -> bool {
        self.exit_code == 0
    }
}
```

### SandboxArtifact

```rust
pub struct SandboxArtifact {
    pub name: String,
    pub content_type: String,
    pub data: Vec<u8>,
}
```

### SandboxError

```rust
pub enum SandboxError {
    Timeout,
    IoError(std::io::Error),
    ContainerError(String),
    NotReady,
    ExecutionFailed(String),
}
```

## Feature Flags

```toml
[dependencies]
agentic-sandbox = { version = "0.1", features = ["docker"] }
```

| Feature | Description |
|---------|-------------|
| `docker` | Enable DockerSandbox (requires bollard) |
