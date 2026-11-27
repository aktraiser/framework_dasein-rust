//! Load and duration test of workflows with a remote step.
//!
//! Runs the same small workflow over and over, from several tasks at once,
//! for a given time, and checks every result. Prints a progress line every
//! ten seconds and a summary; exits with 1 if a run failed or returned
//! something else than expected.
//!
//! ```bash
//! # Everything in this process, no server needed:
//! cargo run --release -p dasein-agentic-core --example soak -- memory 60
//!
//! # Through NATS JetStream (queue, results and shared state on the server):
//! DASEIN_NATS_URL=nats://127.0.0.1:4222 \
//!   cargo run --release -p dasein-agentic-core --example soak -- nats 600
//! ```
//!
//! Optional arguments after the seconds: how many workflows run at once
//! (default 8) and how many workers serve them (default 4).
//! `DASEIN_SOAK_NAME` names the queue and state on NATS (default `soak`),
//! `DASEIN_WORK_MS` is how long a worker takes over one input (default 0),
//! `DASEIN_SOAK_RESULT_MIB` the size of the result ledger (default 512) and
//! `DASEIN_SOAK_RESULT_S` how long it keeps a result (default 120).
//!
//! Each run sends `ITEMS` inputs to the workers and leaves a note in shared
//! state that is its own: an output built from another run's note, or
//! outputs out of order, count as wrong.

use async_trait::async_trait;
use dasein_agentic_core::distributed::bus::{
    DurableQueue, DurableQueueConfig, NatsClient, NatsConfig,
};
use dasein_agentic_core::distributed::graph::{
    Edge, Executor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind, ExecutorRegistry,
    InMemoryStateBackend, InMemoryTaskQueue, NatsStateBackend, RemoteExecutor, RemoteWorker,
    SharedStateBackend, TaskQueue, TaskSource, Workflow, WorkflowBuilder, WorkflowConfig,
};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const GROUP: &str = "soak";
const ITEMS: usize = 4;

/// Local first step: `"<run>"` in, a note for that run in shared state, and
/// `ITEMS` messages `"<run>-<n>"` out.
struct Split;

#[async_trait]
impl Executor for Split {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        static ID: std::sync::OnceLock<ExecutorId> = std::sync::OnceLock::new();
        ID.get_or_init(|| "split".into())
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, run: String, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        ctx.set_shared_state("note", format!("note-of-{run}"))
            .await?;
        for n in 0..ITEMS {
            ctx.send_message(format!("{run}-{n}")).await?;
        }
        Ok(())
    }
}

/// Hosted by the workers: joins its input with the note of the run.
struct Join {
    work: Duration,
}

#[async_trait]
impl Executor for Join {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        static ID: std::sync::OnceLock<ExecutorId> = std::sync::OnceLock::new();
        ID.get_or_init(|| "join".into())
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, input: String, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        if !self.work.is_zero() {
            tokio::time::sleep(self.work).await;
        }
        let note: Option<String> = ctx.get_shared_state("note").await?;
        ctx.yield_output(format!(
            "{input}:{}",
            note.as_deref().unwrap_or("<missing>")
        ))
        .await
    }
}

fn expected(run: &str) -> Vec<String> {
    (0..ITEMS)
        .map(|n| format!("{run}-{n}:note-of-{run}"))
        .collect()
}

/// What to do with the shared state of a finished run.
#[async_trait]
trait Cleanup: Send + Sync {
    async fn forget(
        &self,
        result: &dasein_agentic_core::distributed::graph::WorkflowResult<String>,
    );
}

struct MemoryCleanup(Arc<InMemoryStateBackend>);
#[async_trait]
impl Cleanup for MemoryCleanup {
    async fn forget(
        &self,
        result: &dasein_agentic_core::distributed::graph::WorkflowResult<String>,
    ) {
        self.0.forget(&result.workflow_id, &result.task_id).await;
    }
}

struct NatsCleanup(Arc<NatsStateBackend>);
#[async_trait]
impl Cleanup for NatsCleanup {
    async fn forget(
        &self,
        result: &dasein_agentic_core::distributed::graph::WorkflowResult<String>,
    ) {
        if let Err(error) = self.0.forget(&result.workflow_id, &result.task_id).await {
            eprintln!("could not remove the shared state of a run: {error}");
        }
    }
}

/// Each latency band is this many times wider than the one before.
const BAND: f64 = 1.1;

#[derive(Default)]
struct Tally {
    runs: AtomicU64,
    failed: AtomicU64,
    wrong: AtomicU64,
    latencies: Mutex<Vec<u64>>,
    first_problems: Mutex<Vec<String>>,
}

impl Tally {
    fn problem(&self, text: String) {
        let mut problems = self.first_problems.lock().unwrap();
        if problems.len() < 5 {
            problems.push(text);
        }
    }

