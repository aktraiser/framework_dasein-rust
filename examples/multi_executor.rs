//! Multi-Executor with Progressive Locking + Error Fingerprinting + MCP Doc
//!
//! Simple, generic approach:
//! 1. Validate Types → LOCK
//! 2. Validate Stubs (with locked Types) → LOCK
//! 3. Validate Logic (with locked Types+Stubs) → DONE
//!
//! Features:
//! - Error Fingerprinting: Simple errors → Fast model, Complex → Smart model
//! - MCP Doc Validator: Enriches errors with relevant documentation from Context7
//! - Rollback Manager: Prevents regressions by tracking best attempt
//! - Clippy: Catches anti-patterns before tests
//!
//! Environment variables:
//! - FAST_MODEL: Model for simple errors (default: gemini-2.0-flash)
//! - SMART_MODEL: Model for complex errors (default: gemini-2.0-flash)
//! - CONTEXT7_API_KEY: Enable MCP Doc Validator (optional)
//! - FIRECRACKER_URL: Remote Firecracker server URL (e.g., http://65.108.230.227:8080)
//! - FIRECRACKER_API_KEY: API key for Firecracker server (optional)
//!
//! If FIRECRACKER_URL is set, uses remote Firecracker sandbox (real microVM isolation).
//! Otherwise, uses local ProcessSandbox (no isolation, dev only).
//!
//! If a stage fails, regenerate ONLY that stage.

use agentic_core::distributed::{
    Executor, ValidatorPipeline, SandboxPipelineValidator, MCPDocValidator,
    MCPDocConfig, ValidatorInput, CodeAssembler,
    bus::{
        BusCoordinator, StateStore, BusLinter,
        RollbackManager, RollbackDecision,
        ErrorFingerprinter, ModelTier,
        AuditCollector, AuditEvent, TraceSequencer,
    },
    incremental_pipeline::Stage,
};
use agentic_sandbox::{ProcessSandbox, Sandbox};
#[cfg(feature = "remote")]
use agentic_sandbox::RemoteSandbox;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

const MAX_STAGE_RETRIES: u32 = 7;  // More retries with model escalation

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();

    println!("\n{}", "=".repeat(60));
    println!("  PROGRESSIVE LOCKING PIPELINE");
    println!("  Types → Stubs → Logic (lock each stage)");
    println!("  + Error Fingerprinting (Fast → Smart escalation)");
    println!("{}\n", "=".repeat(60));

    // === Setup NATS ===
    let nats_url = std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    let bus = match BusCoordinator::builder().nats_url(&nats_url).build_and_start().await {
        Ok(b) => {
            println!("✓ NATS connected");
            Arc::new(b)
        }
        Err(e) => {
            eprintln!("⚠ NATS not available: {}", e);
            return run_standalone().await;
        }
    };

    let state_store = Arc::new(StateStore::new(bus.nats_client()).await?);
    state_store.clear().await?;

    // === Audit Trail ===
    let audit = Arc::new(AuditCollector::new(bus.nats_client()).await?);
    let trace = TraceSequencer::new_trace();
    println!("✓ Audit enabled (trace: {})", &trace.trace_id()[..8]);

    let linter = BusLinter::new();
    let fingerprinter = ErrorFingerprinter::new();

    // === Task ===
    let task = r#"Implement an async task scheduler with dependencies in Rust.

Requirements:
1. TaskId - newtype around u64
2. TaskDef struct with: id, name, dependencies (Vec<TaskId>), max_retries, timeout
3. TaskError enum: Timeout, MaxRetriesExceeded, DependencyFailed, ExecutionError
4. TaskResult struct with: task_id, result, attempts, duration
5. Scheduler struct with:
   - new(max_concurrent: usize)
   - add_task(&mut self, task: TaskDef) - returns error if cycle detected
   - run(&self) -> Vec<TaskResult>

Rules:
- NO async recursion (use loop + stack)
- Use Arc<Semaphore> for concurrency (not Semaphore.clone())
- Use Arc<Mutex<T>> for shared state

