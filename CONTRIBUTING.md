# Contributing to Dasein Agentic SDK

First off, thanks for taking the time to contribute!

## Getting Started

### Prerequisites

- Rust 1.75+ (`rustup update stable`)
- Docker (optional, for sandbox features)
- NATS (optional, for distributed mode)

### Setup

```bash
# Clone the repo
git clone https://github.com/aktraiser/framework_dasein-rust.git
cd framework_dasein-rust

# Build everything
cargo build --workspace

# Run tests
cargo test --workspace

# Check formatting
cargo fmt --all -- --check

# Run clippy
cargo clippy --workspace --all-features -- -D warnings
```

### Environment Variables

For running examples with real LLMs:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
export OPENAI_API_KEY="sk-..."
export GEMINI_API_KEY="..."
```

## Development Workflow

### 1. Find an Issue

- Look for issues labeled [`good first issue`](https://github.com/aktraiser/framework_dasein-rust/labels/good%20first%20issue)
- Comment on the issue to let others know you're working on it

### 2. Create a Branch

```bash
git checkout -b feature/your-feature-name
# or
git checkout -b fix/your-bug-fix
```

### 3. Make Your Changes

- Follow the existing code style
- Add tests for new functionality
- Update documentation if needed

### 4. Test Your Changes

```bash
# Run all tests
cargo test --workspace

# Run a specific test
cargo test test_name

# Run tests with output
cargo test -- --nocapture
```

### 5. Submit a Pull Request

- Fill out the PR template
- Link the related issue
- Wait for CI to pass

## Code Style

We use `rustfmt` with the config in `.rustfmt.toml`:

```bash
# Format code
cargo fmt --all

# Check formatting
cargo fmt --all -- --check
```

Key conventions:
- Max line width: 100 characters
- 4 spaces indentation
- Unix line endings

## Project Structure

```
crates/
├── agentic-core/       # Core agent runtime, types, protocols
├── agentic-llm/        # LLM adapters (OpenAI, Anthropic, etc.)
├── agentic-sandbox/    # Code execution (Process, Docker)
├── agentic-bus/        # NATS message bus
├── agentic-storage/    # Redis + Qdrant storage
├── agentic-mcp/        # MCP protocol client
└── agentic-orchestrator/  # Multi-agent workflows

examples/               # Example applications
docs/                   # VitePress documentation
```

## Adding a New LLM Adapter

1. Create `crates/agentic-llm/src/your_provider.rs`
2. Implement the `LLMAdapter` trait
3. Add feature flag in `Cargo.toml`:
   ```toml
   [features]
   your_provider = ["dep:reqwest"]
   ```
4. Export in `lib.rs`:
   ```rust
   #[cfg(feature = "your_provider")]
   mod your_provider;

   #[cfg(feature = "your_provider")]
   pub use your_provider::YourProviderAdapter;
   ```
5. Add tests
6. Update README table

## Adding an Example

1. Create `examples/your_example.rs`
2. Add to `Cargo.toml`:
   ```toml
   [[example]]
   name = "your_example"
   required-features = ["..."]
   ```
3. Add to `docs/examples/index.md`

## Commit Messages

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
feat: add Mistral LLM adapter
fix: handle timeout in Docker sandbox
docs: update README with new providers
refactor: simplify message parsing
test: add integration tests for workflows
chore: update dependencies
```

## Questions?

- Open an issue for bugs or feature requests
- Start a discussion for questions

Thanks for contributing!
