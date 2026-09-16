# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Agent Lab runtime integration — 2026-09-16
- Durable ChatAgent checkpoints and bounded tool execution, preserving provider tool-call state across resume.
- Immutable JetStream forum, verification task lifecycle, behavior traces and specialist claim types.
- Responses adapter and provider usage details, including cached input and reasoning subdivisions.
- Provider tool-history compatibility fixes and isolated temporary Rust validation package identities.
- `TokenUsage` now includes optional `details`; downstream Rust struct literals should set it to `None` when unavailable.


### Added
- Initial public release
- Multi-LLM support: OpenAI, Anthropic, Gemini, Ollama
- Sandboxed code execution (Process, Remote Gateway)
- NATS-based distributed message bus
- Graph-based workflow engine
- MCP protocol client
- Incremental code generation pipeline
- Validation and linting system

### Infrastructure
- GitHub Actions CI/CD pipeline
- VitePress documentation site
- Comprehensive test suite

## [0.1.0] - 2024-XX-XX

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

[Unreleased]: https://github.com/aktraiser/framework_dasein-rust/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/aktraiser/framework_dasein-rust/releases/tag/v0.1.0