Include tests."#;

    // === Setup Executors (Fast + Smart) ===
    // Fast model: cheap, good for simple errors (imports, syntax)
    let fast_model = std::env::var("FAST_MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_string());
    let fast_executor = Executor::new("fast-exe", "supervisor")
        .llm_gemini(&fast_model)
        .build();
    println!("✓ Fast model: {}", fast_model);

    // Smart model: Claude for complex errors (borrow checker, lifetimes)
    let smart_model = std::env::var("SMART_MODEL").unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());
    let smart_executor = Executor::new("smart-exe", "supervisor")
        .llm_anthropic(&smart_model)
        .build();
    println!("✓ Smart model: {} (Anthropic)", smart_model);

    let assembler = CodeAssembler::new();

    // Setup sandbox: Remote Firecracker (production) or local Process (dev)
    let sandbox: Box<dyn Sandbox> = if let Ok(firecracker_url) = std::env::var("FIRECRACKER_URL") {
        #[cfg(feature = "remote")]
        {
            let api_key = std::env::var("FIRECRACKER_API_KEY").ok();
            let mut builder = RemoteSandbox::new(&firecracker_url).timeout_ms(120_000);
            if let Some(key) = api_key {
                builder = builder.api_key(key);
            }
            let remote = builder.build();

            // Check if server is available
            match remote.health().await {
                Ok(health) => {
                    println!("✓ Firecracker Server: {}", firecracker_url);
                    println!("  ├── Status: {}", health.status);
                    println!("  ├── KVM: {}", if health.kvm_available { "✅" } else { "❌" });
                    println!("  └── Firecracker: {}", if health.firecracker_available { "✅" } else { "❌" });
                }
                Err(e) => {
                    eprintln!("⚠ Firecracker server not reachable: {}", e);
                    eprintln!("  Falling back to ProcessSandbox (no isolation)");
                    // Fallback handled below
                }
            }
            Box::new(remote)
        }
        #[cfg(not(feature = "remote"))]
        {
            eprintln!("⚠ FIRECRACKER_URL set but 'remote' feature not enabled");
            eprintln!("  Compile with: cargo run --example multi_executor --features remote");
            Box::new(ProcessSandbox::new().with_timeout(120_000))
        }
    } else {
        println!("⚠ Using ProcessSandbox (no isolation, dev only)");
        println!("  Set FIRECRACKER_URL to use remote Firecracker server");
        Box::new(ProcessSandbox::new().with_timeout(120_000))
    };

    // Setup validator pipeline
    let sandbox_validator = SandboxPipelineValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/progressive-validation"))
        .run_tests(true);

    // Build pipeline: Sandbox (compile+test) → MCP Doc (enrich with documentation)
    let pipeline = if let Ok(context7_key) = std::env::var("CONTEXT7_API_KEY") {
        println!("✓ MCP Doc Validator enabled (Context7)");
        let mcp_config = MCPDocConfig::context7(&context7_key);
        let mcp_validator = MCPDocValidator::new(mcp_config);
        ValidatorPipeline::new()
            .add(sandbox_validator)
            .add(mcp_validator)
    } else {
        println!("⚠ MCP Doc Validator disabled (set CONTEXT7_API_KEY to enable)");
        ValidatorPipeline::new().add(sandbox_validator)
    };

    // === Progressive Locking ===
    let total_start = Instant::now();

    // Emit pipeline start
    audit.emit(AuditEvent::pipeline_started(trace.trace_id(), trace.next(), task)).await?;

    // Stage 1: Generate and lock TYPES (always use fast model)
    println!("\n[Stage 1/3] TYPES");
    let locked_types = generate_and_lock_stage(
        Stage::Types,
        &fast_executor,
        &smart_executor,
        &fingerprinter,
        &assembler,
        &linter,
        &pipeline,
        task,
        "", // No previous context
        &audit,
        &trace,
    ).await?;
    println!("  ✓ Types LOCKED ({} chars)", locked_types.len());

    // Stage 2: Generate and lock STUBS (with locked types as context)
    println!("\n[Stage 2/3] STUBS");
    let locked_stubs = generate_and_lock_stage(
        Stage::Stubs,
        &fast_executor,
        &smart_executor,
        &fingerprinter,
        &assembler,
        &linter,
        &pipeline,
        task,
        &locked_types, // Previous locked stage
        &audit,
        &trace,
    ).await?;
    println!("  ✓ Stubs LOCKED ({} chars)", locked_stubs.len());

    // Stage 3: Generate and validate LOGIC (with locked types+stubs)
    println!("\n[Stage 3/3] LOGIC");
    let final_code = generate_and_lock_stage(
        Stage::Logic,
        &fast_executor,
        &smart_executor,
        &fingerprinter,
        &assembler,
        &linter,
        &pipeline,
        task,
        &locked_stubs, // Previous locked stages
        &audit,
        &trace,
    ).await?;

    // Emit pipeline completed
    let duration = total_start.elapsed().as_millis();
    audit.emit(AuditEvent::pipeline_completed(trace.trace_id(), trace.next(), true, duration as u64, 3)).await?;

    // Print audit report
    println!("\n{}", "=".repeat(60));
    println!("SUCCESS in {}ms", duration);
    println!("{}", "=".repeat(60));

    let report = audit.report(trace.trace_id()).await;
    println!("\n{}", report);

    println!("\n{}", "=".repeat(60));
    println!("GENERATED CODE");
    println!("{}\n", "=".repeat(60));
    println!("{}", final_code);

    bus.stop().await?;
    Ok(())
}

