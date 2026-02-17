//! Benchmark: Graph Architecture vs Linear Architecture
//!
//! Compares the performance overhead of the graph-based workflow system
//! against a simple linear executor chain.
//!
//! Run with:
//! ```bash
//! cargo bench --bench graph_vs_linear
//! ```
//!
//! Metrics compared:
//! - Superstep execution overhead
//! - Message routing latency
//! - Edge evaluation cost
//! - Fan-out/Fan-in parallelism gains
//! - Memory footprint per workflow

use async_trait::async_trait;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use dasein_agentic_core::distributed::graph::{
    Executor as GraphExecutor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind,
    ExecutorRegistry, Workflow, WorkflowBuilder, WorkflowConfig,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;

// ============================================================================
// MESSAGE TYPES
// ============================================================================

/// Simple message for benchmarking
#[derive(Debug, Clone, Serialize, Deserialize)]
struct BenchMessage {
    value: i64,
    transformations: u32,
}

impl BenchMessage {
    fn new(value: i64) -> Self {
        Self {
            value,
            transformations: 0,
        }
    }

    fn transform(&self) -> Self {
        Self {
            value: self.value * 2 + 1,
            transformations: self.transformations + 1,
        }
    }
}

// ============================================================================
// GRAPH EXECUTORS
// ============================================================================

/// Simple transformer executor for graph workflows
struct TransformerExecutor {
    id: ExecutorId,
    call_count: Arc<AtomicUsize>,
}

impl TransformerExecutor {
    fn new(name: &str, call_count: Arc<AtomicUsize>) -> Self {
        Self {
            id: ExecutorId::new(name),
            call_count,
        }
    }
}

#[async_trait]
impl GraphExecutor for TransformerExecutor {
    type Input = BenchMessage;
    type Message = BenchMessage;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        let output = input.transform();
        ctx.send_message(output).await?;
        Ok(())
    }
}

/// Final collector executor
struct CollectorExecutor {
    id: ExecutorId,
    call_count: Arc<AtomicUsize>,
}

impl CollectorExecutor {
    fn new(call_count: Arc<AtomicUsize>) -> Self {
        Self {
            id: ExecutorId::new("collector"),
            call_count,
        }
    }
}

#[async_trait]
impl GraphExecutor for CollectorExecutor {
    type Input = BenchMessage;
    type Message = BenchMessage;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        self.call_count.fetch_add(1, Ordering::Relaxed);
        ctx.yield_output(format!(
            "Result: {} (transforms: {})",
            input.value, input.transformations
        ))
        .await?;
        Ok(())
    }
}

// ============================================================================
// LINEAR IMPLEMENTATION (baseline)
// ============================================================================

/// Simple linear executor chain (no graph overhead)
struct LinearPipeline {
    stages: usize,
    call_count: Arc<AtomicUsize>,
}

impl LinearPipeline {
    fn new(stages: usize, call_count: Arc<AtomicUsize>) -> Self {
        Self { stages, call_count }
    }

    async fn run(&self, input: BenchMessage) -> BenchMessage {
        let mut current = input;
        for _ in 0..self.stages {
            self.call_count.fetch_add(1, Ordering::Relaxed);
            current = current.transform();
        }
        self.call_count.fetch_add(1, Ordering::Relaxed); // collector
        current
    }
}

// ============================================================================
// BENCHMARK HELPERS
// ============================================================================

/// Build a graph workflow with N sequential stages
fn build_sequential_graph(
    stages: usize,
    call_count: Arc<AtomicUsize>,
) -> Workflow<BenchMessage, String> {
    let mut builder = WorkflowBuilder::<BenchMessage>::new("bench-sequential")
        .name("Sequential Benchmark")
        .set_start("stage-0")
        .add_executor_id("collector");

    // Add stage executor IDs
    for i in 1..stages {
        builder = builder.add_executor_id(format!("stage-{}", i));
    }

    // Add edges: stage-0 → stage-1 → ... → stage-N → collector
    for i in 0..stages - 1 {
        builder = builder.add_direct_edge(format!("stage-{}", i), format!("stage-{}", i + 1));
    }
    builder = builder.add_direct_edge(format!("stage-{}", stages - 1), "collector");

    let definition = builder.build().expect("Failed to build workflow");

    // Register executors
    let mut registry: ExecutorRegistry<BenchMessage, String> = ExecutorRegistry::new();
    for i in 0..stages {
        registry.register(TransformerExecutor::new(
            &format!("stage-{}", i),
            call_count.clone(),
        ));
    }
    registry.register(CollectorExecutor::new(call_count.clone()));

    let config = WorkflowConfig::new()
        .with_max_supersteps(stages as u32 + 5)
        .with_max_retries(0);

    Workflow::with_config(definition, registry, config)
}

