# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- A live test of an agent answering from an Obsidian vault through a third-party MCP server, with a small vault as fixture.

### Fixed
- Gemini: tool parameters are sent as JSON Schema (`parametersJsonSchema`). Tools described with keywords the older field refused, as many MCP servers do (`$ref`, `exclusiveMinimum`, `additionalProperties`, `$schema`, a numeric `enum`...), no longer make every request fail.

## [0.2.0] - 2026-11-27

Not published on crates.io: depend on the repository at the tag `v0.2.0`.

### Security
- The legacy `Agent` refuses to run model-generated code on a sandbox without isolation unless the host opts in; `ProcessSandbox` no longer passes secret-looking environment variables to executed code.
- Gateway: the remote shell endpoint `POST /v1/vm/exec` is removed; the SSH host key is pinned; the dashboard is protected against cross-site requests and login brute force.
- The Gemini API key is sent in a header instead of the URL.
- `gmail_send` only sends to an allow-list of recipients.
- Dependencies with known advisories are updated.

### Added
- Remote steps: `RemoteExecutor` runs a workflow step on `RemoteWorker`s in other processes, over a `DurableQueue` on NATS JetStream or an `InMemoryTaskQueue`; `NatsStateBackend` shares workflow state between them. See `docs/REMOTE_EXECUTORS.md` and the `remote_steps` example.
- `ChatAgent::with_mcp_tools` (feature `mcp`): an agent takes its tools from MCP servers.
- Metrics for workflows, executors, model and tool calls, served to Prometheus (feature `prometheus`) or exported as OpenTelemetry spans (feature `otel`). See `docs/OBSERVABILITY.md`.
- Tests against the real provider APIs, off by default: `scripts/live-tests.sh` and `docs/LIVE_TESTS.md`.
- `soak` example: a load and duration test of workflows with a remote step, in memory or through NATS.
- `DurableQueueConfig::result_ttl`, `InMemoryStateBackend::forget`, `InMemoryTaskQueue::remembering`.
- MCP client: stdio transport, `initialize` handshake, sessions, reconnection, all content types.
- Graph: `add_fan_in_edge_with` merges joined messages so the target runs once.
- Gateway: per-key request and token limits are enforced; usage, cost and streamed tokens are logged; `trusted_proxies`.
- LLM adapters: `with_base_url`, `with_timeout`, `with_max_retries` on Anthropic, Gemini and Ollama.
- Firecracker sandbox: `FirecrackerConfig::jailer` runs each microVM through the jailer, unprivileged, chrooted and in a cgroup with CPU, memory and process limits; `JAILER_BIN` turns it on in the HTTP adapter.
- Gateway: several processes can serve from one SQLite file on a machine. Box executions, the box limit and failed dashboard logins are coordinated through the database, which is opened in WAL mode.

### Changed
- The record of a remote task, with its result, is kept for `result_ttl` instead of for ever: a retry or resume that sends the same task again after that runs it again.
- The legacy `Agent`, the sequencer and the crates they superseded are deprecated in favour of the graph runtime.
- A fan-in edge now waits for its sources and delivers their messages together.
- `WorkflowAgent::run` returns an error when the workflow fails.
- Anthropic and Gemini report `FinishReason::ToolCalls` when the model asks for a tool.
- The gateway listens on `127.0.0.1` when `host` is omitted.
- A stdio MCP server no longer inherits the environment of the host process.
- The Rust toolchain is pinned in `rust-toolchain.toml`.

### Fixed
- Gemini: the thought signature of a tool call is sent back with it, which models that sign their tool calls require on the next round.
- Gateway: a request reserves its expected tokens when it arrives, so requests running in parallel count against each other.
- Gateway: processes started together on a new database no longer fail with "database is locked".
- Remote steps over NATS no longer fail with "maximum consumers limit reached" under load: waits share one subscription instead of creating one each.
- The result ledger of a `DurableQueue` no longer fills up for good: records expire after `result_ttl` (one hour by default), and a full ledger is reported as such.
- A workflow no longer keeps the shared state of every run it ever made in memory, and `InMemoryTaskQueue` remembers the last 10 000 results instead of all of them.
- A remote worker no longer spends a millisecond per task on its heartbeat timer.
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

[Unreleased]: https://github.com/aktraiser/framework_dasein-rust/compare/v0.2.0...main
[0.2.0]: https://github.com/aktraiser/framework_dasein-rust/releases/tag/v0.2.0
[0.1.0]: https://crates.io/crates/dasein-agentic-core/0.1.0
