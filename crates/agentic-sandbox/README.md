# dasein-agentic-sandbox

See the [qualified local beta profile](../../docs/SDK_STATUS.md) and [launch guide](../../docs/BETA_LOCAL.md).

Sandboxed code execution for the Agentic Framework.

## Features

- **Multiple Backends**: Process, Docker, Firecracker, Gateway
- **Language Support**: Python, Node.js, Rust, Go, Bash
- **Isolation**: Secure code execution in isolated environments
- **Async-first**: Built on Tokio

## Installation

```toml
[dependencies]
dasein-agentic-sandbox = "0.1"

# With Docker support
dasein-agentic-sandbox = { version = "0.1", features = ["docker"] }

# With Gateway (Python Docker Boxes)
dasein-agentic-sandbox = { version = "0.1", features = ["gateway"] }
```

## Quick Start

```rust
use dasein_agentic_sandbox::{ProcessSandbox, Sandbox};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sandbox = ProcessSandbox::new();

    let result = sandbox.execute("python3 -c \"print(42)\"").await?;

    println!("Output: {}", result.stdout);
    println!("Exit code: {}", result.exit_code);

    Ok(())
}
```

## Backends

| Backend | Feature | Use Case |
|---------|---------|----------|
| `ProcessSandbox` | `process` | Development (default) |
| `DockerSandbox` | `docker` | Controlled Docker laboratory |
| `GatewaySandbox` | `gateway` | Python Docker Boxes via gateway |
| `FirecrackerSandbox` | `firecracker` | Real microVM per execution; [controlled Linux/KVM beta](../../server/firecracker-guest/README.md) |

## Security

- `ProcessSandbox`: **Development only** - no isolation
- `DockerSandbox`: Container isolation
- `GatewaySandbox`: Backend-dependent; local beta uses ephemeral Docker containers

## License

MIT
