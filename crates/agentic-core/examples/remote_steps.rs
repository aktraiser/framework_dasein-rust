//! A workflow whose second step runs on workers in other processes, on this
//! machine or on others, through NATS JetStream.
//!
//! Start one or more workers, anywhere that can reach NATS:
//!
//! ```bash
//! DASEIN_NATS_URL=nats://127.0.0.1:4222 \
//!   cargo run -p dasein-agentic-core --example remote_steps -- worker demo alice
//! ```
//!
//! Then run the workflow, with the number of inputs to send to the workers:
//!
//! ```bash
//! DASEIN_NATS_URL=nats://127.0.0.1:4222 \
//!   cargo run -p dasein-agentic-core --example remote_steps -- workflow demo 8
//! ```
//!
//! `demo` names the queue and the shared state: workers and workflow must use
//! the same. Each output says which worker, on which system, produced it.
//! `DASEIN_WORK_MS` sets how long a worker takes over one input (default 500).
//!
//! A worker runs one input at a time, and the workflow gives up on an input
//! that has no result after `DASEIN_WAIT_S` seconds (default 300): start
//! enough workers for the load, or raise it.
//!
//! The shared state of a run is removed when it ends. The results of the
//! tasks stay in the bucket `DASEIN_RESULTS_<name>` of the server, as the
//! queue needs them to recognise a task it has already run: delete the bucket
//! when you are done with a name.

use async_trait::async_trait;
use dasein_agentic_core::distributed::bus::{
    DurableQueue, DurableQueueConfig, NatsClient, NatsConfig,
};
use dasein_agentic_core::distributed::graph::{
    Edge, Executor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind, ExecutorRegistry,
    NatsStateBackend, RemoteExecutor, RemoteWorker, Workflow, WorkflowBuilder, WorkflowConfig,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

const GROUP: &str = "describers";

/// Local first step: leaves a note in shared state, then sends one message
/// per item to the remote step.
struct Split;

#[async_trait]
impl Executor for Split {
    type Input = Vec<String>;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        static ID: std::sync::OnceLock<ExecutorId> = std::sync::OnceLock::new();
        ID.get_or_init(|| "split".into())
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, items: Vec<String>, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        ctx.set_shared_state("greeting", "hello from the workflow")
            .await?;
        for item in items {
            ctx.send_message(item).await?;
        }
        Ok(())
    }
}

/// Hosted by the workers: says who handled the input, and proves it read the
/// shared state the workflow process wrote.
struct Describe {
    worker: String,
    work: Duration,
}

#[async_trait]
impl Executor for Describe {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        static ID: std::sync::OnceLock<ExecutorId> = std::sync::OnceLock::new();
        ID.get_or_init(|| "describe".into())
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, input: String, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        println!("[{}] working on {input}", self.worker);
        tokio::time::sleep(self.work).await;
        let greeting: Option<String> = ctx.get_shared_state("greeting").await?;
        ctx.yield_output(format!(
            "{input} -> worker {} on {}/{} (pid {}), shared state: {}",
            self.worker,
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::process::id(),
            greeting.as_deref().unwrap_or("<missing>"),
        ))
        .await
    }
}

async fn connect(
    name: &str,
) -> Result<(DurableQueue, Arc<NatsStateBackend>), Box<dyn std::error::Error>> {
    let url = std::env::var("DASEIN_NATS_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
    let nats = Arc::new(NatsClient::connect(NatsConfig::new(url)).await?);
    let queue = DurableQueue::new(
        nats.clone(),
        name,
        DurableQueueConfig {
            // A task whose worker died is handed to another after this long.
            ack_wait: Duration::from_secs(5),
            ..Default::default()
        },
    )
    .await?;
    let state = Arc::new(NatsStateBackend::open(nats, name).await?);
    Ok((queue, state))
}

/// A number given by the user: the default when absent, and the usage error
/// when it is not a number, rather than a run that quietly does something
/// else than what was asked.
fn number(what: &str, given: Option<&str>, default: u64) -> u64 {
    match given {
        None => default,
        Some(text) => text.parse().unwrap_or_else(|_| {
            eprintln!("{what} must be a whole number, got `{text}`");
            std::process::exit(2);
        }),
    }
}

fn from_env(name: &str, default: u64) -> u64 {
    number(name, std::env::var(name).ok().as_deref(), default)
}

/// Ends on Ctrl-C and, on Unix, on the SIGTERM that `kill`, `docker stop` and
/// service managers send: the worker then finishes its task before leaving.
async fn stop_requested() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut terminate) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = terminate.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    let argument = |at: usize| arguments.get(at).map(String::as_str);
    match (argument(0), argument(1), argument(2)) {
        (Some("worker"), Some(name), Some(worker)) => {
            let work = from_env("DASEIN_WORK_MS", 500);
            let (queue, state) = connect(name).await?;
            let mut registry = ExecutorRegistry::new();
            registry.register(Describe {
                worker: worker.to_owned(),
                work: Duration::from_millis(work),
            });
            let source = queue.worker(GROUP).await?;
            println!("[{worker}] serving group `{GROUP}` of `{name}`; Ctrl-C to stop");
            RemoteWorker::<String, String>::new(registry)
                .with_state_backend(state)
                .with_heartbeat(Duration::from_secs(1))
                .serve(&source, stop_requested())
                .await;
            println!("[{worker}] stopped");
        }
        (Some("workflow"), Some(name), count) => {
            let count = number("the number of inputs", count, 4);
            let wait = Duration::from_secs(from_env("DASEIN_WAIT_S", 300));
            let (queue, state) = connect(name).await?;
            let definition = WorkflowBuilder::<String>::new("remote-steps")
                .set_start("split")
                .add_edge(Edge::direct("split", "describe"))
                .build()?;
            let mut registry = ExecutorRegistry::new();
            registry.register(Split);
            registry.register(
                RemoteExecutor::<String, String, String, _>::new("describe", queue, GROUP)
                    .with_wait(wait),
            );
            let workflow = Workflow::with_config(
                definition,
                registry,
                // Longer than the wait, so that the wait is what ends first.
                WorkflowConfig::new().with_executor_timeout_ms(
                    u64::try_from(wait.as_millis())
                        .unwrap_or(u64::MAX)
                        .saturating_add(10_000),
                ),
            )
            .with_state_backend(state.clone());

            let items: Vec<String> = (1..=count).map(|n| format!("item-{n}")).collect();
            let started = Instant::now();
            let result = workflow.run(items).await;
            let elapsed = started.elapsed();
            // The run is over, whatever its result: its shared state is no
            // longer needed.
            if let Ok(result) = &result {
                if let Err(error) = state.forget(&result.workflow_id, &result.task_id).await {
                    eprintln!("could not remove the shared state of the run: {error}");
                }
            }
            match result {
                Ok(result) if result.success => {
                    for output in &result.outputs {
                        println!("{output}");
                    }
                    println!(
                        "{} outputs in {:.1} s",
                        result.outputs.len(),
                        elapsed.as_secs_f32()
                    );
                }
                Ok(result) => {
                    eprintln!("workflow failed: {:?}", result.error);
                    std::process::exit(1);
                }
                Err(error) => {
                    eprintln!("workflow failed: {error}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("usage: remote_steps worker <name> <worker-name> | workflow <name> [count]");
            std::process::exit(2);
        }
    }
    Ok(())
}