/// Build a graph workflow with fan-out/fan-in pattern
fn build_fanout_graph(
    parallel_branches: usize,
    call_count: Arc<AtomicUsize>,
) -> Workflow<BenchMessage, String> {
    let mut builder = WorkflowBuilder::<BenchMessage>::new("bench-fanout")
        .name("Fan-out Benchmark")
        .set_start("splitter")
        .add_executor_id("collector");

    // Add branch executor IDs
    for i in 0..parallel_branches {
        builder = builder.add_executor_id(format!("branch-{}", i));
    }

    // Splitter → all branches (fan-out)
    for i in 0..parallel_branches {
        builder = builder.add_direct_edge("splitter", format!("branch-{}", i));
    }

    // All branches → collector (fan-in)
    for i in 0..parallel_branches {
        builder = builder.add_direct_edge(format!("branch-{}", i), "collector");
    }

    let definition = builder.build().expect("Failed to build workflow");

    // Register executors
    let mut registry: ExecutorRegistry<BenchMessage, String> = ExecutorRegistry::new();
    registry.register(TransformerExecutor::new("splitter", call_count.clone()));
    for i in 0..parallel_branches {
        registry.register(TransformerExecutor::new(
            &format!("branch-{}", i),
            call_count.clone(),
        ));
    }
    registry.register(CollectorExecutor::new(call_count.clone()));

    let config = WorkflowConfig::new()
        .with_max_supersteps(10)
        .with_max_retries(0);

    Workflow::with_config(definition, registry, config)
}

// ============================================================================
// BENCHMARKS
// ============================================================================

fn bench_sequential(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("sequential_pipeline");
    group.measurement_time(Duration::from_secs(10));

    for stages in [3, 5, 10, 20].iter() {
        group.throughput(Throughput::Elements(1));

        // Linear baseline
        let linear_count = Arc::new(AtomicUsize::new(0));
        let linear = LinearPipeline::new(*stages, linear_count.clone());

        group.bench_with_input(BenchmarkId::new("linear", stages), stages, |b, _| {
            b.to_async(&rt).iter(|| async {
                let result = linear.run(BenchMessage::new(1)).await;
                black_box(result)
            });
        });

        // Graph workflow
        let graph_count = Arc::new(AtomicUsize::new(0));
        let workflow = build_sequential_graph(*stages, graph_count.clone());

        group.bench_with_input(BenchmarkId::new("graph", stages), stages, |b, _| {
            b.to_async(&rt).iter(|| async {
                let result = workflow.run(BenchMessage::new(1)).await;
                black_box(result)
            });
        });
    }

    group.finish();
}

