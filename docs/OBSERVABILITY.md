# Observability

The runtime reports what it does in three ways. None of them needs any setup to keep working; export is opt-in.

## Following one run

Metrics and traces say how the system does overall. To see what one run does, as it does it, listen to its events:

```rust
use futures::StreamExt;

let mut events = workflow.run_stream(input);
while let Some(event) = events.next().await {
    println!("{event:?}");
}
```

The stream carries, in order: the run starting, each superstep and executor starting and ending (with durations and errors), outputs, what executors add with `add_event`, checkpoints, every model call of an agent (provider, model, duration, tokens, how many tools it asked for) and every tool call (name, arguments, duration, the beginning of the result, whether it succeeded). The last event is `Completed` or `Failed`.

Steps of a superstep may run side by side, so their events interleave: an agent's events name the executor and the input they belong to, and the agent. A workflow run inside a step reports to the same listener; `Started`, `Completed` and `Failed` carry the workflow id, and the run you listen to is over at the last event, the one with its own id.

To get the result as well, or to follow an agent that runs outside a workflow, give a channel to `with_events`:

```rust
let (listener, mut events) = tokio::sync::mpsc::unbounded_channel();
let answer = with_events(listener, agent.run(messages, &mut thread)).await?;
```

Things to know:

- **Nothing is built when nobody listens.** A run outside `run_stream` and `with_events` reports nothing and pays nothing for it.
- **Remote steps report late, and only if their worker is asked to.** Start the worker with `RemoteWorker::with_event_reporting(true)`: what its executors do then comes back with each step's result, and appears together, between the step's start and its end, not as it happens, and only if the step succeeded. It is off by default because the events are stored with every result, whether or not anyone listens. They get the room the result leaves under the worker's size limit, 200 kB at most; the first ones are kept.
- **Arguments and results are in the events.** A tool call's arguments are reported whole and its result up to 2 000 characters: treat the events like the data they describe.
- **Same task only.** Work an executor moves to another task with `tokio::spawn` is not followed.

## Metrics

`dasein-agentic-core` records metrics through the [`metrics`](https://docs.rs/metrics) facade. Recording costs next to nothing until the application installs a recorder.

| Name | Kind | Labels |
|------|------|--------|
| `dasein_workflow_runs_total` | counter | `workflow`, `outcome` |
| `dasein_workflow_duration_seconds` | histogram | `workflow`, `outcome` |
| `dasein_workflow_supersteps` | histogram | `workflow` |
| `dasein_executor_invocations_total` | counter | `executor`, `outcome` |
| `dasein_executor_duration_seconds` | histogram | `executor` |
| `dasein_llm_requests_total` | counter | `provider`, `model`, `outcome` |
| `dasein_llm_request_duration_seconds` | histogram | `provider`, `model` |
| `dasein_llm_tokens_total` | counter | `provider`, `model`, `kind` |
| `dasein_tool_calls_total` | counter | `tool`, `outcome` |
| `dasein_tool_call_duration_seconds` | histogram | `tool` |

`outcome` is `success`, `error` or `timeout`; `kind` is `prompt` or `completion`. Labels are names the application chose (workflow, executor, tool, model), never per-task identifiers, so their number stays bounded.

Model and tool metrics come from `ChatAgent`. Calling an LLM adapter directly, outside an agent, is not counted.

### Prometheus

With the `prometheus` feature:

```toml
dasein-agentic-core = { git = "https://github.com/aktraiser/framework_dasein-rust", features = ["prometheus"] }
```

```rust
// Serves http://127.0.0.1:9000/metrics. Call once, inside a Tokio runtime.
dasein_agentic_core::telemetry::install_prometheus("127.0.0.1:9000".parse()?)?;
```

To serve the metrics from your own HTTP server instead, use `prometheus_recorder()` and return `handle.render()` from your endpoint.

Any other recorder for the `metrics` crate works too.

## Traces

The runtime's functions are instrumented with `tracing` spans. With the `otel` feature they can be sent to an OpenTelemetry collector over OTLP/HTTP:

```rust
use tracing_subscriber::prelude::*;

let (layer, _guard) = dasein_agentic_core::telemetry::otlp_layer(
    "my-agents",              // service name
    "http://localhost:4318",  // collector base URL
)?;
tracing_subscriber::registry()
    .with(layer)
    .with(tracing_subscriber::fmt::layer())
    .init();
// Keep `_guard` alive: dropping it flushes the buffered spans and stops the export.
```

The layer is returned rather than installed, because the global subscriber belongs to the application.

Only spans are exported. Metrics go through Prometheus as described above; logs are not exported.

## Audit events

Independently of the above, pipelines publish audit events on NATS; see the audit section of [ARCHITECTURE.md](ARCHITECTURE.md).

## The gateway

The gateway has its own `/metrics` endpoint and usage log; see [gateway/README.md](../gateway/README.md).