    /// Count a run's latency. Latencies are kept as counts per band, each
    /// band 10 % wider than the one before, so that the test's own memory
    /// does not grow with the number of runs.
    fn took(&self, elapsed: Duration) {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let band = ((elapsed.as_micros().max(1) as f64).ln() / BAND.ln()) as usize;
        let mut bands = self.latencies.lock().unwrap();
        if bands.len() <= band {
            bands.resize(band + 1, 0);
        }
        bands[band] += 1;
    }

    /// Latency at half, 99 % and all of the runs so far, in milliseconds
    /// (the upper edge of the band it falls in).
    fn percentiles(&self) -> (f64, f64, f64) {
        let bands = self.latencies.lock().unwrap().clone();
        let total: u64 = bands.iter().sum();
        let at = |fraction: f64| {
            #[allow(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            let wanted = ((total as f64) * fraction).ceil().max(1.0) as u64;
            let mut seen = 0;
            for (band, count) in bands.iter().enumerate() {
                seen += count;
                if seen >= wanted {
                    #[allow(clippy::cast_precision_loss, clippy::cast_possible_wrap)]
                    return BAND.powi(band as i32 + 1) / 1000.0;
                }
            }
            0.0
        };
        (at(0.5), at(0.99), at(1.0))
    }
}

/// Resident memory of this process in MiB, as `ps` reports it.
fn resident_mib() -> Option<f64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let kib: f64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    Some(kib / 1024.0)
}

struct Plan {
    seconds: u64,
    at_once: usize,
    workers: usize,
    work: Duration,
}

async fn soak<Q, S>(
    plan: Plan,
    queue: Q,
    sources: Vec<S>,
    state: Arc<dyn SharedStateBackend>,
    cleanup: Arc<dyn Cleanup>,
) -> bool
where
    Q: TaskQueue + 'static,
    S: TaskSource + 'static,
    S::Claim: Sync,
{
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = Arc::new(tokio::sync::Notify::new());

    let mut worker_tasks = Vec::new();
    for source in sources {
        let mut registry = ExecutorRegistry::new();
        registry.register(Join { work: plan.work });
        let worker = RemoteWorker::<String, String>::new(registry)
            .with_state_backend(state.clone())
            .with_heartbeat(Duration::from_secs(1));
        let (stop, stopped) = (stop.clone(), stopped.clone());
        worker_tasks.push(tokio::spawn(async move {
            worker
                .serve(&source, async move {
                    while !stop.load(Ordering::Relaxed) {
                        stopped.notified().await;
                    }
                })
                .await;
        }));
    }

    let definition = WorkflowBuilder::<String>::new("soak")
        .set_start("split")
        .add_edge(Edge::direct("split", "join"))
        .build()
        .expect("workflow definition");
    let mut registry = ExecutorRegistry::new();
    registry.register(Split);
    registry.register(
        RemoteExecutor::<String, String, String, _>::new("join", queue, GROUP)
            .with_wait(Duration::from_secs(120)),
    );
    let workflow = Arc::new(
        Workflow::with_config(
            definition,
            registry,
            WorkflowConfig::new().with_executor_timeout_ms(130_000),
        )
        .with_state_backend(state),
    );

    let tally = Arc::new(Tally::default());
    let started = Instant::now();
    let deadline = started + Duration::from_secs(plan.seconds);
    // Unique across restarts on the same server, so that an earlier soak's
    // results are never taken for this one's.
    let session = uuid::Uuid::new_v4().simple().to_string();
    let next_run = Arc::new(AtomicU64::new(0));

    let mut runners = Vec::new();
    for _ in 0..plan.at_once {
        let (workflow, tally, cleanup, next_run, session) = (
            workflow.clone(),
            tally.clone(),
            cleanup.clone(),
            next_run.clone(),
            session.clone(),
        );
        runners.push(tokio::spawn(async move {
            while Instant::now() < deadline {
                let run = format!("{session}-{}", next_run.fetch_add(1, Ordering::Relaxed));
                let begun = Instant::now();
                let result = workflow.run(run.clone()).await;
                tally.took(begun.elapsed());
                tally.runs.fetch_add(1, Ordering::Relaxed);
                match result {
                    Ok(result) if result.success => {
                        if result.outputs != expected(&run) {
                            tally.wrong.fetch_add(1, Ordering::Relaxed);
                            tally.problem(format!("{run}: wrong outputs {:?}", result.outputs));
                        }
                        cleanup.forget(&result).await;
                    }
                    Ok(result) => {
                        tally.failed.fetch_add(1, Ordering::Relaxed);
                        tally.problem(format!("{run}: {:?}", result.error));
                        cleanup.forget(&result).await;
                    }
                    Err(error) => {
                        tally.failed.fetch_add(1, Ordering::Relaxed);
                        tally.problem(format!("{run}: {error}"));
                    }
                }
            }
        }));
    }

    let first_memory = resident_mib();
    println!(
        "{} workflows at once, {} workers, {} inputs per run, for {} s",
        plan.at_once, plan.workers, ITEMS, plan.seconds
    );
    println!("  time     runs   runs/s   p50 ms   p99 ms   max ms  failed  wrong  memory MiB");
    let report = |tally: &Tally| {
        let runs = tally.runs.load(Ordering::Relaxed);
        let elapsed = started.elapsed().as_secs_f64();
        let (p50, p99, max) = tally.percentiles();
        #[allow(clippy::cast_precision_loss)]
        let rate = runs as f64 / elapsed.max(0.001);
        println!(
            "{:>5.0} s {:>8} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>7} {:>6} {:>11.1}",
            elapsed,
            runs,
            rate,
            p50,
            p99,
            max,
            tally.failed.load(Ordering::Relaxed),
            tally.wrong.load(Ordering::Relaxed),
            resident_mib().unwrap_or(0.0),
        );
    };
    let mut tick = tokio::time::interval(Duration::from_secs(10));
    tick.tick().await;
    while Instant::now() < deadline {
        tokio::select! {
            _ = tick.tick() => report(&tally),
            () = tokio::time::sleep_until(deadline.into()) => {}
        }
    }
    for runner in runners {
        let _ = runner.await;
    }
    stop.store(true, Ordering::Relaxed);
    stopped.notify_waiters();
    for task in worker_tasks {
        let _ = task.await;
    }
    report(&tally);

    let failed = tally.failed.load(Ordering::Relaxed);
    let wrong = tally.wrong.load(Ordering::Relaxed);
    if let (Some(first), Some(last)) = (first_memory, resident_mib()) {
        println!("memory: {first:.1} MiB at the start, {last:.1} MiB at the end");
    }
    for problem in tally.first_problems.lock().unwrap().iter() {
        println!("problem: {problem}");
    }
    println!(
        "{}",
        if failed == 0 && wrong == 0 {
            "RESULT: every run succeeded with the expected outputs"
        } else {
            "RESULT: some runs failed or returned wrong outputs"
        }
    );
    failed == 0 && wrong == 0
}

