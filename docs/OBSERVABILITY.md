# Observability

The runtime reports what it does in three ways. None of them needs any setup to keep working; export is opt-in.

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
