//! Remote workflow steps and shared state over a real NATS JetStream.
use async_trait::async_trait;
use dasein_agentic_core::distributed::bus::{
    DurableQueue, DurableQueueConfig, NatsClient, NatsConfig,
};
use dasein_agentic_core::distributed::graph::*;
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

async fn nats() -> Arc<NatsClient> {
    Arc::new(
        NatsClient::connect(NatsConfig::new(
            std::env::var("DASEIN_TEST_NATS_URL")
                .unwrap_or_else(|_| "nats://127.0.0.1:4222".into()),
        ))
        .await
        .unwrap(),
    )
}

fn unique() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn nats_state_is_shared_snapshotted_and_restored() {
    let name = unique();
    let writer = NatsStateBackend::open(nats().await, &name).await.unwrap();
    // A second handle, as another process would open it.
    let reader = NatsStateBackend::open(nats().await, &name).await.unwrap();

    writer.set("run1:note", json!("héllo")).await.unwrap();
    writer.set("run1:count", json!(3)).await.unwrap();
    writer.set("run2:note", json!("other")).await.unwrap();
    assert_eq!(reader.get("run1:note").await.unwrap(), Some(json!("héllo")));
    assert_eq!(reader.get("missing").await.unwrap(), None);

    // A snapshot holds one namespace only.
    let snapshot = reader.snapshot("run1:").await.unwrap();
    assert_eq!(
        snapshot,
        HashMap::from([
            ("run1:note".to_string(), json!("héllo")),
            ("run1:count".to_string(), json!(3)),
        ])
    );

    // Restoring replaces that namespace and leaves the others alone.
    writer.set("run1:count", json!(99)).await.unwrap();
    writer.set("run1:extra", json!(true)).await.unwrap();
    reader.restore("run1:", &snapshot).await.unwrap();
    assert_eq!(reader.snapshot("run1:").await.unwrap(), snapshot);
    assert_eq!(reader.get("run2:note").await.unwrap(), Some(json!("other")));

    writer.delete("run2:note").await.unwrap();
    assert_eq!(reader.get("run2:note").await.unwrap(), None);
}

/// Hosted by the workers: upper-cases its input and adds the shared note.
struct Shout(Arc<AtomicUsize>);

#[async_trait]
impl Executor for Shout {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        static ID: std::sync::OnceLock<ExecutorId> = std::sync::OnceLock::new();
        ID.get_or_init(|| "shout".into())
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, input: String, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        self.0.fetch_add(1, Ordering::SeqCst);
        let note: Option<String> = ctx.get_shared_state("note").await?;
        ctx.yield_output(format!(
            "{}{}",
            input.to_uppercase(),
            note.unwrap_or_default()
        ))
        .await
    }
}

/// Local first step: leaves a note in shared state and passes on.
struct Start;

#[async_trait]
impl Executor for Start {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        static ID: std::sync::OnceLock<ExecutorId> = std::sync::OnceLock::new();
        ID.get_or_init(|| "start".into())
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, input: String, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        ctx.set_shared_state("note", "!").await?;
        ctx.send_message(input).await
    }
}

#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn workflow_steps_run_on_workers_over_the_durable_queue() {
    let name = unique();
    let open_queue = || async {
        DurableQueue::new(
            nats().await,
            &name,
            DurableQueueConfig {
                max_queue_bytes: 4 * 1024 * 1024,
                max_result_bytes: 4 * 1024 * 1024,
                ack_wait: Duration::from_secs(5),
                ..Default::default()
            },
        )
        .await
        .unwrap()
    };
    let runs = Arc::new(AtomicUsize::new(0));

    // Two workers, each with its own connection, queue handle and state
    // handle, as two machines would have.
    let mut stops = Vec::new();
    for _ in 0..2 {
        let queue = open_queue().await;
        let state = Arc::new(NatsStateBackend::open(nats().await, &name).await.unwrap());
        let mut registry = ExecutorRegistry::new();
        registry.register(Shout(runs.clone()));
        let worker = RemoteWorker::new(registry)
            .with_state_backend(state)
            .with_heartbeat(Duration::from_secs(1));
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        stops.push(stop);
        tokio::spawn(async move {
            let source = queue.worker("workers").await.unwrap();
            worker
                .serve(&source, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
    }

    // The workflow process: a local step, then the remote one.
    let definition = WorkflowBuilder::<String>::new("remote")
        .set_start("start")
        .add_edge(Edge::direct("start", "shout"))
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    registry.register(Start);
    registry.register(
        RemoteExecutor::<String, String, String, _>::new("shout", open_queue().await, "workers")
            .with_wait(Duration::from_secs(20)),
    );
    let flow = Arc::new(
        Workflow::with_config(
            definition,
            registry,
            WorkflowConfig::new().with_executor_timeout_ms(30_000),
        )
        .with_state_backend(Arc::new(
            NatsStateBackend::open(nats().await, &name).await.unwrap(),
        )),
    );

    let pending = (0..10).map(|n| {
        let flow = flow.clone();
        tokio::spawn(async move { flow.run(format!("job{n}")).await.unwrap() })
    });
    let mut outputs = Vec::new();
    for run in pending {
        let result = run.await.unwrap();
        assert!(result.success, "{:?}", result.error);
        outputs.extend(result.outputs);
    }
    outputs.sort();
    let mut expected: Vec<String> = (0..10).map(|n| format!("JOB{n}!")).collect();
    expected.sort();
    // Upper-cased on a worker, with the note the local step wrote to NATS.
    assert_eq!(outputs, expected);
    assert_eq!(runs.load(Ordering::SeqCst), 10, "no task ran twice");
    drop(stops);
}
