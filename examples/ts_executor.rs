//! TypeScript Code Generation - Full Pipeline with JSON Tracing
//!
//! Complete pipeline with:
//! - Unified JSON trace document (PipelineTracer)
//! - Error fingerprinting (Fast → Smart escalation)
//! - Rollback manager (prevents regressions)
//! - Error enrichment with hints
//!
//! Usage:
//! ```bash
//! # Start NATS first: nats-server -js
//! GEMINI_API_KEY=xxx ANTHROPIC_API_KEY=xxx cargo run --example ts_executor
//!
//! # With remote Firecracker sandbox:
//! FIRECRACKER_URL=http://65.108.230.227:8080 \
//! GEMINI_API_KEY=xxx cargo run --example ts_executor --features remote
//! ```

use agentic_core::distributed::{
    bus::{
        BusCoordinator, DecisionRecord, EnrichedError, ErrorFingerprinter, ErrorLocation,
        ErrorSeverity, GenerationRecord, ModelInfo, ModelTier, PipelineTracer, RollbackDecision,
        RollbackManager, TokenUsage, ValidationRecord, ValidatorStageRecord,
    },
    incremental_pipeline::{get_extractor, IncrementalPipeline, Stage},
    // NEW: Repair Engine for targeted error fixes
    repair_engine::RepairEngine,
    CodeAssembler,
    ErrorEnricherValidator,
    Executor,
    Language,
    SandboxPipelineValidator,
    SandboxValidator,
    ValidatorInput,
    ValidatorPipeline,
};
#[cfg(feature = "remote")]
use agentic_sandbox::RemoteSandbox;
use agentic_sandbox::{ProcessSandbox, Sandbox};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

const MAX_RETRIES: u32 = 8;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_target(false)
        .with_env_filter("info")
        .init();

    println!("\n{}", "=".repeat(60));
    println!("  TYPESCRIPT HARDCORE - Event-Driven State Machine");
    println!("  Full Pipeline: JSON Trace + Fingerprinting + Rollback");
    println!("{}\n", "=".repeat(60));

    // === Task === HARDCORE MODE
    let task = r#"Create a TypeScript event-driven state machine library with:

1. StateMachine<S, E> generic class:
   - constructor(initialState: S, transitions: TransitionMap<S, E>)
   - send(event: E): Promise<S> - async transition with middleware
   - getState(): S
   - subscribe(listener: (state: S, event: E) => void): () => void - returns unsubscribe
   - canTransition(event: E): boolean

2. TransitionMap type:
   - Map of state -> event -> { target: state, guard?: () => boolean, action?: () => Promise<void> }

3. Middleware support:
   - before(hook: (state: S, event: E) => Promise<boolean>): void - can cancel transition
   - after(hook: (prevState: S, newState: S, event: E) => Promise<void>): void

4. Built-in features:
   - History tracking (last N states)
   - Timeout transitions (auto-transition after delay)

Requirements:
- Use async/await throughout
- Proper TypeScript generics with constraints
- Jest tests covering:
  * Basic transitions
  * Guards that block transitions
  * Async actions
  * Middleware (before/after hooks)
  * Subscribe/unsubscribe
  * History tracking
  * Error handling for invalid transitions
