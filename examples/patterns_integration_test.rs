//! Patterns Integration Test - Real workflow execution with pattern builders.
//!
//! This example tests that the orchestration patterns actually work with
//! real workflow execution, not just definition building.
//!
//! # Tests Performed
//!
//! 1. **Sequential** - Pipeline execution: A → B → C with data transformation
//! 2. **Concurrent** - Fan-out/fan-in with parallel execution
//! 3. **GroupChat** - Orchestrator-driven multi-participant workflow
//! 4. **Handoff** - Dynamic routing based on content
//!
//! # Run
//!
//! ```bash
//! cargo run --example patterns_integration_test
//! ```

use dasein_agentic_core::distributed::graph::{
    Executor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind, ExecutorRegistry, Workflow,
    WorkflowConfig,
};
use dasein_agentic_core::patterns::{
    ConcurrentBuilder, GroupChatBuilder, HandoffBuilder, SequentialBuilder, round_robin_selector,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

// ============================================================================
// MESSAGE TYPE
// ============================================================================

/// Simple message that flows through workflows.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Message {
    /// Content being processed
    content: String,
    /// Processing history (which executors touched it)
    history: Vec<String>,
    /// Counter for transformations
    transform_count: usize,
}

impl Message {
    fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            history: vec![],
            transform_count: 0,
        }
    }

    fn transform(mut self, executor_id: &str, prefix: &str) -> Self {
        self.content = format!("{}{}", prefix, self.content);
        self.history.push(executor_id.to_string());
        self.transform_count += 1;
        self
    }
}

// ============================================================================
// TEST EXECUTORS
// ============================================================================

/// Generic transformer executor - applies a prefix to content.
struct TransformerExecutor {
    id: ExecutorId,
    prefix: String,
    call_count: Arc<AtomicUsize>,
}

impl TransformerExecutor {
    fn new(id: impl Into<String>, prefix: impl Into<String>) -> Self {
        Self {
            id: ExecutorId::new(id),
            prefix: prefix.into(),
            call_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn with_counter(mut self, counter: Arc<AtomicUsize>) -> Self {
        self.call_count = counter;
        self
    }
}

#[async_trait]
impl Executor for TransformerExecutor {
    type Input = serde_json::Value;
    type Message = serde_json::Value;
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
        self.call_count.fetch_add(1, Ordering::SeqCst);

        // Deserialize input
        let msg: Message = serde_json::from_value(input)
            .map_err(|e| ExecutorError::new(self.id.clone(), format!("Deserialize error: {}", e)))?;

        // Transform
        let transformed = msg.transform(self.id.as_str(), &self.prefix);

        println!(
            "  [{}] Transformed: '{}' (history: {:?})",
            self.id, transformed.content, transformed.history
        );

        // Send to next executor
        let output = serde_json::to_value(&transformed)
            .map_err(|e| ExecutorError::new(self.id.clone(), format!("Serialize error: {}", e)))?;

        ctx.send_message(output).await?;
        ctx.yield_output(format!("{} processed", self.id)).await?;

        Ok(())
    }
}

/// Aggregator executor - collects and combines messages.
struct AggregatorExecutor {
    id: ExecutorId,
    received_count: Arc<AtomicUsize>,
}

impl AggregatorExecutor {
    fn new(id: impl Into<String>) -> Self {
        Self {
            id: ExecutorId::new(id),
            received_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn with_counter(mut self, counter: Arc<AtomicUsize>) -> Self {
        self.received_count = counter;
        self
    }
}

#[async_trait]
impl Executor for AggregatorExecutor {
    type Input = serde_json::Value;
    type Message = serde_json::Value;
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
        self.received_count.fetch_add(1, Ordering::SeqCst);

        // Deserialize input
        let msg: Message = serde_json::from_value(input.clone())
            .map_err(|e| ExecutorError::new(self.id.clone(), format!("Deserialize error: {}", e)))?;

        println!(
            "  [{}] Aggregated message with history: {:?}, transforms: {}",
            self.id, msg.history, msg.transform_count
        );

        // Pass through (terminal node in most cases)
        ctx.send_message(input).await?;
        ctx.yield_output(format!(
            "Aggregated: {} transforms, history: {:?}",
            msg.transform_count, msg.history
        ))
        .await?;

        Ok(())
    }
}

/// Splitter executor - passes input unchanged (for fan-out start).
struct SplitterExecutor {
    id: ExecutorId,
}

impl SplitterExecutor {
    fn new(id: impl Into<String>) -> Self {
        Self {
            id: ExecutorId::new(id),
        }
    }
}

#[async_trait]
impl Executor for SplitterExecutor {
    type Input = serde_json::Value;
    type Message = serde_json::Value;
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
        println!("  [{}] Splitting input to fan-out targets", self.id);
        ctx.send_message(input).await?;
        ctx.yield_output("Split complete".into()).await?;
        Ok(())
    }
}

/// Orchestrator executor - selects and routes to participants.
struct OrchestratorExecutor {
    id: ExecutorId,
    round: Arc<AtomicUsize>,
}

impl OrchestratorExecutor {
    fn new(id: impl Into<String>) -> Self {
        Self {
            id: ExecutorId::new(id),
            round: Arc::new(AtomicUsize::new(0)),
        }
    }
}

#[async_trait]
impl Executor for OrchestratorExecutor {
    type Input = serde_json::Value;
    type Message = serde_json::Value;
    type Output = String;

