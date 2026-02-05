//! Patterns Demo - Demonstrates the 4 orchestration patterns.
//!
//! This example shows how to use the high-level pattern builders
//! from `agentic_core::patterns` module.
//!
//! # Patterns Demonstrated
//!
//! 1. **Sequential** - Linear pipeline: A → B → C
//! 2. **Concurrent** - Fan-out/fan-in: splitter → [A, B, C] → aggregator
//! 3. **GroupChat** - Star topology with orchestrator selecting speakers
//! 4. **Handoff** - Dynamic mesh for specialist routing
//!
//! # Run
//!
//! ```bash
//! cargo run --example patterns_demo
//! ```

use agentic_core::distributed::graph::ExecutorId;
use agentic_core::patterns::{
    // Builders
    ConcurrentBuilder, GroupChatBuilder, HandoffBuilder, SequentialBuilder,
    // Convenience functions
    handoff, sequential,
    // Selectors
    round_robin_selector,
    // Termination conditions
    max_rounds_termination, max_messages_termination, keyword_termination, any_termination,
    // Types
    GroupChatState, HandoffDecision,
};

fn main() {
    println!("=== Agentic Patterns Demo ===\n");

    // Run all demos
    demo_sequential();
    demo_concurrent();
    demo_group_chat();
    demo_handoff();

    println!("\n=== All Patterns Demo Complete ===");
}

// ============================================================================
// PATTERN 1: SEQUENTIAL
// ============================================================================

fn demo_sequential() {
    println!("--- Pattern 1: Sequential Pipeline ---\n");

    // Method 1: Using the builder
    let definition = SequentialBuilder::<serde_json::Value>::new("content-pipeline")
        .name("Content Generation Pipeline")
        .add_participant("writer")
        .add_participant("reviewer")
        .add_participant("polisher")
        .build()
        .expect("Failed to build sequential workflow");

    println!("Built sequential workflow: {}", definition.id);
    println!("  Name: {:?}", definition.name);
    println!("  Start: {}", definition.start);
    println!("  Executors: {}", definition.executors.len());
    println!("  Edges: {}", definition.edges.all().len());
    println!("  Flow: writer → reviewer → polisher\n");

    // Method 2: Using convenience function
    let quick = sequential::<serde_json::Value>(
        "quick-pipeline",
        vec!["step1", "step2", "step3"],
    ).expect("Failed to build");

    println!("Quick sequential: {} with {} steps\n", quick.id, quick.executors.len());
}

// ============================================================================
// PATTERN 2: CONCURRENT
// ============================================================================

fn demo_concurrent() {
    println!("--- Pattern 2: Concurrent (Fan-out/Fan-in) ---\n");

    // Method 1: Using the builder
    let definition = ConcurrentBuilder::<serde_json::Value>::new("analysis")
        .name("Product Analysis")
        .add_participant("researcher")
        .add_participant("marketer")
        .add_participant("legal")
        .build()
        .expect("Failed to build concurrent workflow");

    println!("Built concurrent workflow: {}", definition.id);
    println!("  Name: {:?}", definition.name);
    println!("  Start: {} (splitter)", definition.start);
    println!("  Executors: {} (splitter + 3 workers + aggregator)", definition.executors.len());

    // Check topology
    let has_splitter = definition.has_executor(&ExecutorId::new("analysis-splitter"));
    let has_aggregator = definition.has_executor(&ExecutorId::new("analysis-aggregator"));
    println!("  Has splitter: {}", has_splitter);
    println!("  Has aggregator: {}", has_aggregator);
    println!("  Topology: splitter → [researcher, marketer, legal] → aggregator\n");

    // Method 2: Custom aggregator
    let custom = ConcurrentBuilder::<serde_json::Value>::new("custom")
        .add_participant("worker1")
        .add_participant("worker2")
        .with_aggregator("my-custom-aggregator")
        .build()
        .expect("Failed to build");

    println!("Custom aggregator: {}\n", custom.has_executor(&ExecutorId::new("my-custom-aggregator")));
}

// ============================================================================
// PATTERN 3: GROUP CHAT
// ============================================================================