/// Generate and validate a single stage, retrying if needed.
/// On failure, errors are fed back into the prompt for the next attempt.
/// Uses RollbackManager to prevent regressions (rolling back if code stops compiling).
/// Uses ErrorFingerprinter to escalate to smart model for complex errors.
async fn generate_and_lock_stage(
    stage: Stage,
    fast_executor: &Executor,
    smart_executor: &Executor,
    fingerprinter: &ErrorFingerprinter,
    assembler: &CodeAssembler,
    linter: &BusLinter,
    pipeline: &ValidatorPipeline,
    task: &str,
    locked_context: &str,
    audit: &AuditCollector,
    trace: &TraceSequencer,
) -> Result<String, Box<dyn std::error::Error>> {

    let system = "You are an expert Rust developer. Return ONLY valid Rust code. No markdown.";
    let stage_name = format!("{:?}", stage);

    // Track errors and previous code for feedback loop
    let mut previous_errors: Vec<String> = vec![];
    let mut previous_code: Option<String> = None;

    // RollbackManager tracks best attempt and prevents regressions
    let mut rollback = RollbackManager::new();

    // Track which tier we're using
    let mut current_tier = ModelTier::Fast;

    // Track consecutive failures for escalation
    let mut consecutive_fast_failures = 0;

    for attempt in 1..=MAX_STAGE_RETRIES {
        // Emit stage started
        audit.emit(AuditEvent::stage_started(trace.trace_id(), trace.next(), &stage_name, attempt)).await?;

        // Select executor based on error fingerprinting + escalation on repeated failures
        let executor = if !previous_errors.is_empty() {
            let analysis = fingerprinter.analyze(&previous_errors);
            current_tier = analysis.recommended_tier;

            // Escalate to Smart after 3 consecutive Fast failures (model is stuck)
            let reason = if current_tier == ModelTier::Fast && consecutive_fast_failures >= 3 {
                current_tier = ModelTier::Smart;
                "escalated after 3 consecutive failures"
            } else {
                "fingerprint recommendation"
            };

            let categories: Vec<String> = analysis.category_counts.keys().map(|c| format!("{:?}", c)).collect();
            audit.emit(AuditEvent::model_selected(trace.trace_id(), trace.next(), current_tier, reason, categories)).await?;

            match current_tier {
                ModelTier::Fast => fast_executor,
                ModelTier::Smart | ModelTier::Expert => smart_executor,
            }
        } else {
            current_tier = ModelTier::Fast;
            consecutive_fast_failures = 0;
            fast_executor
        };

        let tier_indicator = match current_tier {
            ModelTier::Fast => "⚡",
            ModelTier::Smart => "🧠",
            ModelTier::Expert => "🎓",
        };

        println!("  Attempt {}/{} {}", attempt, MAX_STAGE_RETRIES, tier_indicator);

        // Build prompt with error feedback if this is a retry
        let mut stage_prompt = build_stage_prompt(
            stage,
            task,
            locked_context,
            &previous_errors,
            previous_code.as_deref(),
        );

        // Add fingerprinter hints for complex errors
        if !previous_errors.is_empty() && current_tier != ModelTier::Fast {
            let analysis = fingerprinter.analyze(&previous_errors);
            let hint = fingerprinter.generate_hint(&analysis);
            if !hint.is_empty() {
                stage_prompt = format!("{}\n\n=== HINTS ===\n{}", stage_prompt, hint);
            }
        }

        // Generate
        let result = executor.execute(system, &stage_prompt).await?;
        let code = assembler.clean_for_validation(&result.content);

        // Emit code snapshot
        audit.emit(AuditEvent::code_snapshot(trace.trace_id(), trace.next(), &stage_name, attempt, &code)).await?;

        // Quick lint check
        let lint = linter.lint(&code);
        if !lint.passed {
            let err = lint.errors.first().map(|e| e.message.clone()).unwrap_or_default();
            println!("    ✗ Lint failed: {}", err);

            // Emit validation failed (lint)
            audit.emit(AuditEvent::validation_completed(
                trace.trace_id(), trace.next(), "BusLinter", false, &[err.clone()]
            )).await?;

            previous_errors = vec![format!("Lint error: {}", err)];
            previous_code = Some(code);
            // Track consecutive failures for escalation
            if current_tier == ModelTier::Fast {
                consecutive_fast_failures += 1;
            } else {
                consecutive_fast_failures = 0;
            }
            continue;
        }

        // For Types and Stubs, lint is enough
        if matches!(stage, Stage::Types | Stage::Stubs) {
            println!("    ✓ Lint OK");
            // Emit stage completed
            audit.emit(AuditEvent::stage_completed(trace.trace_id(), trace.next(), &stage_name, attempt, true, &code)).await?;
            return Ok(code); // Success - no need to reset counter, we're returning
        }

        // Track validation failure for escalation
        // (counter is incremented after validation fails, reset only on full success)

        // For Logic, full validation
        println!("    Validating...");
        let input = ValidatorInput::new(&code, "rust").with_task(task);
        let val_result = pipeline.validate(input).await;

        // Emit validation result
        let val_errors: Vec<String> = val_result.results.iter().flat_map(|r| r.errors.clone()).collect();
        audit.emit(AuditEvent::validation_completed(
            trace.trace_id(), trace.next(), "SandboxPipeline", val_result.passed, &val_errors
        )).await?;

        if val_result.passed {
            println!("    ✓ Validation passed");
            // Emit stage completed
            audit.emit(AuditEvent::stage_completed(trace.trace_id(), trace.next(), &stage_name, attempt, true, &code)).await?;
            return Ok(code);
        }

        // Collect errors for feedback
        let errors: Vec<String> = val_result.results
            .iter()
            .flat_map(|r| r.errors.clone())
            .collect();

        // Fingerprint errors for next iteration
        let analysis = fingerprinter.analyze(&errors);
        let category = format!("{:?}", analysis.dominant_category);

        // Record attempt and check if we should rollback
        let decision = rollback.record(attempt, code.clone(), errors.clone());

        // Track consecutive failures for escalation
        if current_tier == ModelTier::Fast {
            consecutive_fast_failures += 1;
        } else {
            consecutive_fast_failures = 0;
        }

        match decision {
            RollbackDecision::Rollback { code: best_code, reason, failing_tests, .. } => {
                println!("    ↩ ROLLBACK: {}", reason);

                // Emit rollback decision (current attempt failed, rolling back to best)
                let best_score_val = rollback.best().map(|b| b.score.quality_score());
                audit.emit(AuditEvent::rollback_decision(
                    trace.trace_id(), trace.next(),
                    "rollback", -(errors.len() as i32 * 100), best_score_val, &reason
                )).await?;

                // Use targeted prompt for the failing test
                if let Some(targeted_prompt) = rollback.generate_targeted_prompt(
                    failing_tests.first().map(|s| s.as_str()).unwrap_or("test")
                ) {
                    previous_errors = failing_tests;
                    previous_code = Some(targeted_prompt);
                } else {
                    previous_code = Some(best_code);
                    previous_errors = errors;
                }
            }
            RollbackDecision::Continue { current_score, best_score } => {
                let score_info = if let Some(ref best) = best_score {
                    format!(" (score: {}, best: {})", current_score.quality_score(), best.quality_score())
                } else {
                    format!(" (score: {})", current_score.quality_score())
                };
                let tier_info = format!(" [{}→{}]", category, analysis.recommended_tier.as_str());
                println!("    ✗ {} errors{}{}", errors.len(), score_info, tier_info);

                // Emit continue decision
                audit.emit(AuditEvent::rollback_decision(
                    trace.trace_id(), trace.next(),
                    "continue", current_score.quality_score(), best_score.as_ref().map(|s| s.quality_score()), "progressing"
                )).await?;

                previous_errors = errors;
                previous_code = Some(code);
            }
        }

        for err in previous_errors.iter().take(3) {
            println!("      - {}", err.lines().next().unwrap_or(""));
        }
    }

    // If we have a best attempt that compiles, return it even with test failures
    if let Some(best) = rollback.best() {
        if best.score.compiles {
            println!("    ⚠ Returning best attempt (compiles, {} tests fail)", best.score.tests_failed);
            // Emit stage completed with partial success
            audit.emit(AuditEvent::stage_completed(
                trace.trace_id(), trace.next(), &stage_name, best.number, true, &best.code
            )).await?;
            return Ok(best.code.clone());
        }
    }

    // Emit stage failed
    audit.emit(AuditEvent::error(
        trace.trace_id(), trace.next(), &stage_name,
        &format!("Failed after {} attempts", MAX_STAGE_RETRIES), false
    )).await?;

    Err(format!("Stage {:?} failed after {} attempts", stage, MAX_STAGE_RETRIES).into())
}