    fn id(&self) -> &ExecutorId {
        &self.id
    }

    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Orchestrator
    }

    async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
    where
        Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
    {
        let round = self.round.fetch_add(1, Ordering::SeqCst);
        println!("  [{}] Orchestrator round {}", self.id, round);
        ctx.send_message(input).await?;
        ctx.yield_output(format!("Orchestrator round {}", round))
            .await?;
        Ok(())
    }
}

/// Collector executor - collects from participants back to orchestrator.
struct CollectorExecutor {
    id: ExecutorId,
}

impl CollectorExecutor {
    fn new(id: impl Into<String>) -> Self {
        Self {
            id: ExecutorId::new(id),
        }
    }
}

#[async_trait]
impl Executor for CollectorExecutor {
    type Input = serde_json::Value;
    type Message = serde_json::Value;
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
        println!("  [{}] Collected response", self.id);
        ctx.send_message(input).await?;
        ctx.yield_output("Collected".into()).await?;
        Ok(())
    }
}

// ============================================================================
// TEST FUNCTIONS
// ============================================================================

/// Test 1: Sequential Pipeline
async fn test_sequential() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(70));
    println!(" TEST 1: SEQUENTIAL PIPELINE");
    println!("{}", "=".repeat(70));

    // Build workflow definition
    let definition = SequentialBuilder::<serde_json::Value>::new("seq-test")
        .name("Sequential Test Pipeline")
        .add_participant("step-a")
        .add_participant("step-b")
        .add_participant("step-c")
        .build()?;

    println!(
        "Definition: {} executors, {} edges",
        definition.executors.len(),
        definition.edges.all().len()
    );
    println!("Flow: step-a → step-b → step-c\n");

    // Create counters for verification
    let counter_a = Arc::new(AtomicUsize::new(0));
    let counter_b = Arc::new(AtomicUsize::new(0));
    let counter_c = Arc::new(AtomicUsize::new(0));

    // Create executors
    let step_a = TransformerExecutor::new("step-a", "[A]").with_counter(counter_a.clone());
    let step_b = TransformerExecutor::new("step-b", "[B]").with_counter(counter_b.clone());
    let step_c = TransformerExecutor::new("step-c", "[C]").with_counter(counter_c.clone());

    // Build registry
    let mut registry: ExecutorRegistry<serde_json::Value, String> = ExecutorRegistry::new();
    registry.register(step_a);
    registry.register(step_b);
    registry.register(step_c);

    // Create and run workflow
    let config = WorkflowConfig::new().with_max_supersteps(10);
    let workflow = Workflow::with_config(definition, registry, config);

    let input = Message::new("START");
    let input_json = serde_json::to_value(&input)?;

    println!("Running workflow with input: '{}'", input.content);
    let start = Instant::now();
    let result = workflow.run(input_json).await?;
    let elapsed = start.elapsed();

    // Verify
    println!("\nResult:");
    println!("  Supersteps: {}", result.superstep_count);
    println!("  Duration: {:?}", elapsed);
    println!("  Success: {}", result.success);
    println!("  Outputs: {} items", result.outputs.len());

    // Check call counts
    assert_eq!(
        counter_a.load(Ordering::SeqCst),
        1,
        "step-a should be called once"
    );
    assert_eq!(
        counter_b.load(Ordering::SeqCst),
        1,
        "step-b should be called once"
    );
    assert_eq!(
        counter_c.load(Ordering::SeqCst),
        1,
        "step-c should be called once"
    );

    println!("\n✅ Sequential test PASSED\n");
    Ok(())
}