fn demo_group_chat() {
    println!("--- Pattern 3: Group Chat (Star Topology) ---\n");

    // Build a code review group chat
    let definition = GroupChatBuilder::<serde_json::Value>::new("code-review")
        .name("Code Review Discussion")
        .add_participant("developer")
        .add_participant("reviewer")
        .add_participant("tester")
        .with_orchestrator_func(round_robin_selector)
        .with_termination_condition(max_rounds_termination(10))
        .with_max_iterations(20) // Safety limit
        .build()
        .expect("Failed to build group chat workflow");

    println!("Built group chat workflow: {}", definition.id);
    println!("  Name: {:?}", definition.name);
    println!("  Start: {} (orchestrator)", definition.start);
    println!("  Executors: {} (orchestrator + collector + 3 participants)", definition.executors.len());

    // Check components
    let has_orchestrator = definition.has_executor(&ExecutorId::new("code-review-orchestrator"));
    let has_collector = definition.has_executor(&ExecutorId::new("code-review-collector"));
    println!("  Has orchestrator: {}", has_orchestrator);
    println!("  Has collector: {}", has_collector);

    // Demo selectors
    println!("\n  Selector Demo:");
    let state = GroupChatState::new(vec![
        ExecutorId::new("alice"),
        ExecutorId::new("bob"),
        ExecutorId::new("charlie"),
    ]);

    let selected = round_robin_selector(&state);
    println!("    Round 0 (round_robin): {}", selected);

    // Demo termination conditions
    println!("\n  Termination Conditions:");
    let max_msgs = max_messages_termination(5);
    let max_rounds = max_rounds_termination(3);
    let keyword = keyword_termination("approved");
    let combined = any_termination(vec![
        max_messages_termination(10),
        keyword_termination("done"),
    ]);

    println!("    max_messages(5): {}", max_msgs(&state));
    println!("    max_rounds(3): {}", max_rounds(&state));
    println!("    keyword('approved'): {}", keyword(&state));
    println!("    combined: {}\n", combined(&state));
}

// ============================================================================
// PATTERN 4: HANDOFF
// ============================================================================

fn demo_handoff() {
    println!("--- Pattern 4: Handoff (Dynamic Mesh) ---\n");

    // Method 1: Using the builder
    let definition = HandoffBuilder::<serde_json::Value>::new("customer-support")
        .name("Customer Support Mesh")
        .add_participant("general_agent")
        .add_participant("billing_specialist")
        .add_participant("technical_specialist")
        .start("general_agent") // Start with general agent
        .build()
        .expect("Failed to build handoff workflow");

    println!("Built handoff workflow: {}", definition.id);
    println!("  Name: {:?}", definition.name);
    println!("  Start: {}", definition.start);
    println!("  Executors: {}", definition.executors.len());
    println!("  Edges: {} (mesh: each can route to any other)", definition.edges.all().len());

    // Method 2: Convenience function
    let quick = handoff::<serde_json::Value>(
        "quick-support",
        vec!["general", "code_expert", "math_expert"],
    ).expect("Failed to build");

    println!("\n  Quick handoff: {} executors, {} edges",
        quick.executors.len(),
        quick.edges.all().len()
    );

    // Demo HandoffDecision
    println!("\n  HandoffDecision Demo:");
    let handle = HandoffDecision::Handle;
    let transfer = HandoffDecision::HandoffTo(ExecutorId::new("specialist"));
    let done = HandoffDecision::Terminate;

    println!("    Handle.is_handoff(): {}", handle.is_handoff());
    println!("    HandoffTo('specialist').is_handoff(): {}", transfer.is_handoff());
    println!("    HandoffTo('specialist').target(): {:?}", transfer.handoff_target().map(|id| id.as_str()));
    println!("    Terminate.is_terminate(): {}", done.is_terminate());

    // Demo HandoffCapable trait
    println!("\n  HandoffCapable Trait:");
    println!("    Implement this trait on your executors to enable content-based routing.");
    println!("    Methods:");
    println!("      - executor_id() -> &ExecutorId");
    println!("      - can_handle(&str) -> bool");
    println!("      - suggest_handoff(&str) -> Option<ExecutorId>");
    println!();
}