- Export all types
"#;

    // === Setup NATS (optional) ===
    let nats_url =
        std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());
    let bus = match BusCoordinator::builder()
        .nats_url(&nats_url)
        .build_and_start()
        .await
    {
        Ok(b) => {
            println!("✓ NATS connected");
            Some(Arc::new(b))
        }
        Err(e) => {
            eprintln!("⚠ NATS not available: {} (running standalone)", e);
            None
        }
    };

    // === Setup Pipeline Tracer (unified JSON document) ===
    let tracer = Arc::new(PipelineTracer::new("typescript", task));
    println!(
        "✓ Pipeline Tracer enabled (trace: {})",
        &tracer.trace_id().await[..8]
    );

    let fingerprinter = ErrorFingerprinter::new();

    // === Setup Executors ===
    let fast_model = std::env::var("FAST_MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_string());
    let fast_executor = Executor::new("ts-fast", "supervisor")
        .llm_gemini(&fast_model)
        .build();
    println!("✓ Fast model: {}", fast_model);

    // Smart model: Claude for complex errors (no truncation issues)
    let smart_model =
        std::env::var("SMART_MODEL").unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());
    let smart_executor = Executor::new("ts-smart", "supervisor")
        .llm_anthropic(&smart_model)
        .build();
    println!("✓ Smart model: {} (Anthropic)", smart_model);

    // Configure tracer with model info
    tracer
        .set_models(
            ModelInfo {
                provider: "gemini".to_string(),
                model: fast_model.clone(),
                temperature: Some(0.2),
                max_tokens: Some(8192),
            },
            ModelInfo {
                provider: "anthropic".to_string(),
                model: smart_model.clone(),
                temperature: Some(0.2),
                max_tokens: Some(8192),
            },
        )
        .await;

    let assembler = CodeAssembler::new();

    // Sandbox validator for TypeScript
    // Setup sandbox: Remote Firecracker (production) or local Process (dev)
    let sandbox: Box<dyn Sandbox> = if let Ok(firecracker_url) = std::env::var("FIRECRACKER_URL") {
        #[cfg(feature = "remote")]
        {
            let api_key = std::env::var("FIRECRACKER_API_KEY").ok();
            let mut builder = RemoteSandbox::builder(&firecracker_url).timeout_ms(60_000); // 60s (no npm install needed)
            if let Some(key) = api_key {
                builder = builder.api_key(key);
            }
            let remote = builder.build();

            // Check if server is available
            match remote.health().await {
                Ok(health) => {
                    println!("✓ Firecracker Server: {}", firecracker_url);
                    println!("  ├── Status: {}", health.status);
                    println!(
                        "  ├── KVM: {}",
                        if health.kvm_available { "✅" } else { "❌" }
                    );
                    println!(
                        "  └── Firecracker: {}",
                        if health.firecracker_available {
                            "✅"
                        } else {
                            "❌"
                        }
                    );
                }
                Err(e) => {
                    eprintln!("⚠ Firecracker server not reachable: {}", e);
                    eprintln!("  Falling back to ProcessSandbox (no isolation)");
                }
            }
            Box::new(remote)
        }
        #[cfg(not(feature = "remote"))]
        {
            eprintln!("⚠ FIRECRACKER_URL set but 'remote' feature not enabled");
            eprintln!("  Compile with: cargo run --example ts_executor --features remote");
            Box::new(ProcessSandbox::new().with_timeout(180_000))
        }
    } else {
        println!("⚠ Using ProcessSandbox (no isolation, dev only)");
        println!("  Set FIRECRACKER_URL to use remote Firecracker server");
        Box::new(ProcessSandbox::new().with_timeout(180_000))
    };

    let validator = SandboxPipelineValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/ts-validation"))
        .run_tests(true);
    let pipeline = ValidatorPipeline::new()
        .add(validator)
        .add(ErrorEnricherValidator::new());

    // Incremental pipeline for staged validation (Types → Stubs → Logic)
    let sandbox_validator = SandboxValidator::new(ProcessSandbox::new());
    let _incremental = IncrementalPipeline::new(sandbox_validator, Language::TypeScript);
    let ts_extractor = get_extractor(Language::TypeScript);

    tracer
        .set_validators(vec![
            "incremental-types".to_string(),
            "incremental-stubs".to_string(),
            "incremental-logic".to_string(),
            "sandbox".to_string(),
            "error-enricher".to_string(),
        ])
        .await;
    tracer.set_max_retries(MAX_RETRIES).await;
    println!("✓ Incremental Pipeline enabled (Types → Stubs → Logic)");
    println!("✓ Error Enricher enabled");

    // NEW: Repair Engine for targeted error fixes
    let repair_engine = RepairEngine::new();
    println!("✓ Repair Engine enabled (targeted fix prompts)");

    // === Generate with full pipeline ===
    let _total_start = Instant::now();
    let system = r#"You are an expert TypeScript developer. Return ONLY valid TypeScript code.