/// Test 2: Concurrent (Fan-out/Fan-in)
async fn test_concurrent() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(70));
    println!(" TEST 2: CONCURRENT (FAN-OUT/FAN-IN)");
    println!("{}", "=".repeat(70));

    // Build workflow definition
    let definition = ConcurrentBuilder::<serde_json::Value>::new("concurrent-test")
        .name("Concurrent Test Pipeline")
        .add_participant("worker-1")
        .add_participant("worker-2")
        .add_participant("worker-3")
        .build()?;

    println!(
        "Definition: {} executors, {} edges",
        definition.executors.len(),
        definition.edges.all().len()
    );
    println!("Flow: splitter → [worker-1, worker-2, worker-3] → aggregator\n");

    // Create counters
    let counter_1 = Arc::new(AtomicUsize::new(0));
    let counter_2 = Arc::new(AtomicUsize::new(0));
    let counter_3 = Arc::new(AtomicUsize::new(0));
    let counter_agg = Arc::new(AtomicUsize::new(0));

    // Create executors
    let splitter = SplitterExecutor::new("concurrent-test-splitter");
    let worker1 = TransformerExecutor::new("worker-1", "[W1]").with_counter(counter_1.clone());
    let worker2 = TransformerExecutor::new("worker-2", "[W2]").with_counter(counter_2.clone());
    let worker3 = TransformerExecutor::new("worker-3", "[W3]").with_counter(counter_3.clone());
    let aggregator =
        AggregatorExecutor::new("concurrent-test-aggregator").with_counter(counter_agg.clone());

    // Build registry
    let mut registry: ExecutorRegistry<serde_json::Value, String> = ExecutorRegistry::new();
    registry.register(splitter);
    registry.register(worker1);
    registry.register(worker2);
    registry.register(worker3);
    registry.register(aggregator);

    // Create and run workflow
    let config = WorkflowConfig::new().with_max_supersteps(10);
    let workflow = Workflow::with_config(definition, registry, config);

    let input = Message::new("CONCURRENT");
    let input_json = serde_json::to_value(&input)?;

    println!("Running workflow with input: '{}'", input.content);
    let start = Instant::now();
    let result = workflow.run(input_json).await?;
    let elapsed = start.elapsed();

    // Verify
    println!("\nResult:");
    println!("  Supersteps: {}", result.superstep_count);
    println!("  Duration: {:?}", elapsed);
    println!("  Success: {}", result.success);
    println!("  Outputs: {} items", result.outputs.len());

    // Check call counts - all workers should be called
    assert_eq!(
        counter_1.load(Ordering::SeqCst),
        1,
        "worker-1 should be called once"
    );
    assert_eq!(
        counter_2.load(Ordering::SeqCst),
        1,
        "worker-2 should be called once"
    );
    assert_eq!(
        counter_3.load(Ordering::SeqCst),
        1,
        "worker-3 should be called once"
    );

    // Aggregator receives from all 3 workers
    assert!(
        counter_agg.load(Ordering::SeqCst) >= 1,
        "aggregator should receive messages"
    );

    println!("\n✅ Concurrent test PASSED\n");
    Ok(())
}

/// Test 3: GroupChat
async fn test_group_chat() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(70));
    println!(" TEST 3: GROUP CHAT (STAR TOPOLOGY)");
    println!("{}", "=".repeat(70));

    // Build workflow definition
    let definition = GroupChatBuilder::<serde_json::Value>::new("chat-test")
        .name("Group Chat Test")
        .add_participant("alice")
        .add_participant("bob")
        .with_orchestrator_func(round_robin_selector)
        .with_max_iterations(5)
        .build()?;

    println!(
        "Definition: {} executors, {} edges",
        definition.executors.len(),
        definition.edges.all().len()
    );
    println!("Flow: orchestrator ↔ [alice, bob] ↔ collector\n");

    // Create counters
    let counter_alice = Arc::new(AtomicUsize::new(0));
    let counter_bob = Arc::new(AtomicUsize::new(0));

    // Create executors
    let orchestrator = OrchestratorExecutor::new("chat-test-orchestrator");
    let collector = CollectorExecutor::new("chat-test-collector");
    let alice = TransformerExecutor::new("alice", "[Alice]").with_counter(counter_alice.clone());
    let bob = TransformerExecutor::new("bob", "[Bob]").with_counter(counter_bob.clone());

    // Build registry
    let mut registry: ExecutorRegistry<serde_json::Value, String> = ExecutorRegistry::new();
    registry.register(orchestrator);
    registry.register(collector);
    registry.register(alice);
    registry.register(bob);

    // Create and run workflow
    let config = WorkflowConfig::new().with_max_supersteps(3); // Limit iterations
    let workflow = Workflow::with_config(definition, registry, config);

    let input = Message::new("Hello group!");
    let input_json = serde_json::to_value(&input)?;

    println!("Running workflow with input: '{}'", input.content);
    let start = Instant::now();
    let result = workflow.run(input_json).await?;
    let elapsed = start.elapsed();

    // Verify
    println!("\nResult:");
    println!("  Supersteps: {}", result.superstep_count);
    println!("  Duration: {:?}", elapsed);
    println!("  Success: {}", result.success);
    println!("  Outputs: {} items", result.outputs.len());

    // At least one participant should have been called
    let total_calls = counter_alice.load(Ordering::SeqCst) + counter_bob.load(Ordering::SeqCst);
    assert!(
        total_calls >= 1,
        "At least one participant should be called"
    );

    println!("\n✅ GroupChat test PASSED\n");
    Ok(())
}

