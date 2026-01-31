//! Scale test - 1 Supervisor, 50 Executors, 10 Validators
//!
//! Run with:
//! ```bash
//! cargo run --example scale_test
//! ```

use dasein_agentic_core::distributed::{Capability, Supervisor};
use std::time::Instant;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║           AGENTIC-RS SCALE TEST                              ║");
    println!("║           1 Supervisor | 50 Executors | 10 Validators        ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // ========== CRÉATION ==========
    println!("▶ Creating supervisor with 50 executors and 10 validators...");
    let start = Instant::now();

    let supervisor = Supervisor::new("sup-scale-test")
        .domain("scale")
        .executors(50)
        .validators(10)
        .llm_gemini("gemini-2.0-flash")
        .sandbox_process()
        .allow_lending(true)
        .allow_borrowing(true)
        .max_borrowed(20)
        .capability(Capability::CodeGeneration)
        .capability(Capability::CodeExecution)
        .build_async()
        .await;

    let creation_time = start.elapsed();
    println!("  ✓ Created in {:?}", creation_time);
    println!();

    // ========== STATS ==========
    println!("▶ Supervisor Stats:");
    println!("  ├── ID:              {}", supervisor.id);
    println!("  ├── Domain:          {}", supervisor.domain);
    println!("  ├── Pool size:       {}", supervisor.pool_size().await);
    println!("  ├── Idle executors:  {}", supervisor.idle_count().await);
    println!("  ├── Validators:      {}", supervisor.validator_count());
    println!(
        "  ├── Utilization:     {:.1}%",
        supervisor.utilization().await * 100.0
    );
    println!("  └── Can lend:        {}", supervisor.can_lend(10).await);
    println!();

    // ========== SCALE UP TEST ==========
    println!("▶ Testing scale up (adding 10 more executors)...");
    let start = Instant::now();
    supervisor.scale_up(10).await;
    println!("  ✓ Scale up in {:?}", start.elapsed());
    println!("  └── New pool size: {}", supervisor.pool_size().await);
    println!();

    // ========== GET EXECUTORS TEST ==========
    println!("▶ Testing get multiple executors...");
    let start = Instant::now();
    let executors = supervisor.get_executors(20).await;
    println!(
        "  ✓ Got {} executors in {:?}",
        executors.len(),
        start.elapsed()
    );
    println!();

    // ========== VALIDATION TEST ==========
    println!("▶ Testing validation (10 validators)...");
    let test_outputs: &[&str] = &[
        "fn main() { println!(\"Hello\"); }",
        "",                                    // Should fail: empty
        "fn test() { /* TODO: implement */ }", // Should fail: has TODO
        "let password = \"secret123\";",       // Should fail: has secret
        "pub fn calculate(x: i32) -> i32 { x * 2 }",
    ];

    for (i, output) in test_outputs.iter().enumerate() {
        let result = supervisor.validate_with(i, output, 0);
        let status = if result.passed {
            "✓ PASS"
        } else {
            "✗ FAIL"
        };
        let preview = if output.len() > 30 {
            format!("{}...", &output[..30])
        } else if output.is_empty() {
            "(empty)".to_string()
        } else {
            output.to_string()
        };
        println!("  {} [score: {:3}] {:?}", status, result.score, preview);
    }
    println!();

    // ========== LENDING TEST ==========
    println!("▶ Testing executor lending...");
    if supervisor.can_lend(5).await {
        let grant = supervisor.lend_executors("sup-other", 5, 60).await;
        if let Some(grant) = grant {
            println!(
                "  ✓ Lent {} executors to sup-other",
                grant.executor_ids.len()
            );
            println!("  └── Lease ID: {}", grant.lease_id);
            println!("  └── Lent count: {}", supervisor.lent_count().await);
            println!(
                "  └── Effective pool: {}",
                supervisor.effective_pool_size().await
            );
        }
    }
    println!();

    // ========== SCALE DOWN TEST ==========
    println!("▶ Testing scale down (removing 20 executors)...");
    let start = Instant::now();
    supervisor.scale_down(20).await;
    println!("  ✓ Scale down in {:?}", start.elapsed());
    println!("  └── New pool size: {}", supervisor.pool_size().await);
    println!();

    // ========== MEMORY ESTIMATE ==========
    let executor_size_estimate = 1024 * 20; // ~20KB per executor (rough)
    let total_memory_kb = supervisor.pool_size().await * executor_size_estimate / 1024;
    println!("▶ Memory Estimate:");
    println!(
        "  └── ~{} KB for {} executors",
        total_memory_kb,
        supervisor.pool_size().await
    );
    println!();

    // ========== FINAL STATS ==========
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║                     FINAL STATS                              ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!(
        "║  Pool size:        {:>4}                                     ║",
        supervisor.pool_size().await
    );
    println!(
        "║  Idle:             {:>4}                                     ║",
        supervisor.idle_count().await
    );
    println!(
        "║  Validators:       {:>4}                                     ║",
        supervisor.validator_count()
    );
    println!(
        "║  Lent:             {:>4}                                     ║",
        supervisor.lent_count().await
    );
    println!(
        "║  Effective pool:   {:>4}                                     ║",
        supervisor.effective_pool_size().await
    );
    println!(
        "║  Creation time:    {:>10?}                              ║",
        creation_time
    );
    println!("╚══════════════════════════════════════════════════════════════╝");

    Ok(())
}
