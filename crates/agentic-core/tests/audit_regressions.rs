use async_trait::async_trait;
use dasein_agentic_core::distributed::bus::{
    AuditCollector, AuditEvent, NatsClient, NatsConfig, Sequencer, SequencerConfig, StreamConfig,
    Task,
};
use dasein_agentic_core::distributed::graph::agent::ToolParameters;
use dasein_agentic_core::distributed::graph::*;
use futures::StreamExt;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;

#[test]
fn audit_tool_schema_defaults_to_object() {
    assert_eq!(
        serde_json::to_value(ToolParameters::new()).unwrap()["type"],
        "object"
    );
}

#[test]
fn audit_unicode_event_does_not_panic() {
    let task = format!("{}é", "a".repeat(99));
    let _event = AuditEvent::pipeline_started("unicode", 1, &task);
}

#[tokio::test]
async fn audit_blocked_tasks_respect_queue_capacity() {
    let seq = Sequencer::with_config(SequencerConfig {
        max_pending: 1,
        ..Default::default()
    });
    seq.publish(Task::new("one", "s", json!({})).with_depends_on(vec!["missing".into()]))
        .await
        .unwrap();
    let result = seq
        .publish(Task::new("two", "s", json!({})).with_depends_on(vec!["missing".into()]))
        .await;
    let stats = seq.stats().await;
    assert!(
        result.is_err(),
        "accepted {} blocked tasks with capacity 1",
        stats.blocked_count
    );
}

struct Worker {
    id: ExecutorId,
    calls: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    fail_first: bool,
    relay: bool,
}
#[async_trait]
impl Executor for Worker {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        &self.id
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, input: String, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        let attempt = self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_first && attempt == 0 {
            return Err(ExecutorError::new(self.id.clone(), "transient"));
        }
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(50)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        if self.relay {
            ctx.send_message(input).await?;
        } else {
            ctx.yield_output(input).await?;
        }
        Ok(())
    }
}

#[tokio::test]
async fn audit_workflow_retries_transient_failure() {
    let calls = Arc::new(AtomicUsize::new(0));
    let worker = Worker {
        id: "a".into(),
        calls: calls.clone(),
        active: Arc::new(AtomicUsize::new(0)),
        peak: Arc::new(AtomicUsize::new(0)),
        fail_first: true,
        relay: false,
    };
    let definition = WorkflowBuilder::<String>::new("retry")
        .set_start("a")
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    registry.register(worker);
    let flow = Workflow::with_config(
        definition,
        registry,
        WorkflowConfig::new()
            .with_max_retries(3)
            .with_retry_backoff(1, 2),
    );
    let result = flow.run("hello".to_string()).await;
    assert!(
        result.as_ref().is_ok_and(|r| r.success),
        "calls={}, result={result:?}",
        calls.load(Ordering::SeqCst)
    );
}

#[tokio::test]
async fn audit_fanout_workers_execute_concurrently() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let definition = WorkflowBuilder::<String>::new("parallel")
        .set_start("start")
        .add_fan_out_edge("start", vec!["a", "b"])
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    for name in ["start", "a", "b"] {
        registry.register(Worker {
            id: name.into(),
            calls: Arc::new(AtomicUsize::new(0)),
            active: active.clone(),
            peak: peak.clone(),
            fail_first: false,
            relay: name == "start",
        });
    }
    let result = Workflow::new(definition, registry)
        .run("hello".to_string())
        .await
        .unwrap();
    assert!(result.success);
    assert_eq!(peak.load(Ordering::SeqCst), 2, "fanout remained sequential");
}

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