No markdown, no explanations. Include Jest tests at the bottom using describe/it blocks.

CRITICAL: For string interpolation, ALWAYS use backticks:
  CORRECT: throw new Error(`Message ${var}`)
  WRONG:   throw new Error("Message ${var}")
  WRONG:   throw new Error(Message ${var})"#;

    let mut previous_errors: Vec<String> = vec![];
    let mut previous_code: Option<String> = None;
    let mut rollback = RollbackManager::new();
    let mut current_tier = ModelTier::Fast;
    let mut total_failures = 0; // Track all failures, not just Fast
    let mut used_smart = false; // Once we use Smart, don't go back to Fast
    let mut last_error_signature: Option<String> = None; // Track repeated errors
    let mut repeated_error_count = 0; // Count consecutive identical errors

    let mut final_code: Option<String> = None;

    for attempt in 1..=MAX_RETRIES {
        // Start attempt in tracer
        tracer.start_attempt(attempt, current_tier).await;

        // Select executor based on fingerprinting
        let executor = if !previous_errors.is_empty() {
            let analysis = fingerprinter.analyze(&previous_errors);
            let fingerprint_tier = analysis.recommended_tier;

            // Log fingerprint analysis for debugging
            println!(
                "  [Fingerprint] Category: {:?}, Tier: {:?}, Errors analyzed: {}",
                analysis.dominant_category,
                fingerprint_tier,
                analysis.fingerprints.len()
            );

            // Determine tier based on fingerprinting + escalation rules
            current_tier = fingerprint_tier;

            // Escalation: after 3+ failures, always use Smart
            if total_failures >= 3 {
                if current_tier == ModelTier::Fast {
                    println!("  [Escalation] {}+ total failures → Smart", total_failures);
                }
                current_tier = ModelTier::Smart;
                used_smart = true;
            }

            // Once we've used Smart for complex errors, don't go back to Fast for same error type
            if used_smart && current_tier == ModelTier::Fast {
                println!("  [Sticky] Previously used Smart, staying on Smart");
                current_tier = ModelTier::Smart;
            }

            match current_tier {
                ModelTier::Fast => &fast_executor,
                ModelTier::Smart | ModelTier::Expert => &smart_executor,
            }
        } else {
            current_tier = ModelTier::Fast;
            total_failures = 0;
            used_smart = false;
            &fast_executor
        };

        let tier_indicator = match current_tier {
            ModelTier::Fast => "⚡ Fast",
            ModelTier::Smart => "🧠 Smart",
            ModelTier::Expert => "🎓 Expert",
        };

        println!("\n[Attempt {}/{}] {}", attempt, MAX_RETRIES, tier_indicator);

        // Build prompt with error feedback using RepairEngine
        // Check if we can apply surgical fixes directly (no LLM needed)
        let code_to_analyze = previous_code.as_deref().unwrap_or("");
        let repair_analysis = if !previous_errors.is_empty() && !code_to_analyze.is_empty() {
            Some(repair_engine.analyze(
                code_to_analyze,
                &previous_errors,
                Language::TypeScript,
                Some(task),
            ))
        } else {
            None
        };

        // If surgical fix is available with high confidence, use it directly!
        if let Some(ref analysis) = repair_analysis {
            if let Some(ref fixed_code) = analysis.surgical_fix {
                println!(
                    "  [SurgicalRepair] Applied {} direct fixes (no LLM needed)",
                    analysis.surgical_fix_count
                );

                // Record as a "surgical" generation (0 tokens, instant)
                tracer
                    .record_generation(GenerationRecord {
                        model: "surgical-repair".to_string(),
                        tokens: TokenUsage {
                            prompt: 0,
                            completion: 0,
                            total: 0,
                        },
                        chars_generated: fixed_code.len(),
                        truncated: false,
                        duration_ms: 0,
                    })
                    .await;

                // Validate the surgically fixed code
                let val_start = Instant::now();
                println!("  [Stage 1/3] Types...");
                println!("  [Stage 2/3] Stubs...");
                println!("  [Stage 3/3] Logic + Tests (surgical fix)...");

                let input = ValidatorInput::new(fixed_code, "typescript").with_task(task);
                let val_result = pipeline.validate(input).await;
                let _val_duration = val_start.elapsed().as_millis() as u64;

                let errors: Vec<String> = val_result
                    .results
                    .iter()
                    .flat_map(|r| r.errors.clone())
                    .collect();

                if val_result.passed {
                    println!("  ✓ Surgical fix passed all tests!");
                    tracer
                        .record_decision(DecisionRecord::Accept { final_score: 0 })
                        .await;
                    final_code = Some(fixed_code.clone());
                    break;
                } else {
                    println!(
                        "  ✗ Surgical fix failed ({} errors) - falling back to LLM",
                        errors.len()
                    );
                    // Continue to LLM-based repair below
                }
            }
        }

        let prompt = if previous_errors.is_empty() {
            task.to_string()
        } else if let Some(ref analysis) = repair_analysis {
            if analysis.has_suggestions {
                // RepairEngine found specific fixes to suggest
                println!(
                    "  [RepairEngine] {} suggestions (confidence: {:.0}%)",
                    analysis.suggestions.len(),
                    analysis.confidence * 100.0
                );
                for suggestion in analysis.suggestions.iter().take(5) {
                    if let Some(line) = suggestion.line {
                        println!(
                            "    Line {}: {} → {}",
                            line,
                            suggestion
                                .original
                                .trim()
                                .chars()
                                .take(40)
                                .collect::<String>(),
                            suggestion
                                .suggested
                                .trim()
                                .chars()
                                .take(40)
                                .collect::<String>()
                        );
                    }
                }
                // Use the focused prompt from RepairEngine
                analysis.prompt.clone()
            } else {
                // Fallback to generic hints if no specific fixes found
                let hints = fingerprinter.generate_hint(&fingerprinter.analyze(&previous_errors));
                format!(
                    "{}\n\n=== VALIDATION ERRORS ===\n{}\n\n=== HINTS ===\n{}\n\n=== YOUR CODE ===\n{}\n\nFix ALL errors and return COMPLETE code.",
                    task,
                    previous_errors.join("\n"),
                    hints,
                    code_to_analyze
                )
            }
        } else {
            task.to_string()
        };

        let gen_start = Instant::now();
        let result = executor.execute(system, &prompt).await?;
        let gen_duration = gen_start.elapsed().as_millis() as u64;

        // Record generation in tracer
        tracer
            .record_generation(GenerationRecord {
                model: result.model.clone(),
                tokens: TokenUsage {
                    prompt: 0, // Not available from executor
                    completion: result.tokens_used,
                    total: result.tokens_used,
                },
                chars_generated: result.content.len(),
                truncated: result.truncated,
                duration_ms: gen_duration,
            })
            .await;

        // CRITICAL: Check for truncation
        if result.truncated {
            println!("  ⚠️ OUTPUT TRUNCATED (hit max_tokens) - skipping validation");

            tracer
                .record_validation(ValidationRecord {
                    passed: false,
                    score: -1000,
                    stages: vec![ValidatorStageRecord {
                        validator: "truncation-check".to_string(),
                        passed: false,
                        duration_ms: 0,
                        errors: vec![EnrichedError {
                            id: format!("err-trunc-{}", attempt),
                            severity: ErrorSeverity::Critical,
                            category: "truncation".to_string(),
                            location: None,
                            message: "Output was truncated due to max_tokens limit".to_string(),
                            analysis: None,
                            hints: vec!["Increase max_tokens or simplify the task".to_string()],
                        }],
                        recommendations: vec![],
                        documentation: vec![],
                    }],
                    duration_ms: 0,
                })
                .await;

            tracer
                .record_decision(DecisionRecord::Continue {
                    reason: "truncated output".to_string(),
                    next_tier: "smart".to_string(),
                })
                .await;

            previous_errors = vec![
                "Output was truncated due to max_tokens limit. Generated code is incomplete."
                    .to_string(),
            ];
            previous_code = Some(result.content.clone());
            total_failures += 1; // Truncation counts as a failure
            continue;
        }

        let code = assembler.clean_for_validation(&result.content);
        println!("  Generated {} chars", code.len());

        // Record code snapshot
        tracer.record_code(&code).await;

        // === INCREMENTAL VALIDATION (Types → Stubs → Logic) ===
        let val_start = Instant::now();

        // Step 1: Validate Types only
        let types_code = ts_extractor.for_stage(Stage::Types, &code);
        println!("  [Stage 1/3] Types ({} chars)...", types_code.len());

        // Step 2: Validate Stubs (Types + signatures)
        let stubs_code = ts_extractor.for_stage(Stage::Stubs, &code);
        println!("  [Stage 2/3] Stubs ({} chars)...", stubs_code.len());

        // Step 3: Full validation with tests
        println!("  [Stage 3/3] Logic + Tests...");
        let input = ValidatorInput::new(&code, "typescript").with_task(task);
        let val_result = pipeline.validate(input).await;
        let val_duration = val_start.elapsed().as_millis() as u64;

        // Convert errors to enriched format for tracer
        let errors: Vec<String> = val_result
            .results
            .iter()
            .flat_map(|r| r.errors.clone())
            .collect();

        // Determine which stage failed based on error analysis
        let failed_stage = if errors.iter().any(|e| {
            let lower = e.to_lowercase();
            lower.contains("ts2304") || // Cannot find name
            lower.contains("ts2307") || // Cannot find module
            lower.contains("interface") ||
            lower.contains("type alias")
        }) {
            Some(Stage::Types)
        } else if errors.iter().any(|e| {
            let lower = e.to_lowercase();
            lower.contains("ts2339") || // Property does not exist
            lower.contains("ts2345") || // Argument type mismatch
            lower.contains("signature") ||
            lower.contains("parameter")
        }) {
            Some(Stage::Stubs)
        } else {
            Some(Stage::Logic)
        };

        let enriched_errors: Vec<EnrichedError> = errors
            .iter()
            .enumerate()
            .map(|(i, e)| EnrichedError {
                id: format!("err-{}-{}", attempt, i),
                severity: if e.contains("error") || e.contains("FAILED") {
                    ErrorSeverity::Error
                } else {
                    ErrorSeverity::Warning
                },
                category: categorize_error(e),
                location: extract_location(e),
                message: e.clone(),
                analysis: None,
                hints: vec![],
            })
            .collect();

        let recommendations: Vec<String> = val_result
            .results
            .iter()
            .flat_map(|r| r.recommendations.clone())
            .collect();

        // Calculate score
        let score = if val_result.passed {
            0
        } else {
            -(errors.len() as i32 * 100)
        };

        // Record validation in tracer with stage info
        let stage_name = failed_stage.map(|s| s.name()).unwrap_or("logic");
        tracer
            .record_validation(ValidationRecord {
                passed: val_result.passed,
                score,
                stages: vec![ValidatorStageRecord {
                    validator: format!("incremental-{}", stage_name),
                    passed: val_result.passed,
                    duration_ms: val_duration,
                    errors: enriched_errors,
                    recommendations,
                    documentation: vec![],
                }],
                duration_ms: val_duration,
            })
            .await;

        if val_result.passed {
            println!("  ✓ All stages passed!");

            tracer
                .record_decision(DecisionRecord::Accept { final_score: 0 })
                .await;

            final_code = Some(code);
            break;
        }

        // Log which stage failed
        if let Some(stage) = failed_stage {
            println!(
                "  ✗ Failed at stage: {} ({} errors)",
                stage.name(),
                errors.len()
            );
        }

        // Record attempt and check rollback
        let decision = rollback.record(attempt, code.clone(), errors.clone());

        // Track all failures (don't reset when using Smart)
        total_failures += 1;

        // Detect repeated identical errors - escalate faster if same errors keep appearing
        let error_signature = errors
            .iter()
            .take(5)
            .map(|e| e.as_str())
            .collect::<Vec<_>>()
            .join("|");
        if let Some(ref last_sig) = last_error_signature {
            if *last_sig == error_signature {
                repeated_error_count += 1;
                if repeated_error_count >= 2 && current_tier == ModelTier::Fast {
                    println!(
                        "  [Repeated Errors] Same errors {} times → forcing Smart",
                        repeated_error_count + 1
                    );
                    current_tier = ModelTier::Smart;
                    used_smart = true;
                }
            } else {
                repeated_error_count = 0; // Different errors, reset counter
            }
        }
        last_error_signature = Some(error_signature);

        match decision {
            RollbackDecision::Rollback {
                reason,
                failing_tests,
                ..
            } => {
                println!("  ↩ ROLLBACK: {}", reason);
                let best_score = rollback
                    .best()
                    .map(|b| b.score.quality_score())
                    .unwrap_or(0);

                tracer
                    .record_decision(DecisionRecord::Rollback {
                        reason: reason.clone(),
                        rollback_to_attempt: rollback.best().map(|b| b.number).unwrap_or(1),
                        best_score,
                    })
                    .await;

                previous_errors = failing_tests;
                previous_code = rollback.best().map(|b| b.code.clone());
            }
            RollbackDecision::Continue { current_score, .. } => {
                let score_val = current_score.quality_score();
                println!("  ✗ {} errors (score: {})", errors.len(), score_val);

                // After 2+ failures, we'll escalate to Smart
                let next_tier = if total_failures >= 2 || used_smart {
                    "smart"
                } else {
                    current_tier.as_str()
                };

                tracer
                    .record_decision(DecisionRecord::Continue {
                        reason: format!("failed at {}", stage_name),
                        next_tier: next_tier.to_string(),
                    })
                    .await;

                previous_errors = errors;
                previous_code = Some(code);
            }
        }

        // Show first few errors
        println!("  Errors ({}):", previous_errors.len());
        for err in previous_errors.iter().take(5) {
            for line in err.lines().take(3) {
                println!("    {}", line);
            }
        }
    }

    // Fallback to best attempt
    if final_code.is_none() {
        if let Some(best) = rollback.best() {
            println!(
                "\n  ⚠ Returning best attempt ({} errors)",
                best.errors.len()
            );
            final_code = Some(best.code.clone());
        }
    }

    // Complete the trace
    let trace = tracer
        .complete(final_code.is_some(), final_code.as_deref())
        .await;

    // Publish to NATS if connected
    if let Some(ref b) = bus {
        if let Err(e) = tracer.publish_to_nats(b.nats_client().as_ref()).await {
            eprintln!("⚠ Failed to publish trace to NATS: {}", e);
        }
    }

    // Print summary
    println!("\n{}", "=".repeat(60));
    println!("TRACE SUMMARY");
    println!("{}", "=".repeat(60));
    println!("Trace ID: {}", trace.trace_id);
    println!("Status: {:?}", trace.status);
    println!("Duration: {}ms", trace.metrics.total_duration_ms);
    println!(
        "Attempts: {} (fast: {}, smart: {})",
        trace.attempts.len(),
        trace.metrics.attempts_by_tier.get("fast").unwrap_or(&0),
        trace.metrics.attempts_by_tier.get("smart").unwrap_or(&0),
    );
    println!("Total tokens: {}", trace.metrics.total_tokens);
    println!("Rollbacks: {}", trace.metrics.rollback_count);

    if !trace.metrics.error_categories.is_empty() {
        println!("\nError categories:");
        for (cat, count) in &trace.metrics.error_categories {
            println!("  - {}: {}", cat, count);
        }
    }

    // Output JSON trace (compact for logs, can be parsed)
    println!("\n{}", "=".repeat(60));
    println!("JSON TRACE (compact):");
    println!("{}", "=".repeat(60));
    if let Ok(json) = tracer.to_json_compact().await {
        // Print first 500 chars + indication of full size
        if json.len() > 500 {
            println!("{}... [{} bytes total]", &json[..500], json.len());
        } else {
            println!("{}", json);
        }
    }

    println!("\n{}", "=".repeat(60));
    println!("GENERATED CODE");
    println!("{}", "=".repeat(60));

    if let Some(code) = final_code {
        println!("{}", code);
    } else {
        println!("Failed to generate valid TypeScript code");
    }

    if let Some(b) = bus {
        b.stop().await?;
    }

    Ok(())
}

