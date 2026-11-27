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
                .await;
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
            .await;
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

/// One invocation of the remote `shout` step, always the same one: what a
/// retry or a resumed workflow presents again.
async fn invoke(
    step: &RemoteExecutor<String, String, String, InMemoryTaskQueue>,
    input: &str,
) -> Result<Vec<String>, ExecutorError> {
    let mut ctx = WorkflowContextBuilder::<String, String>::new(
        WorkflowId::new("remote"),
        TaskId::new("run-1"),
        ExecutorId::new("shout"),
    )
    .superstep(1)
    .build();
    step.handle(input.to_string(), &mut ctx).await?;
    Ok(ctx.drain_outputs())
}

fn step(setup: &Setup, group: &str) -> RemoteExecutor<String, String, String, InMemoryTaskQueue> {
    RemoteExecutor::new("shout", setup.queue.clone(), group).with_wait(Duration::from_secs(5))
}

#[tokio::test]
async fn task_nobody_took_in_time_is_dropped_and_a_later_attempt_runs() {
    let setup = Setup::new();
    let step = step(&setup, "workers").with_task_ttl(Duration::from_millis(50));

    // Sent while no worker is there, and found only once it has expired.
    let (first, ()) = tokio::join!(invoke(&step, "stale"), async {
        tokio::time::sleep(Duration::from_millis(200)).await;
        setup
            .worker(0)
            .run(
                setup
                    .queue
                    .next(Duration::from_secs(5))
                    .await
                    .unwrap()
                    .expect("the step submitted a task"),
            )
            .await;
    });
    let error = first.unwrap_err();
    assert!(error.message.contains("expired"), "{error}");
    assert!(!error.retriable);
    assert_eq!(
        setup.runs.load(Ordering::SeqCst),
        0,
        "an expired task is not run"
    );

    // The same invocation again, as a resumed workflow would present it:
    // the dropped task is not its answer, a new one is sent and runs.
    let _worker = setup.start_worker(0);
    assert_eq!(invoke(&step, "stale").await.unwrap(), ["STALE"]);
    assert_eq!(setup.runs.load(Ordering::SeqCst), 1);
    // And that result is then remembered.
    assert_eq!(invoke(&step, "stale").await.unwrap(), ["STALE"]);
    assert_eq!(setup.runs.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn invocation_passes_once_the_workers_host_its_executor() {
    let setup = Setup::new();
    let step = step(&setup, "workers");

    // A worker deployed without the executor.
    let empty: RemoteWorker<String, String> = RemoteWorker::new(ExecutorRegistry::new());
    let (first, ()) = tokio::join!(invoke(&step, "later"), async {
        let claim = setup.queue.next(Duration::from_secs(5)).await.unwrap();
        empty.run(claim.expect("the step submitted a task")).await;
    });
    let error = first.unwrap_err();
    assert!(
        error.message.contains("not hosted by this worker"),
        "{error}"
    );

    // The deployment is fixed and the run resumed.
    let _worker = setup.start_worker(0);
    assert_eq!(invoke(&step, "later").await.unwrap(), ["LATER"]);
}

#[tokio::test]
async fn result_too_large_to_report_is_a_final_failure() {
    let setup = Setup::new();
    let worker = setup.worker(0).with_max_result_bytes(600);
    let queue = setup.queue.clone();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        worker
            .serve(&queue, async {
                let _ = stopped.await;
            })
            .await;
    });
    let step = step(&setup, "workers");

    assert_eq!(invoke(&step, "small").await.unwrap(), ["SMALL"]);
    let error = invoke(&step, &"x".repeat(400)).await.unwrap_err();
    assert!(error.message.contains("too large"), "{error}");
    assert!(!error.retriable);
    // Asking again does not run the executor again.
    assert!(invoke(&step, &"x".repeat(400)).await.is_err());
    assert_eq!(setup.runs.load(Ordering::SeqCst), 2);
    drop(stop);
}