fn bench_fanout(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("fanout_pattern");
    group.measurement_time(Duration::from_secs(10));

    for branches in [2, 4, 8, 16].iter() {
        group.throughput(Throughput::Elements(*branches as u64));

        // Linear simulation (sequential processing of all branches)
        let linear_count = Arc::new(AtomicUsize::new(0));
        let linear = LinearPipeline::new(*branches + 1, linear_count.clone()); // +1 for splitter

        group.bench_with_input(
            BenchmarkId::new("linear_sequential", branches),
            branches,
            |b, _| {
                b.to_async(&rt).iter(|| async {
                    let result = linear.run(BenchMessage::new(1)).await;
                    black_box(result)
                });
            },
        );

        // Graph workflow with fan-out (parallel execution)
        let graph_count = Arc::new(AtomicUsize::new(0));
        let workflow = build_fanout_graph(*branches, graph_count.clone());

        group.bench_with_input(
            BenchmarkId::new("graph_parallel", branches),
            branches,
            |b, _| {
                b.to_async(&rt).iter(|| async {
                    let result = workflow.run(BenchMessage::new(1)).await;
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

fn bench_superstep_overhead(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("superstep_overhead");
    group.measurement_time(Duration::from_secs(5));

    // Measure overhead per superstep with minimal work
    for stages in [1, 2, 5, 10].iter() {
        let graph_count = Arc::new(AtomicUsize::new(0));
        let workflow = build_sequential_graph(*stages, graph_count.clone());

        group.bench_with_input(BenchmarkId::new("supersteps", stages), stages, |b, _| {
            b.to_async(&rt).iter(|| async {
                let result = workflow.run(BenchMessage::new(1)).await;
                black_box(result)
            });
        });
    }

    group.finish();
}

fn bench_message_throughput(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("message_throughput");
    group.measurement_time(Duration::from_secs(10));

    // Test with different message sizes (simulated by transformation count)
    let stages = 5;

    for iterations in [10, 100, 1000].iter() {
        group.throughput(Throughput::Elements(*iterations as u64));

        // Linear
        let linear_count = Arc::new(AtomicUsize::new(0));
        let linear = LinearPipeline::new(stages, linear_count.clone());

        group.bench_with_input(
            BenchmarkId::new("linear", iterations),
            iterations,
            |b, iters| {
                b.to_async(&rt).iter(|| async {
                    for i in 0..*iters {
                        let result = linear.run(BenchMessage::new(i as i64)).await;
                        black_box(result);
                    }
                });
            },
        );

        // Graph
        let graph_count = Arc::new(AtomicUsize::new(0));
        let workflow = build_sequential_graph(stages, graph_count.clone());

        group.bench_with_input(
            BenchmarkId::new("graph", iterations),
            iterations,
            |b, iters| {
                b.to_async(&rt).iter(|| async {
                    for i in 0..*iters {
                        let result = workflow.run(BenchMessage::new(i as i64)).await;
                        let _ = black_box(result);
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_edge_evaluation(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    let mut group = c.benchmark_group("edge_evaluation");
    group.measurement_time(Duration::from_secs(5));

    // Benchmark conditional edge evaluation overhead
    // Build a workflow with conditional edges
    let call_count = Arc::new(AtomicUsize::new(0));

    let mut builder = WorkflowBuilder::<BenchMessage>::new("bench-conditional")
        .name("Conditional Edge Benchmark")
        .set_start("router")
        .add_executor_id("path-a")
        .add_executor_id("path-b")
        .add_executor_id("collector");

    // Conditional routing based on value
    builder = builder.add_conditional_edge(
        "router",
        "path-a",
        |m: &BenchMessage| m.value % 2 == 0,
        "even",
    );
    builder = builder.add_conditional_edge(
        "router",
        "path-b",
        |m: &BenchMessage| m.value % 2 != 0,
        "odd",
    );
    builder = builder.add_direct_edge("path-a", "collector");
    builder = builder.add_direct_edge("path-b", "collector");

    let definition = builder.build().expect("Failed to build workflow");

    let mut registry: ExecutorRegistry<BenchMessage, String> = ExecutorRegistry::new();
    registry.register(TransformerExecutor::new("router", call_count.clone()));
    registry.register(TransformerExecutor::new("path-a", call_count.clone()));
    registry.register(TransformerExecutor::new("path-b", call_count.clone()));
    registry.register(CollectorExecutor::new(call_count.clone()));

    let config = WorkflowConfig::new()
        .with_max_supersteps(10)
        .with_max_retries(0);

    let workflow = Workflow::with_config(definition, registry, config);

    for value in [0i64, 1, 100, 999].iter() {
        group.bench_with_input(
            BenchmarkId::new("conditional_routing", value),
            value,
            |b, v| {
                b.to_async(&rt).iter(|| async {
                    let result = workflow.run(BenchMessage::new(*v)).await;
                    black_box(result)
                });
            },
        );
    }

    group.finish();
}

// ============================================================================
// CRITERION MAIN
// ============================================================================

criterion_group!(
    benches,
    bench_sequential,
    bench_fanout,
    bench_superstep_overhead,
    bench_message_throughput,
    bench_edge_evaluation,
);

criterion_main!(benches);
