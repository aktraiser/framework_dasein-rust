//! The runtime's metrics, as a Prometheus scraper would see them.
#![cfg(feature = "prometheus")]

use async_trait::async_trait;
use dasein_agentic_core::distributed::graph::*;
use metrics_exporter_prometheus::PrometheusBuilder;

/// Yields its input, or fails when the input is "fail".
struct Step(ExecutorId);

#[async_trait]
impl Executor for Step {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        &self.0
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, input: String, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        if input == "fail" {
            return Err(ExecutorError::internal("asked to fail"));
        }
        ctx.yield_output(input).await
    }
}

fn workflow() -> Workflow<String, String> {
    let definition = WorkflowBuilder::<String>::new("metered")
        .set_start("only")
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    registry.register(Step("only".into()));
    Workflow::new(definition, registry)
}

#[test]
fn workflow_and_executor_metrics_are_recorded() {
    // A recorder local to this thread, so tests do not share a global one.
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    metrics::with_local_recorder(&recorder, || {
        runtime.block_on(async {
            let flow = workflow();
            assert!(flow.run("ok".to_string()).await.unwrap().success);
            assert!(flow.run("ok".to_string()).await.unwrap().success);
            assert!(!flow.run("fail".to_string()).await.unwrap().success);
        });
    });

    let metrics = handle.render();
    for expected in [
        r#"dasein_workflow_runs_total{workflow="metered",outcome="success"} 2"#,
        r#"dasein_workflow_runs_total{workflow="metered",outcome="error"} 1"#,
        r#"dasein_executor_invocations_total{executor="only",outcome="success"} 2"#,
        r#"dasein_executor_invocations_total{executor="only",outcome="error"} 1"#,
        "dasein_workflow_duration_seconds",
        "dasein_executor_duration_seconds",
        "dasein_workflow_supersteps",
    ] {
        assert!(
            metrics.contains(expected),
            "missing `{expected}` in:\n{metrics}"
        );
    }
}

#[cfg(feature = "otel")]
#[test]
fn otlp_layer_can_be_added_to_a_subscriber() {
    use tracing_subscriber::prelude::*;
    // Nothing listens on this port: building the pipeline must not need a
    // collector, and dropping the guard must not hang.
    let (layer, guard) =
        dasein_agentic_core::telemetry::otlp_layer("test-service", "http://127.0.0.1:1/").unwrap();
    let subscriber = tracing_subscriber::registry().with(layer);
    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("probe");
        let _entered = span.enter();
    });
    drop(guard);
}
