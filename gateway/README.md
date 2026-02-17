# Agentic Gateway

A high-performance Rust gateway providing unified access to LLM providers and Firecracker sandboxes.

## Features

- **OpenAI-compatible API** - Drop-in replacement for OpenAI endpoints
- **Multi-provider routing** - OpenAI, Anthropic, with automatic failover
- **Rate limiting** - Per-provider RPM, TPM, TPD limits
- **Firecracker sandboxes** - Sub-200ms code execution in microVMs
- **Sandbox pooling** - Pre-warmed VMs for instant allocation

## Quick Start

```bash
# Build
cargo build --release

# Configure
cp config.example.toml config.toml
# Edit config.toml with your API keys

# Run
OPENAI_API_KEY=sk-... ANTHROPIC_API_KEY=sk-ant-... cargo run
```

## API Endpoints

### LLM

```bash
# Chat completion (OpenAI-compatible)
curl -X POST http://localhost:8080/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello!"}]
  }'

# List models
curl http://localhost:8080/v1/models
```

### Sandbox

```bash
# Create sandbox
curl -X POST http://localhost:8080/v1/sandboxes \
  -H "Content-Type: application/json" \
  -d '{"runtime": "python"}'

# Execute code
curl -X POST http://localhost:8080/v1/sandboxes/{id}/execute \
  -H "Content-Type: application/json" \
  -d '{"code": "print(1 + 1)"}'

# Delete sandbox
curl -X DELETE http://localhost:8080/v1/sandboxes/{id}
```

### Health

```bash
# Health check
curl http://localhost:8080/health

# Prometheus metrics
curl http://localhost:8080/metrics
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                     Agentic Gateway                         │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐  │
│  │   Axum      │  │   Tower     │  │   Metrics           │  │
│  │   Router    │  │   Middleware│  │   Prometheus        │  │
│  └──────┬──────┘  └──────┬──────┘  └──────────┬──────────┘  │
│         │                │                     │            │
│  ┌──────┴────────────────┴─────────────────────┴──────────┐ │
│  │                    Request Handler                      │ │
│  └──────┬─────────────────────────────────────┬───────────┘ │
│         │                                     │             │
│  ┌──────▼──────┐                      ┌───────▼─────────┐   │
│  │ LLM Router  │                      │ Sandbox Pool    │   │
│  │ - Failover  │                      │ - Warm VMs      │   │
│  │ - Rate Limit│                      │ - Fast alloc    │   │
│  └──────┬──────┘                      └───────┬─────────┘   │
│         │                                     │             │
└─────────┼─────────────────────────────────────┼─────────────┘
          │                                     │
   ┌──────▼──────┐                      ┌───────▼─────────┐
   │  Providers  │                      │   Firecracker   │
   │ ┌─────────┐ │                      │   microVMs      │
   │ │ OpenAI  │ │                      │ ┌─────────────┐ │
   │ ├─────────┤ │                      │ │ Python      │ │
   │ │Anthropic│ │                      │ ├─────────────┤ │
   │ ├─────────┤ │                      │ │ Node.js     │ │
   │ │ Groq    │ │                      │ ├─────────────┤ │
   │ └─────────┘ │                      │ │ Rust        │ │
   └─────────────┘                      │ └─────────────┘ │
                                        └─────────────────┘
```

## Configuration

```toml
[server]
host = "0.0.0.0"
port = 8080

[llm.providers.openai]
api_key = "${OPENAI_API_KEY}"
enabled = true

[llm.providers.anthropic]
api_key = "${ANTHROPIC_API_KEY}"
enabled = true

[llm.models.gpt-4o]
provider = "openai"
fallbacks = ["anthropic"]

[sandbox]
firecracker_path = "/usr/bin/firecracker"
kernel_path = "/var/lib/firecracker/vmlinux"
rootfs_path = "/var/lib/firecracker/rootfs.ext4"

[sandbox.pool]
min_ready = 2
max_total = 10

[rate_limit]
enabled = true
requests_per_minute = 60
tokens_per_minute = 100000
```

## Crates

| Crate | Description |
|-------|-------------|
| `gateway-core` | Core types (LLM, Sandbox, Config, Error) |
| `gateway-llm` | Provider routing and rate limiting |
| `gateway-sandbox` | Firecracker management and pooling |
| `gateway-server` | HTTP API (Axum) |

## Performance Targets

| Metric | Target |
|--------|--------|
| P50 latency overhead | < 5ms |
| P99 latency overhead | < 15ms |
| Sandbox cold start | < 200ms |
| Sandbox warm start | < 50ms |
| Throughput | > 1000 req/s |

## License

MIT