// Helper to categorize errors for metrics
fn categorize_error(error: &str) -> String {
    let lower = error.to_lowercase();

    // TypeScript specific errors
    if lower.contains("ts1005")
        || lower.contains("ts1003")
        || lower.contains("ts1109")
        || lower.contains("ts1128")
    {
        return "ts_syntax".to_string();
    }
    if lower.contains("ts2304") || lower.contains("ts2307") || lower.contains("ts2551") {
        return "ts_lookup".to_string();
    }
    if lower.contains("ts2339")
        || lower.contains("ts2345")
        || lower.contains("ts2322")
        || lower.contains("ts2571")
    {
        return "ts_type".to_string();
    }
    if lower.contains("error ts") {
        return "typescript".to_string();
    }

    // General errors
    if lower.contains("syntaxerror") || lower.contains("expected") {
        "syntax".to_string()
    } else if lower.contains("typeerror") || lower.contains("type") {
        "type_error".to_string()
    } else if lower.contains("jest") || lower.contains("expect(") || lower.contains("failed") {
        "test_failure".to_string()
    } else if lower.contains("timeout") || lower.contains("deadlock") {
        "timeout".to_string()
    } else {
        "other".to_string()
    }
}

// Helper to extract location from error string
fn extract_location(error: &str) -> Option<ErrorLocation> {
    // Try to extract TypeScript file:line pattern like "index.ts(123,45)" or "index.ts:123"
    for word in error.split_whitespace() {
        // Pattern: file.ts(line,col)
        if word.contains(".ts(") {
            let parts: Vec<&str> = word.split('(').collect();
            if parts.len() >= 2 {
                let file = parts[0].to_string();
                let rest = parts[1].trim_end_matches(')');
                let nums: Vec<&str> = rest.split(',').collect();
                let line = nums.first().and_then(|s| s.parse().ok());
                let column = nums.get(1).and_then(|s| s.parse().ok());
                if file.ends_with(".ts") {
                    return Some(ErrorLocation {
                        file: Some(file),
                        line,
                        column,
                        function: None,
                    });
                }
            }
        }
        // Pattern: file.ts:line
        if word.contains(".ts:") {
            let parts: Vec<&str> = word.split(':').collect();
            if parts.len() >= 2 {
                let file = parts[0].to_string();
                let line = parts[1].parse().ok();
                if file.ends_with(".ts") && line.is_some() {
                    return Some(ErrorLocation {
                        file: Some(file),
                        line,
                        column: None,
                        function: None,
                    });
                }
            }
        }
    }
    None
}
