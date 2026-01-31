//! NATS Bus Demo - Real NATS integration for distributed agents.
//!
//! This example demonstrates:
//! - Connecting to NATS with JetStream
//! - Task sequencing with priorities
//! - Proposal arbitrage (best-of-N)
//! - Deduplication
//! - Centralized logging
//!
//! Prerequisites:
//! ```bash
//! # Start NATS with JetStream
//! docker run -d --name nats -p 4222:4222 -p 8222:8222 nats:latest -js
//!
//! # Verify it's running
//! docker logs nats
//! ```
//!
//! Run with: cargo run --example nats_bus_demo

use agentic_core::distributed::bus::{
    BusCoordinator, Proposal, Task, TaskPriority,
};
use futures::StreamExt;
use serde_json::json;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();

    println!("\n{}", "=".repeat(70));
    println!("  NATS BUS COORDINATOR DEMO");
    println!("  Real NATS JetStream Integration");
    println!("{}\n", "=".repeat(70));

    // === Connect to NATS ===
    println!("[1] Connecting to NATS...\n");

    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());

    let coordinator = match BusCoordinator::builder()
        .nats_url(&nats_url)
        .build_and_start()
        .await
    {
        Ok(c) => {
            println!("     Connected to NATS at {}", nats_url);
            println!("     JetStream streams created");
            c
        }
        Err(e) => {
            eprintln!("\n     ERROR: Could not connect to NATS: {}", e);
            eprintln!("     Make sure NATS is running:");
            eprintln!("       docker run -d --name nats -p 4222:4222 nats:latest -js\n");
            return Err(e.into());
        }
    };

    // === Demo 1: Task Sequencing with Priorities ===
    println!("\n[2] Task Sequencing Demo...\n");

    // Publish tasks with different priorities
    let tasks = vec![
        ("task-low", TaskPriority::Low, "Low priority task"),
        ("task-normal", TaskPriority::Normal, "Normal priority task"),
        ("task-high", TaskPriority::High, "High priority task"),
        ("task-critical", TaskPriority::Critical, "Critical task!"),
    ];

    for (id, priority, desc) in &tasks {
        let task = Task::new(*id, "supervisor-demo", json!({ "description": desc }))
            .with_priority(*priority);

        coordinator.publish_task(task).await?;
        println!("     Published: {} ({:?})", id, priority);
    }

    // Check stats
    let stats = coordinator.stats().await;
    println!("\n     Stats: {} pending, {} completed", stats.tasks_pending, stats.tasks_completed);

    // === Demo 2: Deduplication ===
    println!("\n[3] Deduplication Demo...\n");

    // Try to publish duplicate task
    let dup_task = Task::new("task-dup", "supervisor-demo", json!({ "data": "same_content" }));
    coordinator.publish_task(dup_task.clone()).await?;
    println!("     Published: task-dup (first time)");

    // This should be deduplicated
    coordinator.publish_task(dup_task).await?;
    println!("     Published: task-dup (second time - should be deduplicated)");

    let stats = coordinator.stats().await;
    println!("\n     Duplicates prevented: {}", stats.duplicates_prevented);

    // === Demo 3: Proposal Arbitrage ===
    println!("\n[4] Proposal Arbitrage Demo...\n");

    // Request proposals
    let request_id = coordinator
        .request_proposals("Generate a fibonacci function", 3)
        .await?;
    println!("     Requested proposals: {}", request_id);

    // Simulate executors submitting proposals
    let proposals = vec![
        ("exe-1", 85.0, 0.9, 150),
        ("exe-2", 92.0, 0.85, 200),
        ("exe-3", 78.0, 0.95, 100),
    ];

    for (exe_id, quality, confidence, latency) in proposals {
        let proposal = Proposal::new(&request_id, exe_id, format!("Code from {}", exe_id))
            .with_scores(quality, confidence)
            .with_latency(latency);

        coordinator.submit_proposal(proposal).await?;
        println!(
            "     Proposal from {}: quality={}, confidence={}, latency={}ms",
            exe_id, quality, confidence, latency
        );
    }

    // Select best proposal
    tokio::time::sleep(Duration::from_millis(100)).await;
    let best = coordinator.select_best_proposal(&request_id).await?;
    println!(
        "\n     Best proposal: {} (score: {:.2})",
        best.proposal.executor_id, best.final_score
    );

    // === Demo 4: Logging ===
    println!("\n[5] Centralized Logging Demo...\n");

    // Log some events
    coordinator.log_info("executor-1", "Task received").await?;
    coordinator.log_info("executor-1", "Processing started").await?;
    coordinator
        .log_info("executor-1", "Task completed successfully")
        .await?;
    coordinator
        .log_error("executor-2", "Connection timeout")
        .await?;

    // Query logs
    let logs = coordinator
        .query_logs(agentic_core::distributed::bus::LogQuery::for_agent("executor-1"))
        .await;
    println!("     Logs from executor-1: {} entries", logs.len());
    for log in &logs {
        println!("       [{}] {}", log.level.as_str().to_uppercase(), log.message);
    }

    let stats = coordinator.stats().await;
    println!("\n     Total logs collected: {}", stats.logs_collected);

    // === Demo 5: Subscribe to Events ===
    println!("\n[6] Real-time Subscription Demo...\n");

    // Subscribe to audit events
    let mut audit_sub = coordinator.subscribe_audit().await?;

    // Publish an audit event
    coordinator
        .audit(
            "demo_event",
            json!({
                "message": "Demo completed",
                "duration_ms": 1234
            }),
        )
        .await?;

    // Wait for the event
    if let Ok(Some(msg)) = tokio::time::timeout(Duration::from_secs(1), audit_sub.next()).await {
        println!("     Received audit event: {} bytes", msg.payload.len());
    }

    // === Summary ===
    println!("\n{}", "=".repeat(70));
    println!("  DEMO COMPLETE");
    println!("{}", "=".repeat(70));

    let final_stats = coordinator.stats().await;
    println!("\n  Final Statistics:");
    println!("    Connected: {}", final_stats.connected);
    println!("    Tasks pending: {}", final_stats.tasks_pending);
    println!("    Tasks completed: {}", final_stats.tasks_completed);
    println!("    Duplicates prevented: {}", final_stats.duplicates_prevented);
    println!("    Logs collected: {}", final_stats.logs_collected);

    println!("\n  NATS Subjects Used:");
    println!("    agentic.tasks.>      - Task queue");
    println!("    agentic.proposals.>  - Proposal arbitrage");
    println!("    agentic.logs.>       - Centralized logs");
    println!("    agentic.audit.>      - Audit events");

    println!("\n  JetStream Streams:");
    println!("    AGENTIC_TASKS      - Work queue (retention: WorkQueue)");
    println!("    AGENTIC_PROPOSALS  - Proposals storage");
    println!("    AGENTIC_LOGS       - Log retention (7 days)");
    println!("    AGENTIC_AUDIT      - Audit trail\n");

    coordinator.stop().await?;

    Ok(())
}
