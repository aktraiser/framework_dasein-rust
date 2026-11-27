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
worker.serve(&source, shutdown_signal()).await;
```

`serve` runs until the shutdown signal. If the queue cannot be read, because NATS is restarting or the network is down, it keeps trying, at most every 30 seconds, and resumes by itself.

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
- **One task per invocation.** An invocation is identified by workflow, run, superstep, executor, input and the input's position in the superstep, and always maps to the same queue task. A retry or a resumed workflow finds the work already submitted, or already done, instead of starting it again.
- **Inputs of one superstep run in parallel.** When a remote step receives several inputs in the same superstep (a splitter, a join), each is a task of its own, equal inputs included, and all are sent at once: with enough workers they run at the same time, up to the workflow's `max_concurrent_executors`. Results come back in input order. Local steps still take their inputs one after the other; an executor of your own opts in with `concurrent_inputs()`.
- **Results are pushed, not polled.** With the durable queue a waiting step is told by NATS when its result is recorded.
- **Results are final.** That includes failures of the executor: transient errors are retried by the worker (3 attempts by default), and what it finally reports is what the workflow gets for that invocation.
- **A task that never ran can be sent again.** Two failures mean the executor did not run: the task expired before a worker took it, or it reached a worker that does not host its executor. The workflow gets that failure once. A later retry or resume of the same invocation sends a new task, so fixing the deployment and resuming the run works.
- **Tasks expire.** A task older than 15 minutes when a worker takes it is dropped instead of run, so work nobody waits for any more does not fill the queue. Change it with `RemoteExecutor::with_task_ttl`, and keep it above the time the step is given to succeed, retries included. The age is measured with the clocks of both machines: keep them in sync.
- **Results have a size limit.** A result that does not fit in one queue record (900 kB with its task by default, `RemoteWorker::with_max_result_bytes`) is a final failure with a clear reason. Keep large data in a store and pass a reference.
- **Timeouts.** A remote step waits 50 s for its result by default, under the workflow's 60 s executor timeout. Raise both for longer work. When the wait runs out the step fails as a timeout; a retry resumes waiting for the same task.
- **Shared state needs a shared backend.** Without `with_state_backend` on both sides, a remote executor sees an empty state of its own.
- **Shared state in a backend you give is kept until you remove it.** `forget(&workflow_id, &task_id)`, on `NatsStateBackend` and `InMemoryStateBackend`, removes the state of one run; call it when the run is finished and will not be resumed. A workflow without `with_state_backend` keeps state in a private store and drops the state of each run when it ends.
- **Results are kept for an hour.** `DurableQueueConfig::result_ttl` is how long the record of a task and its result stay in the ledger, counted from its last change. A task that waits and runs for longer still completes, but keep the lifetime above the time your tasks take. After that the task is forgotten: a retry or resume that sends it again runs it again. The ledger must hold every task of that period within `max_result_bytes` (512 MiB by default, about 700 bytes per small task): when it is full, tasks can neither be submitted nor completed until records expire. Shorten the lifetime or enlarge the ledger for a high rate.
- **Waiting costs one subscription per process.** Every wait of a `DurableQueue` and its clones shares one subscription to the results, and reads the record again every two seconds in case that subscription is being re-established.
- **One group, one set of executors.** Any worker of a group may receive any of its tasks: workers of a group must host the same executors. Use several groups to give different executors to different machines.
- **One task at a time per `serve`.** Run several `serve` loops, or several workers, for more parallelism.

## What it does not do

- Choose a worker by load or capability: tasks go to whichever worker of the group asks first.
- Cancel a task that is already running when the workflow gives up.
- Send back the events a remote executor emits with `add_event`.
- Move the workflow engine itself: supersteps, routing and checkpoints stay in the workflow process.

## Try it across machines

The example [`remote_steps`](../crates/agentic-core/examples/remote_steps.rs) is a workflow whose second step runs on workers. Start a NATS server with JetStream (`nats-server -js`) where every machine can reach it, then, on each machine that should do work:

```sh
DASEIN_NATS_URL=nats://nats-host:4222 \
  cargo run -p dasein-agentic-core --example remote_steps -- worker demo alice
