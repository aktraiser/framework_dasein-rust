# Qualified local beta profile

The local beta profile consists of the graph runtime, GroupChat, a durable NATS queue, ephemeral Docker execution, and a single-process gateway with Python Boxes and OpenAI Chat Completions streaming/embeddings. It is intended for controlled laboratory use. The SDK also has a [controlled Firecracker beta](../server/firecracker-guest/README.md), tested with real nested KVM on Apple M4; its authenticated HTTP adapter is separate from the Docker Boxes gateway. The legacy gateway warm-VM pool remains disabled. No release has been published by these local checks.

## Runtime and state

- Executors run concurrently with a configurable bound (default 16). Retries respect `ExecutorError::retriable`; external side effects must be idempotent.
- GroupChat executes the selected registered participant, records its responses, checks termination and enforces the iteration limit. Each participant receives the latest response and can read the full history with `ctx.get_shared_state::<GroupChatState>("group_chat")`. The virtual orchestrator requires no executor implementation. Participant output/message JSON must be compatible with the next participant's input.
- `run_with_resume` checkpoints pending work, outputs, retries, conversation state and shared state at the configured interval (default 5). Set interval 1 for recovery after every completed step. A crash can replay work since the last checkpoint.
- Shared state is scoped by **workflow ID and task ID**. Different tasks cannot accidentally read or restore one another's state. These IDs are application namespaces, not authentication boundaries. Do not run the same task ID concurrently in multiple workflow processes.
- Migration: old workflow-only shared-state keys need explicit migration. New namespace: `len(workflow):workflowlen(task):task:`. Existing old checkpoints are not automatically migrated. Start new task IDs or migrate in a copy before resuming an old checkpoint.
- `run_stream` emits final graph status; it does not stream individual executor events. This is separate from the gateway's LLM SSE stream.
- ChatAgent executable tools retain bounded rounds, tool deadlines and correlation IDs. External tools must enforce their own authorization and cancellation.

## Durable queue

`DurableQueue`, also available through `BusCoordinator::durable_queue`, uses a file-backed JetStream work stream and a result ledger. Publication waits for broker acknowledgement. Workers share one durable consumer per supervisor, use ACK/NAK/progress heartbeats, and redeliver abandoned work. Completion is persisted before ACK. Task IDs are reserved; conflicting payloads with the same ID are rejected.

Delivery is **at least once**, not exactly once. A lease can expire while an old worker still runs; external side effects require idempotency keyed by task ID. Call `heartbeat()` during long work. The first persisted terminal outcome wins.

Dependencies are checked against durable outcomes. Failed dependencies fail the child; missing dependencies and cycles stay pending. Scheduling is FIFO among eligible deliveries, not priority scheduling. Queue settings must agree across clients. A failed publish can leave a pending reservation: retry the same ID and payload. Do not infer acceptance from a pending ledger entry alone.

Each queue and result ledger is bounded at 512 MiB by default (configurable); pending task count is configurable. Completed records have no automatic retention purge: archive/rotate a completed namespace before its ledger fills. A single NATS node does not provide high availability or protection from disk loss. Configure broker authentication/authorization and storage backups for use beyond a local lab.

Legacy `Sequencer` remains an in-memory priority scheduler. Legacy coordinator Core NATS subscriptions are live observers, not durable workers. Its task publication now waits for the JetStream ACK and no longer silently drops distinct task IDs with identical payloads. Use the durable API for recovery and dependencies.

## Docker and Boxes

The operator installs `python:3.12-alpine` (or sets `DASEIN_BOX_IMAGE` to a compatible prebuilt Python image). The SDK never downloads dependencies at execution time. Containers run as UID/GID 65534 with no network, no host mounts, read-only root, dropped capabilities, no-new-privileges, a PID cap and memory/CPU/output limits. Each execution has a fresh tmpfs workspace. Docker shares a Linux kernel and is not a VM security boundary.

Timeouts, nonzero exits, separate stdout/stderr, output overflow and cancellation cleanup are tested. `DockerSandbox::reap_expired()` recovers labelled containers left by a process crash. The gateway runs it every 15 seconds. SDK embedders must schedule it themselves. Cleanup requires a reachable Docker daemon; if the host application dies, containers can remain until the reaper runs again.

A Box is a persistent ownership/policy record, **not a persistent filesystem**. The Python gateway supports create/get/execute/delete with per-key ownership stored in SQLite. Workspaces are discarded after every execution; store long-lived agent memory separately. Limits: 1 execution per Box, 16 per gateway process, 1000 Box records, 32–512 MiB, 1–120 seconds, 256 KiB each for code/stdin, 1 MiB output. Excess concurrency returns 429. Requested timeouts cannot exceed the Box policy. Package installation and other runtimes return explicit errors.

This manager is qualified for one gateway process. Do not share its SQLite database across independently running gateways: distributed execution leases and cancellation are not implemented.

## Gateway and LLM

`GATEWAY_CONFIG` loads an explicit configuration file; otherwise environment/dashboard configuration is used. The default environment-only listener is localhost. `/health` is public; other API routes require a bearer token. `GATEWAY_API_KEY` is the operator token. Enabled database API keys may access LLM routes and their own Boxes. Shared files, VM administration and legacy direct execution remain operator-only.

Configured OpenAI models support ordinary chat, embeddings, and actual SSE streaming. UTF-8 split across chunks is preserved; a truncated stream emits an error rather than a fabricated completion. Stream requests have a 120-second overall timeout and a 1 MiB event limit. Streams use the configured primary provider and are never automatically replayed after opening. Anthropic ordinary chat remains available; its gateway streaming is still unavailable. Anthropic has no embeddings API. Other routing strategies are explicitly unsupported. Live model accounts, quotas and model-specific compatibility have not been tested; integration uses local mock providers.

Persisting provider keys requires `GATEWAY_MASTER_KEY`, base64-encoded random 32 bytes, kept outside SQLite. AES-256-GCM authenticates the provider ID. Existing plaintext credentials must be re-entered; key rotation requires a credential migration. A global LLM spending limit and production rate-limit enforcement are not part of this qualification.

## Remaining release gates

Firecracker jailer/cgroup integration, other host architectures and production-host qualification; multi-process gateway leases; persistent Box filesystem if required; additional runtimes; live provider qualification; sustained load and resource exhaustion; Qdrant integration; external security review and CI execution on Linux. ProcessSandbox is for trusted development code and has no security isolation. No Rust/Python comparative performance claim follows from these tests.

See [local launch](BETA_LOCAL.md). Run `bash scripts/test-services.sh` only against disposable NATS/Redis with Docker and the Python image installed. Tests create/delete broker streams. Ignored documentation examples are pseudocode and must not be force-run.

API references: [OpenAI tool calling](https://developers.openai.com/api/docs/guides/function-calling), [OpenAI streaming](https://developers.openai.com/api/docs/guides/streaming-responses).