#[tokio::test]
#[ignore = "Requires disposable NATS/Redis; see scripts/test-services.sh"]
async fn audit_nats_100_agent_delivery() {
    let bus = nats().await;
    let subject = format!("auditprobe.{}", uuid::Uuid::new_v4().simple());
    let mut sub = bus.subscribe(&subject).await.unwrap();
    bus.client().flush().await.unwrap();
    let mut tasks = Vec::new();
    for i in 0..100 {
        let bus = bus.clone();
        let subject = subject.clone();
        tasks.push(tokio::spawn(async move {
            bus.publish(&subject, &i).await.unwrap();
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    bus.client().flush().await.unwrap();
    let mut received = std::collections::HashSet::new();
    for _ in 0..100 {
        let msg = tokio::time::timeout(Duration::from_secs(3), sub.next())
            .await
            .unwrap()
            .unwrap();
        received.insert(serde_json::from_slice::<usize>(&msg.payload).unwrap());
    }
    assert_eq!(received.len(), 100);
}

#[tokio::test]
#[ignore = "Requires disposable NATS/Redis; see scripts/test-services.sh"]
async fn audit_nats_persistence_after_client_reconnect() {
    let bus = nats().await;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let stream = format!("PROBE_{id}");
    let subject = format!("probe.{id}");
    bus.ensure_stream(StreamConfig::new(&stream).subjects(vec![subject.clone()]))
        .await
        .unwrap();
    bus.publish_jetstream(&subject, &json!({"finding":42}))
        .await
        .unwrap()
        .await
        .unwrap();
    drop(bus);
    let bus = nats().await;
    let consumer = bus
        .create_consumer(&stream, "reader", Some(&subject))
        .await
        .unwrap();
    let mut messages = consumer.messages().await.unwrap();
    let message = tokio::time::timeout(Duration::from_secs(3), messages.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&message.payload).unwrap()["finding"],
        42
    );
    message.ack().await.unwrap();
}

#[tokio::test]
#[ignore = "Requires disposable NATS/Redis; see scripts/test-services.sh"]
async fn audit_trace_history_can_be_read_twice() {
    let bus = nats().await;
    let collector = AuditCollector::new(bus.clone()).await.unwrap();
    let trace = format!("repeat-{}", uuid::Uuid::new_v4().simple());
    // Await broker ACK explicitly so this test isolates the read path.
    bus.publish_jetstream(
        &format!("audit.{trace}"),
        &AuditEvent::pipeline_started(&trace, 1, "hello"),
    )
    .await
    .unwrap()
    .await
    .unwrap();
    let first = collector.for_trace_persistent(&trace).await.unwrap();
    let second = collector.for_trace_persistent(&trace).await.unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(
        second.len(),
        1,
        "second query lost the already-ACKed history"
    );
}

#[tokio::test]
#[ignore = "Requires disposable NATS/Redis; see scripts/test-services.sh"]
async fn audit_emit_reports_missing_stream() {
    let bus = nats().await;
    let collector = AuditCollector::new(bus.clone()).await.unwrap();
    bus.jetstream()
        .unwrap()
        .delete_stream("AGENTIC_AUDIT")
        .await
        .unwrap();
    let result = collector
        .emit(AuditEvent::pipeline_started("missing-stream", 1, "hello"))
        .await;
    assert!(
        result.is_err(),
        "emit returned success without durable storage"
    );
}

#[tokio::test]
#[ignore = "Requires disposable NATS/Redis; see scripts/test-services.sh"]
async fn audit_concurrent_thread_index_keeps_every_thread() {
    use dasein_agentic_core::distributed::graph::agent::{
        AgentId, AgentThread, NatsThreadStore, ThreadStore,
    };
    let store = Arc::new(NatsThreadStore::new(nats().await).await.unwrap());
    let agent = AgentId::new(format!("agent-{}", uuid::Uuid::new_v4().simple()));
    let gate = Arc::new(tokio::sync::Barrier::new(50));
    let mut tasks = Vec::new();
    for _ in 0..50 {
        let store = store.clone();
        let agent = agent.clone();
        let gate = gate.clone();
        tasks.push(tokio::spawn(async move {
            let thread = AgentThread::for_agent(agent);
            gate.wait().await;
            store.save(&thread).await.unwrap();
            thread.id
        }));
    }
    let mut ids = Vec::new();
    for task in tasks {
        ids.push(task.await.unwrap());
    }
    for id in &ids {
        assert!(store.load(id).await.unwrap().is_some());
    }
    let listed = store.list_by_agent(&agent).await.unwrap();
    assert_eq!(
        listed.len(),
        50,
        "all 50 bodies exist, but index lost entries"
    );
}

#[tokio::test]
async fn audit_checkpoint_contains_shared_state() {
    let definition = WorkflowBuilder::<String>::new("checkpoint")
        .set_start("start")
        .add_direct_edge("start", "end")
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    for name in ["start", "end"] {
        registry.register(Worker {
            id: name.into(),
            calls: Arc::new(AtomicUsize::new(0)),
            active: Arc::new(AtomicUsize::new(0)),
            peak: Arc::new(AtomicUsize::new(0)),
            fail_first: false,
            relay: name == "start",
        });
    }
    let state = Arc::new(InMemoryStateBackend::new());
    state
        .set("10:checkpoint15:checkpoint-task:discovery", json!(42))
        .await
        .unwrap();
    let checkpoints = Arc::new(InMemoryPersistentBackend::new());
    let flow = Workflow::with_config(
        definition,
        registry,
        WorkflowConfig::new()
            .with_checkpointing(true)
            .with_checkpoint_interval(1),
    )
    .with_state_backend(state.clone())
    .with_persistent_backend(checkpoints.clone());
    let task = TaskId::new("checkpoint-task");
    assert!(
        flow.run_with_resume(task.clone(), None, "hello".to_string())
            .await
            .unwrap()
            .success
    );
    let saved = checkpoints
        .load_persistent(flow.id(), &task)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved
            .shared_state
            .get("10:checkpoint15:checkpoint-task:discovery"),
        Some(&json!(42)),
        "checkpoint silently omitted shared state"
    );
    assert!(!saved.checkpoint.outputs.is_empty());
    state
        .set("10:checkpoint15:checkpoint-task:discovery", json!(0))
        .await
        .unwrap();
    state
        .set("10:checkpoint15:checkpoint-task:stale", json!(true))
        .await
        .unwrap();
    let resumed = flow
        .run_with_resume(task, None, "different input")
        .await
        .unwrap();
    assert!(resumed.success);
    assert_eq!(
        state
            .get("10:checkpoint15:checkpoint-task:stale")
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        serde_json::to_value(resumed.outputs).unwrap(),
        serde_json::to_value(saved.checkpoint.outputs).unwrap()
    );
    assert_eq!(
        state
            .get("10:checkpoint15:checkpoint-task:discovery")
            .await
            .unwrap(),
        Some(json!(42))
    );
}

#[tokio::test]
async fn last_allowed_superstep_can_complete() {
    let definition = WorkflowBuilder::<String>::new("boundary")
        .set_start("only")
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    registry.register(Worker {
        id: "only".into(),
        calls: Arc::new(AtomicUsize::new(0)),
        active: Arc::new(AtomicUsize::new(0)),
        peak: Arc::new(AtomicUsize::new(0)),
        fail_first: false,
        relay: false,
    });
    let flow = Workflow::with_config(
        definition,
        registry,
        WorkflowConfig::new().with_max_supersteps(1),
    );
    assert!(flow.run("hello").await.unwrap().success);
    let events: Vec<_> = flow.run_stream("hello").collect().await;
    assert!(matches!(
        events.last(),
        Some(WorkflowStreamEvent::Completed {
            output_count: 1,
            superstep_count: 1,
            ..
        })
    ));
}
