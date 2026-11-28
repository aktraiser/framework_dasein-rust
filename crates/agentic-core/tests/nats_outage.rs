//! A NATS server that goes away and comes back, between a client and the
//! real server: what was waiting for it carries on instead of failing.
//!
//! The tests talk to NATS through a small relay they can cut, so they need
//! nothing more than the disposable server of the other NATS tests.
use dasein_agentic_core::distributed::bus::*;
use dasein_agentic_core::distributed::graph::{NatsStateBackend, SharedStateBackend};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::watch;

/// A TCP relay to the NATS server that can be cut and restored.
struct Relay {
    url: String,
    up: watch::Sender<bool>,
}

impl Relay {
    async fn start() -> Self {
        let target = std::env::var("DASEIN_TEST_NATS_URL")
            .unwrap_or_else(|_| "nats://127.0.0.1:4222".into())
            .trim_start_matches("nats://")
            .to_owned();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("nats://{}", listener.local_addr().unwrap());
        let (up, state) = watch::channel(true);
        tokio::spawn(async move {
            loop {
                let Ok((mut client, _)) = listener.accept().await else {
                    return;
                };
                let mut state = state.clone();
                if !*state.borrow() {
                    // Cut: the connection is refused as soon as it is made.
                    continue;
                }
                let target = target.clone();
                tokio::spawn(async move {
                    let Ok(mut server) = tokio::net::TcpStream::connect(&target).await else {
                        return;
                    };
                    tokio::select! {
                        _ = tokio::io::copy_bidirectional(&mut client, &mut server) => {}
                        // Cut while in use: both ends are dropped.
                        _ = state.wait_for(|up| !*up) => {}
                    }
                });
            }
        });
        Self { url, up }
    }

    fn cut(&self) {
        self.up.send_replace(false);
    }

    fn restore(&self) {
        self.up.send_replace(true);
    }

    /// A client that gives up on a request after a second, so that an
    /// outage of a few seconds is one it has to wait out.
    async fn client(&self) -> Arc<NatsClient> {
        let mut config = NatsConfig::new(self.url.clone());
        config.request_timeout = Duration::from_secs(1);
        Arc::new(NatsClient::connect(config).await.unwrap())
    }
}

fn name() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// How long the server is away: longer than the client waits for one answer.
const OUTAGE: Duration = Duration::from_secs(7);

#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn shared_state_waits_for_a_server_that_is_away() {
    let relay = Relay::start().await;
    let state = Arc::new(
        NatsStateBackend::open(relay.client().await, &name())
            .await
            .unwrap(),
    );
    state.set("3:run1:a:kept", json!("before")).await.unwrap();

    relay.cut();
    let started = Instant::now();
    let writing = {
        let state = state.clone();
        tokio::spawn(async move { state.set("3:run1:a:note", json!("written")).await })
    };
    let listing = {
        let state = state.clone();
        tokio::spawn(async move { state.snapshot("3:run1:a:").await })
    };
    tokio::time::sleep(OUTAGE).await;
    assert!(
        !writing.is_finished(),
        "the write did not wait for the server"
    );
    relay.restore();

    // Both carried on once the server was back, instead of failing.
    writing.await.unwrap().unwrap();
    let listed = listing.await.unwrap().unwrap();
    assert_eq!(listed.get("3:run1:a:kept"), Some(&json!("before")));
    assert!(started.elapsed() >= OUTAGE);
    assert_eq!(
        state.get("3:run1:a:note").await.unwrap(),
        Some(json!("written"))
    );

    // Not for ever: with no patience left the failure is reported.
    let hasty = Arc::new(
        NatsStateBackend::open(relay.client().await, &name())
            .await
            .unwrap()
            .with_patience(Duration::from_secs(2)),
    );
    relay.cut();
    // The server stays away until the write has given up, however long one
    // attempt takes on this machine.
    assert!(hasty.set("3:run1:a:note", json!(1)).await.is_err());
    relay.restore();
}

#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn a_result_is_recorded_once_the_server_is_back_and_the_task_is_not_run_again() {
    let relay = Relay::start().await;
    let queue = DurableQueue::new(
        relay.client().await,
        &name(),
        DurableQueueConfig {
            ack_wait: Duration::from_secs(60),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let worker = queue.worker("worker").await.unwrap();
    queue
        .publish(Task::new("one", "worker", json!(1)))
        .await
        .unwrap();
    let delivery = worker.next(Duration::from_secs(5)).await.unwrap().unwrap();

    // The work is done while the server is away.
    relay.cut();
    let reporting = tokio::spawn(delivery.complete(json!("done")));
    tokio::time::sleep(OUTAGE).await;
    assert!(
        !reporting.is_finished(),
        "the report did not wait for the server"
    );
    relay.restore();

    reporting.await.unwrap().unwrap();
    assert_eq!(
        queue.outcome("one").await.unwrap(),
        Some(TaskOutcome::Completed(json!("done")))
    );
    // Recorded and acknowledged: nothing is handed out again.
    assert!(worker.next(Duration::from_secs(2)).await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn a_remote_step_spends_one_wait_on_sending_and_on_its_result_together() {
    use async_trait::async_trait;
    use dasein_agentic_core::distributed::graph::*;

    /// The local first step.
    struct Pass;
    #[async_trait]
    impl Executor for Pass {
        type Input = String;
        type Message = String;
        type Output = String;
        fn id(&self) -> &ExecutorId {
            static ID: std::sync::OnceLock<ExecutorId> = std::sync::OnceLock::new();
            ID.get_or_init(|| "pass".into())
        }
        fn kind(&self) -> ExecutorKind {
            ExecutorKind::Worker
        }
        async fn handle<C>(&self, input: String, ctx: &mut C) -> Result<(), ExecutorError>
        where
            C: ExecutorContext<String, String> + Send,
        {
            ctx.send_message(input).await
        }
    }

    let relay = Relay::start().await;
    let queue = DurableQueue::new(relay.client().await, &name(), DurableQueueConfig::default())
        .await
        .unwrap();
    let definition = WorkflowBuilder::<String>::new("waiting")
        .set_start("pass")
        .add_edge(Edge::direct("pass", "far"))
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    registry.register(Pass);
    // Nobody serves this group: the step can only run out of time.
    let wait = Duration::from_secs(9);
    registry.register(
        RemoteExecutor::<String, String, String, _>::new("far", queue, "nobody").with_wait(wait),
    );
    let workflow = Workflow::with_config(
        definition,
        registry,
        WorkflowConfig::new()
            .with_max_retries(0)
            .with_executor_timeout_ms(60_000),
    );

    // The server is away for most of the wait, then back.
    relay.cut();
    let started = Instant::now();
    let running = tokio::spawn(async move { workflow.run("x".to_string()).await });
    tokio::time::sleep(Duration::from_secs(6)).await;
    relay.restore();
    let result = running.await.unwrap().unwrap();
    let took = started.elapsed();

    assert!(!result.success);
    // It waited out the outage rather than failing at once, and did not
    // start a second full wait for the result after it.
    assert!(took >= Duration::from_secs(6), "{took:?}");
    assert!(took < wait + Duration::from_secs(3), "{took:?}");
}
