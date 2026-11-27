//! Metrics and trace export.
//!
//! The runtime records metrics through the [`metrics`] facade at the points
//! that matter when operating agents: workflow runs, executor invocations,
//! model calls and tool calls. Recording costs next to nothing until a
//! recorder is installed, so nothing is exported unless the application asks
//! for it:
//!
//! - with the `prometheus` feature, [`install_prometheus`] serves the
//!   metrics over HTTP for a Prometheus scraper, and
//!   [`prometheus_recorder`] returns a handle to render them from the
//!   application's own endpoint;
//! - any other recorder for the `metrics` crate works as well.
//!
//! The runtime's `tracing` spans can be sent to an OpenTelemetry collector
//! with the `otel` feature: see [`otlp_layer`].
//!
//! # Metrics
//!
//! | Name | Kind | Labels |
//! |------|------|--------|
//! | `dasein_workflow_runs_total` | counter | `workflow`, `outcome` |
//! | `dasein_workflow_duration_seconds` | histogram | `workflow`, `outcome` |
//! | `dasein_workflow_supersteps` | histogram | `workflow` |
//! | `dasein_executor_invocations_total` | counter | `executor`, `outcome` |
//! | `dasein_executor_duration_seconds` | histogram | `executor` |
//! | `dasein_llm_requests_total` | counter | `provider`, `model`, `outcome` |
//! | `dasein_llm_request_duration_seconds` | histogram | `provider`, `model` |
//! | `dasein_llm_tokens_total` | counter | `provider`, `model`, `kind` |
//! | `dasein_tool_calls_total` | counter | `tool`, `outcome` |
//! | `dasein_tool_call_duration_seconds` | histogram | `tool` |
//!
//! `outcome` is `success`, `error` or `timeout`; `kind` is `prompt` or
//! `completion`. Labels are names fixed by the application (workflow,
//! executor, tool, model), never per-task identifiers.

use std::time::Instant;

use metrics::{counter, histogram};

/// Outcome label of a recorded operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Outcome {
    Success,
    Error,
    Timeout,
}

impl Outcome {
    const fn label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
            Self::Timeout => "timeout",
        }
    }
}

/// Record a finished workflow run. `supersteps` is absent when the run
/// failed before producing a result.
pub(crate) fn workflow_run(
    workflow: &str,
    started: Instant,
    success: bool,
    supersteps: Option<u32>,
) {
    let outcome = if success {
        Outcome::Success
    } else {
        Outcome::Error
    }
    .label();
    let workflow = workflow.to_owned();
    counter!("dasein_workflow_runs_total", "workflow" => workflow.clone(), "outcome" => outcome)
        .increment(1);
    histogram!("dasein_workflow_duration_seconds", "workflow" => workflow.clone(), "outcome" => outcome)
        .record(started.elapsed().as_secs_f64());
    if let Some(supersteps) = supersteps {
        histogram!("dasein_workflow_supersteps", "workflow" => workflow)
            .record(f64::from(supersteps));
    }
}

/// Record one invocation of an executor (each retry is an invocation).
pub(crate) fn executor_invocation(executor: &str, started: Instant, outcome: Outcome) {
    let executor = executor.to_owned();
    counter!("dasein_executor_invocations_total", "executor" => executor.clone(), "outcome" => outcome.label())
        .increment(1);
    histogram!("dasein_executor_duration_seconds", "executor" => executor)
        .record(started.elapsed().as_secs_f64());
}

/// Record a model call and, when it succeeded, the tokens it used.
pub(crate) fn llm_request(
    provider: &str,
    model: &str,
    started: Instant,
    usage: Option<&dasein_agentic_llm::TokenUsage>,
) {
    let (provider, model) = (provider.to_owned(), model.to_owned());
    let outcome = if usage.is_some() {
        Outcome::Success
    } else {
        Outcome::Error
    }
    .label();
    counter!("dasein_llm_requests_total", "provider" => provider.clone(), "model" => model.clone(), "outcome" => outcome)
        .increment(1);
    histogram!("dasein_llm_request_duration_seconds", "provider" => provider.clone(), "model" => model.clone())
        .record(started.elapsed().as_secs_f64());
    if let Some(usage) = usage {
        counter!("dasein_llm_tokens_total", "provider" => provider.clone(), "model" => model.clone(), "kind" => "prompt")
            .increment(u64::from(usage.prompt));
        counter!("dasein_llm_tokens_total", "provider" => provider, "model" => model, "kind" => "completion")
            .increment(u64::from(usage.completion));
    }
}