#[tokio::test]
async fn in_memory_worker_takes_the_tasks_of_its_group_only() {
    let setup = Setup::new();
    let for_cpu = step(&setup, "cpu");
    let for_gpu = step(&setup, "gpu");
    let gpu_source = setup.queue.worker("gpu");
    let cpu_source = setup.queue.worker("cpu");

    let (on_cpu, on_gpu, ()) = tokio::join!(invoke(&for_cpu, "c"), invoke(&for_gpu, "g"), async {
        // Both tasks are waiting; each source sees its own.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let gpu = gpu_source
            .next(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(gpu.task().supervisor, "gpu");
        assert!(gpu_source
            .next(Duration::from_millis(50))
            .await
            .unwrap()
            .is_none());
        let cpu = cpu_source
            .next(Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cpu.task().supervisor, "cpu");
        let worker = setup.worker(0);
        worker.run(gpu).await;
        worker.run(cpu).await;
    });
    assert_eq!(on_cpu.unwrap(), ["C"]);
    assert_eq!(on_gpu.unwrap(), ["G"]);
}

#[tokio::test]
async fn invalid_group_name_fails_at_once_with_the_reason() {
    let setup = Setup::new();
    let error = invoke(&step(&setup, "workers.eu"), "x").await.unwrap_err();
    assert!(error.message.contains("worker group name"), "{error}");
    assert!(!error.retriable);
}

/// A source whose first reads fail, as during a server restart.
struct Flaky {
    queue: InMemoryTaskQueue,
    failures_left: AtomicUsize,
}

#[async_trait]
impl TaskSource for Flaky {
    type Claim = InMemoryClaim;
    async fn next(
        &self,
        timeout: Duration,
    ) -> Result<Option<InMemoryClaim>, dasein_agentic_core::distributed::bus::BusError> {
        // Only one worker reads this source.
        let left = self.failures_left.load(Ordering::SeqCst);
        let failing = left > 0;
        if failing {
            self.failures_left.store(left - 1, Ordering::SeqCst);
        }
        if failing {
            return Err(
                dasein_agentic_core::distributed::bus::BusError::PublishFailed(
                    "connection lost".into(),
                ),
            );
        }
        self.queue.next(timeout).await
    }
}

#[tokio::test]
async fn worker_keeps_serving_after_the_queue_was_unreachable() {
    let setup = Setup::new();
    let source = Flaky {
        queue: setup.queue.clone(),
        failures_left: AtomicUsize::new(2),
    };
    let worker = setup.worker(0);
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(async move {
        worker
            .serve(&source, async {
                let _ = stopped.await;
            })
            .await;
    });

    assert_eq!(
        invoke(&step(&setup, "workers"), "back").await.unwrap(),
        ["BACK"]
    );
    drop(stop);
    serving.await.unwrap();
}

/// Sends its input on four times: four equal inputs for the next step, in
/// one superstep.
struct Fan;

#[async_trait]
impl Executor for Fan {
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
        for _ in 0..4 {
            ctx.send_message(input.clone()).await?;
        }
        Ok(())
    }
}

/// Takes a moment over each input, and records how many it had in hand at
/// the same time.
#[derive(Clone, Default)]
struct Slow {
    runs: Arc<AtomicUsize>,
    in_hand: Arc<AtomicUsize>,
    most_in_hand: Arc<AtomicUsize>,
}

#[async_trait]
impl Executor for Slow {
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
        self.runs.fetch_add(1, Ordering::SeqCst);
        let in_hand = self.in_hand.fetch_add(1, Ordering::SeqCst) + 1;
        self.most_in_hand.fetch_max(in_hand, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(150)).await;
        self.in_hand.fetch_sub(1, Ordering::SeqCst);
        ctx.yield_output(input.to_uppercase()).await
    }
}

fn fan_definition() -> WorkflowDefinition<String> {
    WorkflowBuilder::<String>::new("fan")
        .set_start("start")
        .add_edge(Edge::direct("start", "shout"))
        .build()
        .unwrap()
}

#[tokio::test]
async fn equal_inputs_of_one_superstep_each_run_and_in_parallel() {
    let queue = InMemoryTaskQueue::new();
    let slow = Slow::default();
    // Four workers, as four machines would be.
    let mut stops = Vec::new();
    for _ in 0..4 {
        let mut registry = ExecutorRegistry::new();
        registry.register(slow.clone());
        let worker: RemoteWorker<String, String> = RemoteWorker::new(registry);
        let source = queue.worker("workers");
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        stops.push(stop);
        tokio::spawn(async move {
            worker
                .serve(&source, async {
                    let _ = stopped.await;
                })
                .await;
        });
    }

    let mut registry = ExecutorRegistry::new();
    registry.register(Fan);
    registry.register(
        RemoteExecutor::<String, String, String, _>::new("shout", queue, "workers")
            .with_wait(Duration::from_secs(5)),
    );
    let result = Workflow::new(fan_definition(), registry)
        .run("same".to_string())
        .await
        .unwrap();
    assert!(result.success, "{:?}", result.error);
    // Equal inputs are not merged into one task.
    assert_eq!(result.outputs, ["SAME"; 4]);
    assert_eq!(slow.runs.load(Ordering::SeqCst), 4);
    // And they did not wait for one another.
    assert!(
        slow.most_in_hand.load(Ordering::SeqCst) > 1,
        "the four inputs ran one at a time"
    );
    drop(stops);
}

#[tokio::test]
async fn local_step_still_takes_its_inputs_one_at_a_time() {
    let slow = Slow::default();
    let mut registry = ExecutorRegistry::new();
    registry.register(Fan);
    registry.register(slow.clone());
    let result = Workflow::new(fan_definition(), registry)
        .run("same".to_string())
        .await
        .unwrap();
    assert!(result.success, "{:?}", result.error);
    assert_eq!(result.outputs, ["SAME"; 4]);
    assert_eq!(slow.most_in_hand.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn in_memory_queue_forgets_the_oldest_results() {
    use dasein_agentic_core::distributed::bus::{Task, TaskOutcome};
    // Asking for fewer than 100 still keeps 100.
    let queue = InMemoryTaskQueue::new().remembering(2);
    for n in 0..150 {
        let id = n.to_string();
        queue
            .submit(Task::new(id.clone(), "group", serde_json::json!(n)))
            .await
            .unwrap();
        let claim = queue.next(Duration::from_secs(1)).await.unwrap().unwrap();
        claim.complete(serde_json::json!(n)).await.unwrap();
    }
    // A queue serving for a long time keeps the latest results only.
    for forgotten in ["0", "49"] {
        assert!(queue.outcome(forgotten).await.unwrap().is_none());
    }
    for kept in ["50", "149"] {
        assert!(matches!(
            queue.outcome(kept).await.unwrap(),
            Some(TaskOutcome::Completed(_))
        ));
    }
}