```

And, from any machine, the workflow with the number of inputs to send:

```sh
DASEIN_NATS_URL=nats://nats-host:4222 \
  cargo run -p dasein-agentic-core --example remote_steps -- workflow demo 8
```

Each output names the worker, the system and the process that produced it, and shows the shared state it read. `DASEIN_WORK_MS` sets how long a worker takes over one input, `DASEIN_WAIT_S` how long the workflow waits for one.

This was run with NATS 2.15.3 and one worker in a Linux virtual machine, and a second worker and the workflow on the macOS host:

| Scenario | Result |
|---|---|
| 8 inputs of one second, two workers | 4 on each worker, 8 outputs in input order in 4.0 s |
| The Linux worker killed with `kill -9` during a task | Its task was done by the other worker after the 5 s acknowledgement deadline; 6 outputs of 6 |
| NATS stopped for 3 s, then restarted | Both workers kept running and served the next run |

The two systems were on one physical machine: a slow or lossy network between distant machines has not been tried.

## Load and duration test

The example [`soak`](../crates/agentic-core/examples/soak.rs) runs a small workflow with a remote step over and over, from several tasks at once, for a given time. It checks every result (outputs in order, built from the shared state of their own run), and prints throughput, latencies, failures and its memory every ten seconds.

```sh
# Everything in one process:
cargo run --release -p dasein-agentic-core --example soak -- memory 600

# Queue, results and shared state on a NATS server:
DASEIN_NATS_URL=nats://127.0.0.1:4222 \
  cargo run --release -p dasein-agentic-core --example soak -- nats 600
```

Measured on an Apple M4, with 8 workflows at once, 4 workers and 4 remote inputs per run, the NATS 2.15.3 server on the same machine:

| Run | Workflow runs | Failed or wrong | Rate | Median / 99 % | Memory of the process |
|---|---|---|---|---|---|
| In memory, 10 min | 8 126 225 | 0 | 13 500 /s | 0.5 ms / 1.3 ms | 34 MiB at the end, flat |
| Through NATS, 10 min | 814 840 | 0 | 1 360 /s | 5.3 ms / 18 ms | 8 MiB, flat |
| Through NATS, 1 min, server killed for 3 s | 68 985 | 0 | 1 150 /s | 5.8 ms / 18 ms | 9 MiB |
| Through NATS, 90 s, server down for 40 s | 89 720 | 1 | 1 000 /s | 4.0 ms / 9 ms | 9 MiB |

Two things this showed about sizing:

- In the 10 minute run the result ledger (512 MiB, results kept 120 s for the test) was full after 100 s, at 7 400 tasks a second. Everything waited 25 s, until the first records expired, then went on at the rate the ledger allowed. Nothing failed, because the steps were given 120 s to wait. Size `max_result_bytes` and `result_ttl` for your rate.
- The NATS server used 1.2 GiB of memory by the end of that run, against 11 MiB at the start, and gave it back only as records expired. Plan the server's memory for the number of records kept.

- In the run with the server down for 40 s, the steps waiting for a result went on waiting and got it when the server was back. The one failure was a local step that writes shared state: it used up its four attempts during the outage. A step that must write state or submit a task while NATS is away fails once its retries are spent; give it more retries if it has to ride out long outages.

These are short runs on one machine: they say nothing about days of operation, a real network, or several workflow processes at once.

## Without NATS

`InMemoryTaskQueue` has the same contract inside one process, minus durability, for development and tests. `queue.worker(group)` is the source of one group, as with the durable queue; the queue itself is a source for the tasks of every group.

```rust
let queue = InMemoryTaskQueue::new();
let source = queue.worker("summarizers");
tokio::spawn(async move { worker.serve(&source, shutdown).await });
```