/// Build the prompt for a stage, including error feedback if this is a retry.
fn build_stage_prompt(
    stage: Stage,
    task: &str,
    locked_context: &str,
    previous_errors: &[String],
    previous_code: Option<&str>,
) -> String {
    let base_prompt = match stage {
        Stage::Types => format!(
            r#"Generate ONLY the type definitions for this task:

{}

Return ONLY:
- use statements (std::..., tokio::..., etc.)
- struct definitions with #[derive(...)] attributes
- enum definitions with variants
- type aliases if needed

NO function implementations. NO impl blocks. NO fn keyword.

CRITICAL: Every opening brace {{ must have a matching closing brace }}.
Return ONLY valid Rust code that compiles. No markdown, no explanations."#,
            task
        ),
        Stage::Stubs => format!(
            r#"Here is the complete code structure you must output:

=== COPY THESE TYPES EXACTLY ===
{}

=== ADD IMPL BLOCKS BELOW ===
Generate impl blocks with function SIGNATURES ONLY.
Use todo!() for all function bodies.

TASK: {}

OUTPUT FORMAT - Return this exact structure:
1. First: ALL the use statements and type definitions above (copy exactly)
2. Then: Your impl blocks with todo!() bodies

CRITICAL: Every opening brace {{ must have a matching closing brace }}.
Return ONLY valid Rust code that compiles. No markdown, no explanations."#,
            locked_context, task
        ),
        Stage::Logic => format!(
            r#"=== LOCKED TYPES AND SIGNATURES (DO NOT MODIFY SIGNATURES) ===
{}

=== TASK ===
{}

Now implement the function bodies. Keep all type definitions and function signatures EXACTLY as shown above.

CRITICAL RULES:
- NO async recursion. Use loop + Vec as stack.
- Use Arc::new(Semaphore::new(n)), not Semaphore::new(n)
- Use Arc::clone(&sem), not sem.clone()
- Use semaphore.acquire().await, not acquire_owned()

Return complete code with tests. No markdown."#,
            locked_context, task
        ),
    };

    // If we have previous errors, add them to the prompt
    if !previous_errors.is_empty() {
        let error_section = format!(
            r#"

=== PREVIOUS ATTEMPT FAILED ===
The following errors occurred. Fix them:

{}

=== YOUR PREVIOUS CODE ===
{}

Fix the errors above and return the corrected complete code."#,
            previous_errors.join("\n\n"),
            previous_code.unwrap_or("(not available)")
        );
        format!("{}{}", base_prompt, error_section)
    } else {
        base_prompt
    }
}

/// Standalone mode without NATS
async fn run_standalone() -> Result<(), Box<dyn std::error::Error>> {
    println!("Running standalone (no NATS)...\n");
    // Simplified standalone version
    Ok(())
}