fn usage() -> ! {
    eprintln!("usage: soak memory|nats <seconds> [workflows-at-once] [workers]");
    std::process::exit(2);
}

fn number(what: &str, given: Option<&str>, default: Option<u64>) -> u64 {
    match (given, default) {
        (Some(text), _) => text.parse().unwrap_or_else(|_| {
            eprintln!("{what} must be a whole number, got `{text}`");
            std::process::exit(2);
        }),
        (None, Some(default)) => default,
        (None, None) => usage(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let argument = |at: usize| arguments.get(at).map(String::as_str);
    let at_once = number("workflows at once", argument(2), Some(8)).max(1);
    let workers = number("workers", argument(3), Some(4)).max(1);
    let plan = Plan {
        seconds: number("seconds", argument(1), None),
        at_once: usize::try_from(at_once)?,
        workers: usize::try_from(workers)?,
        work: Duration::from_millis(number(
            "DASEIN_WORK_MS",
            std::env::var("DASEIN_WORK_MS").ok().as_deref(),
            Some(0),
        )),
    };

    let passed = match argument(0) {
        Some("memory") => {
            let queue = InMemoryTaskQueue::new();
            let sources = (0..plan.workers).map(|_| queue.worker(GROUP)).collect();
            let state = Arc::new(InMemoryStateBackend::new());
            let cleanup = Arc::new(MemoryCleanup(state.clone()));
            soak(plan, queue, sources, state, cleanup).await
        }
        Some("nats") => {
            let url =
                std::env::var("DASEIN_NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
            let name = std::env::var("DASEIN_SOAK_NAME").unwrap_or_else(|_| "soak".into());
            let result_mib = number(
                "DASEIN_SOAK_RESULT_MIB",
                std::env::var("DASEIN_SOAK_RESULT_MIB").ok().as_deref(),
                Some(512),
            );
            let result_seconds = number(
                "DASEIN_SOAK_RESULT_S",
                std::env::var("DASEIN_SOAK_RESULT_S").ok().as_deref(),
                Some(120),
            );
            let nats = Arc::new(NatsClient::connect(NatsConfig::new(url)).await?);
            let queue = DurableQueue::new(
                nats.clone(),
                &name,
                DurableQueueConfig {
                    ack_wait: Duration::from_secs(5),
                    max_result_bytes: i64::try_from(result_mib)? * 1024 * 1024,
                    // Short, so that the ledger holds a run at full speed.
                    result_ttl: Duration::from_secs(result_seconds),
                    ..Default::default()
                },
            )
            .await?;
            let mut sources = Vec::new();
            for _ in 0..plan.workers {
                sources.push(queue.worker(GROUP).await?);
            }
            let state = Arc::new(NatsStateBackend::open(nats, &name).await?);
            let cleanup = Arc::new(NatsCleanup(state.clone()));
            soak(plan, queue, sources, state, cleanup).await
        }
        _ => usage(),
    };
    if !passed {
        std::process::exit(1);
    }
    Ok(())
}
