# Running workflow steps on other machines

A workflow runs in one process. Any of its steps can be handed to workers in other processes, on the same machine or elsewhere, through a task queue. The workflow engine does not change: a remote step is an ordinary executor that sends its input to the queue and waits for the result.

```text
 process A (workflow)                         process B, C... (workers)
 ┌────────────────────┐      task queue      ┌─────────────────────┐
 │ RemoteExecutor "x" │ ── input ─────────▶  │ RemoteWorker        │
 │                    │ ◀─ messages/outputs  │   real executor "x" │
 └────────────────────┘                      └─────────────────────┘
```

With `DurableQueue` the queue is NATS JetStream: tasks survive a restart, a worker that dies has its task handed to another, and adding a machine means starting one more worker.

## The worker process

```rust
use dasein_agentic_core::distributed::bus::{DurableQueue, DurableQueueConfig, NatsClient, NatsConfig};
use dasein_agentic_core::distributed::graph::*;

let nats = Arc::new(NatsClient::connect(NatsConfig::new("nats://nats:4222")).await?);
let queue = DurableQueue::new(nats.clone(), "prod", DurableQueueConfig::default()).await?;

// The executors this worker hosts.
let mut registry = ExecutorRegistry::new();
registry.register(Summarize::new(llm));

let worker = RemoteWorker::new(registry)
    // Same bucket as the workflow, so both see the same shared state.
    .with_state_backend(Arc::new(NatsStateBackend::open(nats, "prod").await?));

let source = queue.worker("summarizers").await?;
worker.serve(&source, shutdown_signal()).await?;
```

## The workflow process

```rust
let mut registry = ExecutorRegistry::new();
registry.register(Fetch::new());                       // runs here
registry.register(                                     // runs on a worker
    RemoteExecutor::<Doc, Doc, Summary, _>::new("summarize", queue, "summarizers")
        .with_wait(Duration::from_secs(120)),
);
let workflow = Workflow::with_config(
    definition,
    registry,
    WorkflowConfig::new().with_executor_timeout_ms(130_000),
)
.with_state_backend(Arc::new(NatsStateBackend::open(nats, "prod").await?));
```

`RemoteExecutor::new(id, queue, group)`: `id` is the ID of the executor the workers host, `group` the name the workers passed to `queue.worker(...)`. The type parameters are that executor's input, message and output types.

## What to know

- **At least once.** A task whose worker died mid-way is run again. A remote executor with side effects must make them idempotent.
- **One task per invocation.** An invocation is identified by workflow, run, superstep, executor and input, and always maps to the same queue task. A retry or a resumed workflow finds the work already submitted, or already done, instead of starting it again.
- **Results are final.** That includes failures: transient errors are retried by the worker (3 attempts by default), and what it finally reports is what the workflow gets for that invocation.
- **Timeouts.** A remote step waits 50 s for its result by default, under the workflow's 60 s executor timeout. Raise both for longer work. When the wait runs out the step fails as a timeout; a retry resumes waiting for the same task.
- **Shared state needs a shared backend.** Without `with_state_backend` on both sides, a remote executor sees an empty state of its own.
- **One group, one set of executors.** Any worker of a group may receive any of its tasks: workers of a group must host the same executors. Use several groups to give different executors to different machines.
- **One task at a time per `serve`.** Run several `serve` loops, or several workers, for more parallelism.

## What it does not do

- Choose a worker by load or capability: tasks go to whichever worker of the group asks first.
- Cancel a task that is already running when the workflow gives up.
- Send back the events a remote executor emits with `add_event`.
- Move the workflow engine itself: supersteps, routing and checkpoints stay in the workflow process.

## Without NATS

`InMemoryTaskQueue` has the same contract inside one process, minus durability. It serves as both queue and source, for development and tests:

```rust
let queue = InMemoryTaskQueue::new();
tokio::spawn(async move { worker.serve(&queue_for_worker, shutdown).await });
```
