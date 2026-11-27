# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security
- The legacy `Agent` refuses to run model-generated code on a sandbox without isolation unless the host opts in; `ProcessSandbox` no longer passes secret-looking environment variables to executed code.
- Gateway: the remote shell endpoint `POST /v1/vm/exec` is removed; the SSH host key is pinned; the dashboard is protected against cross-site requests and login brute force.
- The Gemini API key is sent in a header instead of the URL.
- `gmail_send` only sends to an allow-list of recipients.
- Dependencies with known advisories are updated.

### Added
- MCP client: stdio transport, `initialize` handshake, sessions, reconnection, all content types.
- Graph: `add_fan_in_edge_with` merges joined messages so the target runs once.
- Gateway: per-key request and token limits are enforced; usage, cost and streamed tokens are logged; `trusted_proxies`.
- LLM adapters: `with_base_url`, `with_timeout`, `with_max_retries` on Anthropic, Gemini and Ollama.
- Gateway: several processes can serve from one SQLite file on a machine. Box executions, the box limit and failed dashboard logins are coordinated through the database, which is opened in WAL mode.

### Changed
- A fan-in edge now waits for its sources and delivers their messages together.
- `WorkflowAgent::run` returns an error when the workflow fails.
- Anthropic and Gemini report `FinishReason::ToolCalls` when the model asks for a tool.
- The gateway listens on `127.0.0.1` when `host` is omitted.
- A stdio MCP server no longer inherits the environment of the host process.
- The Rust toolchain is pinned in `rust-toolchain.toml`.

### Fixed
- Streams from Gemini, Anthropic and Ollama no longer lose or corrupt text split across network chunks.
- A durable `ChatAgent` run can resume after a crash during a model call.
- Text previews no longer panic on non-ASCII input.
- MCP `isError` is read: a failed tool call is no longer reported as a success.

## [0.1.0] - 2026-02-16

### Added
- `agentic-core`: Core agent runtime with executor trait
- `agentic-llm`: LLM adapters for OpenAI, Anthropic, Gemini, Ollama
- `agentic-sandbox`: Process and Docker sandbox execution
- `agentic-bus`: NATS message bus integration
- `agentic-storage`: Redis state + Qdrant vector storage
- `agentic-mcp`: Model Context Protocol client
- `agentic-orchestrator`: Multi-agent workflow orchestration

### Examples
- `chat`: Interactive CLI chat with any LLM
- `single_agent`: Basic agent with sandbox
- `graph_workflow`: DAG-based workflow execution
- `validator_pipeline`: Code validation with retries

---

[Unreleased]: https://github.com/aktraiser/framework_dasein-rust/commits/main
[0.1.0]: https://crates.io/crates/dasein-agentic-core/0.1.0
