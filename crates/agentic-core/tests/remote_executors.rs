//! Workflow steps run by workers, over the in-process task queue.
use async_trait::async_trait;
use dasein_agentic_core::distributed::graph::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// The executor workers host: upper-cases its input and adds a note left in
/// shared state. Counts its runs, and can fail on purpose.
struct Shout {
    runs: Arc<AtomicUsize>,
    /// Runs that report a transient error before one succeeds.
    transient_failures: usize,
}

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
        let run = self.runs.fetch_add(1, Ordering::SeqCst);
        if input == "refuse" {
            return Err(ExecutorError::internal("refused on purpose"));
        }
        if run < self.transient_failures {
            return Err(ExecutorError::new("shout", "try again"));
        }
        let note: Option<String> = ctx.get_shared_state("note").await.unwrap_or(None);
        ctx.yield_output(format!(
            "{}{}",
            input.to_uppercase(),
            note.unwrap_or_default()
        ))
        .await
    }
}

/// The local first step: leaves a note in shared state and passes on.
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

struct Setup {
    queue: InMemoryTaskQueue,
    state: Arc<InMemoryStateBackend>,
    runs: Arc<AtomicUsize>,
}

impl Setup {
    fn new() -> Self {
        Self {
            queue: InMemoryTaskQueue::new(),
            state: Arc::new(InMemoryStateBackend::new()),
            runs: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// start (local) -> shout (remote).
    fn workflow(&self, wait: Duration) -> Workflow<String, String> {
        let definition = WorkflowBuilder::<String>::new("remote")
            .set_start("start")
            .add_edge(Edge::direct("start", "shout"))
            .build()
            .unwrap();
        let mut registry = ExecutorRegistry::new();
        registry.register(Start);
        registry.register(
            RemoteExecutor::<String, String, String, _>::new(
                "shout",
                self.queue.clone(),
                "workers",
            )
            .with_wait(wait),
        );
        Workflow::with_config(
            definition,
            registry,
            WorkflowConfig::new()
                .with_max_retries(8)
                .with_retry_backoff(20, 50),
        )
        .with_state_backend(self.state.clone())
    }

    fn worker(&self, transient_failures: usize) -> RemoteWorker<String, String> {
        let mut registry = ExecutorRegistry::new();
        registry.register(Shout {
            runs: self.runs.clone(),
            transient_failures,
        });
        RemoteWorker::new(registry).with_state_backend(self.state.clone())
    }

    /// Run a worker in the background until the returned sender is dropped.
    fn start_worker(&self, transient_failures: usize) -> tokio::sync::oneshot::Sender<()> {
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let (worker, queue) = (self.worker(transient_failures), self.queue.clone());
        tokio::spawn(async move {
            worker
                .serve(&queue, async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        stop
    }
}

#[tokio::test]
async fn remote_step_runs_on_a_worker_and_shares_state() {
    let setup = Setup::new();
    let _worker = setup.start_worker(0);
    let result = setup
        .workflow(Duration::from_secs(5))
        .run("hello".to_string())
        .await
        .unwrap();
    assert!(result.success, "{:?}", result.error);
    // Upper-cased by the worker, with the note the local step left in state.
    assert_eq!(result.outputs, ["HELLO!"]);
    assert_eq!(setup.runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn many_runs_across_two_workers_each_execute_once() {
    let setup = Setup::new();
    let _first = setup.start_worker(0);
    let _second = setup.start_worker(0);
    let flow = Arc::new(setup.workflow(Duration::from_secs(5)));
    let runs = (0..20).map(|n| {
        let flow = flow.clone();
        tokio::spawn(async move { flow.run(format!("job{n}")).await.unwrap() })
    });
    let mut outputs = Vec::new();
    for run in runs {
        let result = run.await.unwrap();
        assert!(result.success, "{:?}", result.error);
        outputs.extend(result.outputs);
    }
    outputs.sort();
    let mut expected: Vec<String> = (0..20).map(|n| format!("JOB{n}!")).collect();
    expected.sort();
    assert_eq!(outputs, expected);
    assert_eq!(setup.runs.load(Ordering::SeqCst), 20, "no task ran twice");
}

#[tokio::test]
async fn step_waits_for_a_worker_without_running_the_task_twice() {
    let setup = Setup::new();
    // No worker yet: the step times out and is retried by the workflow.
    let flow = setup.workflow(Duration::from_millis(100));
    let running = tokio::spawn(async move { flow.run("late".to_string()).await.unwrap() });
    tokio::time::sleep(Duration::from_millis(350)).await;
    let _worker = setup.start_worker(0);

    let result = running.await.unwrap();
    assert!(result.success, "{:?}", result.error);
    assert_eq!(result.outputs, ["LATE!"]);
    // Every retry found the task it had already submitted.
    assert_eq!(setup.runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn task_of_a_worker_that_gave_up_goes_to_another() {
    let setup = Setup::new();
    let flow = setup.workflow(Duration::from_secs(5));
    let running = tokio::spawn(async move { flow.run("orphan".to_string()).await.unwrap() });

    // A worker takes the task and disappears without reporting.
    let claim = setup
        .queue
        .next(Duration::from_secs(5))
        .await
        .unwrap()
        .expect("the remote step submitted a task");
    drop(claim);

    let _worker = setup.start_worker(0);
    let result = running.await.unwrap();
    assert!(result.success, "{:?}", result.error);
    assert_eq!(result.outputs, ["ORPHAN!"]);
}

#[tokio::test]
async fn worker_retries_transient_errors_and_reports_final_ones() {
    // Two transient failures, then success: retried on the worker.
    let setup = Setup::new();
    let _worker = setup.start_worker(2);
    let result = setup
        .workflow(Duration::from_secs(5))
        .run("flaky".to_string())
        .await
        .unwrap();
    assert!(result.success, "{:?}", result.error);
    assert_eq!(setup.runs.load(Ordering::SeqCst), 3);

    // A refusal is final: the workflow fails with the worker's reason, and
    // does not send the task again.
    let setup = Setup::new();
    let _worker = setup.start_worker(0);
    let result = setup
        .workflow(Duration::from_secs(5))
        .run("refuse".to_string())
        .await
        .unwrap();
    assert!(!result.success);
    let error = result.error.unwrap();
    assert!(error.contains("refused on purpose"), "{error}");
    assert_eq!(setup.runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn task_for_an_executor_no_worker_hosts_fails_clearly() {
    let setup = Setup::new();
    // A worker with an empty registry.
    let worker: RemoteWorker<String, String> = RemoteWorker::new(ExecutorRegistry::new());
    let queue = setup.queue.clone();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        worker
            .serve(&queue, async {
                let _ = stopped.await;
            })
            .await
            .unwrap();
    });

    let result = setup
        .workflow(Duration::from_secs(5))
        .run("anyone".to_string())
        .await
        .unwrap();
    drop(stop);
    assert!(!result.success);
    let error = result.error.unwrap();
    assert!(error.contains("not hosted by this worker"), "{error}");
}
