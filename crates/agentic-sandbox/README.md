# agentic-sandbox

Sandboxed code execution for the Agentic Framework.

## Features

- **Multiple Backends**: Process, Docker, Firecracker, Gateway
- **Language Support**: Python, Node.js, Rust, Go, Bash
- **Isolation**: Secure code execution in isolated environments
- **Async-first**: Built on Tokio

## Installation

```toml
[dependencies]
agentic-sandbox = "0.1"

# With Docker support
agentic-sandbox = { version = "0.1", features = ["docker"] }

# With Gateway (Firecracker)
agentic-sandbox = { version = "0.1", features = ["gateway"] }
```

## Quick Start

```rust
use agentic_sandbox::{ProcessSandbox, Sandbox};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sandbox = ProcessSandbox::new();

    let result = sandbox.execute("print('Hello from Python!')").await?;

    println!("Output: {}", result.stdout);
    println!("Exit code: {}", result.exit_code);

    Ok(())
}
```

## Backends

| Backend | Feature | Use Case |
|---------|---------|----------|
| `ProcessSandbox` | `process` | Development (default) |
| `DockerSandbox` | `docker` | Production (containers) |
| `GatewaySandbox` | `gateway` | Production (Firecracker microVMs) |
| `FirecrackerSandbox` | `firecracker` | Direct Firecracker access |

## Security

- `ProcessSandbox`: **Development only** - no isolation
- `DockerSandbox`: Container isolation
- `GatewaySandbox`: microVM isolation (recommended for production)

## License

MIT