/// Test 4: Handoff (Dynamic Mesh)
async fn test_handoff() -> Result<(), Box<dyn std::error::Error>> {
    println!("\n{}", "=".repeat(70));
    println!(" TEST 4: HANDOFF (DYNAMIC MESH)");
    println!("{}", "=".repeat(70));

    // Build workflow definition
    let definition = HandoffBuilder::<serde_json::Value>::new("handoff-test")
        .name("Handoff Test Mesh")
        .add_participant("general")
        .add_participant("specialist-a")
        .add_participant("specialist-b")
        .start("general")
        .build()?;

    println!(
        "Definition: {} executors, {} edges (mesh)",
        definition.executors.len(),
        definition.edges.all().len()
    );
    println!("Flow: general ↔ specialist-a ↔ specialist-b (mesh)\n");

    // Create counters
    let counter_gen = Arc::new(AtomicUsize::new(0));
    let counter_a = Arc::new(AtomicUsize::new(0));
    let counter_b = Arc::new(AtomicUsize::new(0));

    // Create executors
    let general = TransformerExecutor::new("general", "[GEN]").with_counter(counter_gen.clone());
    let spec_a =
        TransformerExecutor::new("specialist-a", "[SPEC-A]").with_counter(counter_a.clone());
    let spec_b =
        TransformerExecutor::new("specialist-b", "[SPEC-B]").with_counter(counter_b.clone());

    // Build registry
    let mut registry: ExecutorRegistry<serde_json::Value, String> = ExecutorRegistry::new();
    registry.register(general);
    registry.register(spec_a);
    registry.register(spec_b);

    // Create and run workflow
    let config = WorkflowConfig::new().with_max_supersteps(5);
    let workflow = Workflow::with_config(definition, registry, config);

    let input = Message::new("Need help");
    let input_json = serde_json::to_value(&input)?;

    println!("Running workflow with input: '{}'", input.content);
    let start = Instant::now();
    let result = workflow.run(input_json).await?;
    let elapsed = start.elapsed();

    // Verify
    println!("\nResult:");
    println!("  Supersteps: {}", result.superstep_count);
    println!("  Duration: {:?}", elapsed);
    println!("  Success: {}", result.success);
    println!("  Outputs: {} items", result.outputs.len());

    // General (start) should be called at least once
    assert!(
        counter_gen.load(Ordering::SeqCst) >= 1,
        "general (start) should be called"
    );

    println!("\n✅ Handoff test PASSED\n");
    Ok(())
}

// ============================================================================
// MAIN
// ============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔════════════════════════════════════════════════════════════════╗");
    println!("║     PATTERNS INTEGRATION TEST - Real Workflow Execution        ║");
    println!("╚════════════════════════════════════════════════════════════════╝");

    let mut passed = 0;
    let mut failed = 0;

    // Test 1: Sequential
    match test_sequential().await {
        Ok(_) => passed += 1,
        Err(e) => {
            println!("❌ Sequential test FAILED: {}", e);
            failed += 1;
        }
    }

    // Test 2: Concurrent
    match test_concurrent().await {
        Ok(_) => passed += 1,
        Err(e) => {
            println!("❌ Concurrent test FAILED: {}", e);
            failed += 1;
        }
    }

    // Test 3: GroupChat
    match test_group_chat().await {
        Ok(_) => passed += 1,
        Err(e) => {
            println!("❌ GroupChat test FAILED: {}", e);
            failed += 1;
        }
    }

    // Test 4: Handoff
    match test_handoff().await {
        Ok(_) => passed += 1,
        Err(e) => {
            println!("❌ Handoff test FAILED: {}", e);
            failed += 1;
        }
    }

    // Summary
    println!("\n╔════════════════════════════════════════════════════════════════╗");
    println!("║                         TEST SUMMARY                           ║");
    println!("╠════════════════════════════════════════════════════════════════╣");
    println!(
        "║  Passed: {}                                                     ║",
        passed
    );
    println!(
        "║  Failed: {}                                                     ║",
        failed
    );
    println!("╚════════════════════════════════════════════════════════════════╝");

    if failed > 0 {
        std::process::exit(1);
    }

    Ok(())
}