/// Record a tool call made by an agent.
pub(crate) fn tool_call(tool: &str, started: Instant, outcome: Outcome) {
    let tool = tool.to_owned();
    counter!("dasein_tool_calls_total", "tool" => tool.clone(), "outcome" => outcome.label())
        .increment(1);
    histogram!("dasein_tool_call_duration_seconds", "tool" => tool)
        .record(started.elapsed().as_secs_f64());
}

/// Failure to set up an exporter.
#[derive(Debug, thiserror::Error)]
#[error("telemetry setup failed: {0}")]
pub struct TelemetryError(String);

/// Serve the metrics in the Prometheus text format at `http://{address}/metrics`.
///
/// Call once, from inside a Tokio runtime, before the work to observe starts.
///
/// # Errors
/// Fails if a metrics recorder is already installed or the address cannot be
/// bound.
#[cfg(feature = "prometheus")]
pub fn install_prometheus(address: std::net::SocketAddr) -> Result<(), TelemetryError> {
    metrics_exporter_prometheus::PrometheusBuilder::new()
        .with_http_listener(address)
        .install()
        .map_err(|e| TelemetryError(e.to_string()))
}

/// Install a Prometheus recorder without a listener and return its handle.
/// `handle.render()` gives the text to serve from the application's own
/// `/metrics` endpoint.
///
/// # Errors
/// Fails if a metrics recorder is already installed.
#[cfg(feature = "prometheus")]
pub fn prometheus_recorder() -> Result<metrics_exporter_prometheus::PrometheusHandle, TelemetryError>
{
    metrics_exporter_prometheus::PrometheusBuilder::new()
        .install_recorder()
        .map_err(|e| TelemetryError(e.to_string()))
}

/// Keeps the OpenTelemetry pipeline alive; dropping it flushes the spans
/// still buffered and stops the export.
#[cfg(feature = "otel")]
pub struct OtelGuard {
    provider: opentelemetry_sdk::trace::SdkTracerProvider,
}

#[cfg(feature = "otel")]
impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Err(error) = self.provider.shutdown() {
            tracing::debug!(%error, "OpenTelemetry shutdown reported an error");
        }
    }
}

/// Build a `tracing` layer that exports the runtime's spans to an
/// OpenTelemetry collector over OTLP/HTTP.
///
/// `endpoint` is the collector's base URL, for example
/// `http://localhost:4318`. Add the layer to the application's subscriber and
/// keep the guard for as long as spans should be exported:
///
/// ```rust,ignore
/// use tracing_subscriber::prelude::*;
///
/// let (layer, _guard) = dasein_agentic_core::telemetry::otlp_layer(
///     "my-agents",
///     "http://localhost:4318",
/// )?;
/// tracing_subscriber::registry()
///     .with(layer)
///     .with(tracing_subscriber::fmt::layer())
///     .init();
/// ```
///
/// # Errors
/// Fails if the exporter cannot be built from the endpoint.
#[cfg(feature = "otel")]
pub fn otlp_layer<S>(
    service_name: &str,
    endpoint: &str,
) -> Result<
    (
        tracing_opentelemetry::OpenTelemetryLayer<S, opentelemetry_sdk::trace::Tracer>,
        OtelGuard,
    ),
    TelemetryError,
>
where
    S: tracing::Subscriber + for<'span> tracing_subscriber::registry::LookupSpan<'span>,
{
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_otlp::WithExportConfig as _;

    let exporter = opentelemetry_otlp::SpanExporter::builder()
        .with_http()
        .with_endpoint(format!("{}/v1/traces", endpoint.trim_end_matches('/')))
        .build()
        .map_err(|e| TelemetryError(e.to_string()))?;
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(
            opentelemetry_sdk::Resource::builder()
                .with_service_name(service_name.to_owned())
                .build(),
        )
        .build();
    let layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("dasein-agentic-core"));
    Ok((layer, OtelGuard { provider }))
}
