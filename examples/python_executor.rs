//! Python Code Generation - Full Pipeline with JSON Tracing
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
//! GEMINI_API_KEY=xxx ANTHROPIC_API_KEY=xxx cargo run --example python_executor
//! ```

use dasein_agentic_core::distributed::{
    bus::{
        BusCoordinator, DecisionRecord, EnrichedError, ErrorFingerprinter, ErrorLocation,
        ErrorSeverity, GenerationRecord, ModelInfo, ModelTier, PipelineTracer, RollbackDecision,
        RollbackManager, TokenUsage, ValidationRecord, ValidatorStageRecord,
    },
    CodeAssembler, ErrorEnricherValidator, Executor, SandboxPipelineValidator, ValidatorInput,
    ValidatorPipeline,
};
use dasein_agentic_sandbox::FirecrackerSandbox;
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
    println!("  PYTHON HARDCORE - Distributed Rate Limiter");
    println!("  Full Pipeline: JSON Trace + Fingerprinting + Rollback");
    println!("{}\n", "=".repeat(60));

    // === Task === HARDCORE MODE
    let task = r#"Create a Python distributed rate limiter with the following components:

1. TokenBucket class:
   - __init__(self, capacity: int, refill_rate: float) - tokens per second refill
   - acquire(self, tokens: int = 1) -> bool - try to acquire tokens, return success
   - wait_and_acquire(self, tokens: int = 1) -> float - block until acquired, return wait time

2. SlidingWindowCounter class:
   - __init__(self, window_size: float, max_requests: int)
   - allow(self) -> bool - check if request is allowed in current window
   - get_stats(self) -> Dict with current_count, window_start, remaining

3. CircuitBreaker class:
   - States: CLOSED, OPEN, HALF_OPEN
   - __init__(self, failure_threshold: int, recovery_timeout: float, half_open_max: int)
   - call(self, func: Callable) -> Any - execute with circuit breaker pattern
   - Automatic state transitions based on success/failure counts

4. AdaptiveRateLimiter class combining all three:
   - __init__(self, base_rate: int, burst_capacity: int, window_size: float)
   - acquire(self) -> Tuple[bool, Dict] - returns (allowed, stats_dict)
   - Should automatically back off when circuit breaker opens
   - Track metrics: total_requests, allowed, rejected, circuit_state

Requirements:
- Thread-safe using threading.Lock
- Type hints throughout
- Comprehensive pytest tests including:
  * Token bucket refill behavior
  * Sliding window edge cases
  * Circuit breaker state machine (all transitions)
  * Adaptive limiter backoff behavior
- Use time.monotonic() for timing
- Handle edge cases: zero tokens, negative values

The code must be production-quality with proper error handling.
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
    let tracer = Arc::new(PipelineTracer::new("python", task));
    println!(
        "✓ Pipeline Tracer enabled (trace: {})",
        &tracer.trace_id().await[..8]
    );

    let fingerprinter = ErrorFingerprinter::new();

    // === Setup Executors ===
    let fast_model = std::env::var("FAST_MODEL").unwrap_or_else(|_| "gemini-2.0-flash".to_string());
    let fast_executor = Executor::new("py-fast", "supervisor")
        .llm_gemini(&fast_model)
        .build();
    println!("✓ Fast model: {}", fast_model);

    let smart_model =
        std::env::var("SMART_MODEL").unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());
    let smart_executor = Executor::new("py-smart", "supervisor")
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

    // Sandbox validator for Python
    let sandbox = FirecrackerSandbox::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build();
    let validator = SandboxPipelineValidator::new(sandbox)
        .workspace(PathBuf::from("/tmp/python-validation"))
        .run_tests(true);
    let pipeline = ValidatorPipeline::new()
        .add(validator)
        .add(ErrorEnricherValidator::new());

    tracer
        .set_validators(vec!["sandbox".to_string(), "error-enricher".to_string()])
        .await;
    tracer.set_max_retries(MAX_RETRIES).await;
    println!("✓ Error Enricher enabled");

    // === Generate with full pipeline ===
    let _total_start = Instant::now();
    let system = "You are an expert Python developer. Return ONLY valid Python code. No markdown, no explanations. Include pytest tests at the bottom of the file.";

    let mut previous_errors: Vec<String> = vec![];
    let mut previous_code: Option<String> = None;
    let mut rollback = RollbackManager::new();
    let mut current_tier = ModelTier::Fast;
    let mut total_failures = 0; // Track all failures
    let mut used_smart = false; // Once we use Smart, don't go back to Fast

    let mut final_code: Option<String> = None;

    for attempt in 1..=MAX_RETRIES {
        // Start attempt in tracer
        tracer.start_attempt(attempt, current_tier).await;

        // Select executor based on fingerprinting
        let executor = if !previous_errors.is_empty() {
            let analysis = fingerprinter.analyze(&previous_errors);
            current_tier = analysis.recommended_tier;

            // Escalation: after 3+ failures, always use Smart
            if total_failures >= 3 {
                current_tier = ModelTier::Smart;
                used_smart = true;
            }

            // Once we've used Smart, stay on Smart
            if used_smart && current_tier == ModelTier::Fast {
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
            ModelTier::Fast => "⚡",
            ModelTier::Smart => "🧠",
            ModelTier::Expert => "🎓",
        };

        println!("\n[Attempt {}/{}] {}", attempt, MAX_RETRIES, tier_indicator);

        // Build prompt with error feedback
        let prompt = if previous_errors.is_empty() {
            task.to_string()
        } else {
            let hints = fingerprinter.generate_hint(&fingerprinter.analyze(&previous_errors));
            format!(
                "{}\n\n=== PREVIOUS ERRORS ===\n{}\n\n=== HINTS ===\n{}\n\n=== YOUR PREVIOUS CODE ===\n{}\n\nFix the errors and return complete corrected code.",
                task,
                previous_errors.join("\n"),
                hints,
                previous_code.as_deref().unwrap_or("(not available)")
            )
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

        // Validate
        let val_start = Instant::now();
        let input = ValidatorInput::new(&code, "python").with_task(task);
        let val_result = pipeline.validate(input).await;
        let val_duration = val_start.elapsed().as_millis() as u64;

        // Convert errors to enriched format for tracer
        let errors: Vec<String> = val_result
            .results
            .iter()
            .flat_map(|r| r.errors.clone())
            .collect();
        let enriched_errors: Vec<EnrichedError> = errors
            .iter()
            .enumerate()
            .map(|(i, e)| EnrichedError {
                id: format!("err-{}-{}", attempt, i),
                severity: if e.contains("FAILED") {
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

        // Record validation in tracer
        tracer
            .record_validation(ValidationRecord {
                passed: val_result.passed,
                score,
                stages: vec![ValidatorStageRecord {
                    validator: "sandbox+enricher".to_string(),
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
            println!("  ✓ Validation passed!");

            tracer
                .record_decision(DecisionRecord::Accept { final_score: 0 })
                .await;

            final_code = Some(code);
            break;
        }

        // Record attempt and check rollback
        let decision = rollback.record(attempt, code.clone(), errors.clone());

        // Track all failures
        total_failures += 1;

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
                let score_info = format!("score: {}", current_score.quality_score());
                println!("  ✗ {} errors ({})", errors.len(), score_info);

                // After 2+ failures, we'll escalate to Smart
                let next_tier = if total_failures >= 2 || used_smart {
                    "smart"
                } else {
                    current_tier.as_str()
                };

                tracer
                    .record_decision(DecisionRecord::Continue {
                        reason: "progressing".to_string(),
                        next_tier: next_tier.to_string(),
                    })
                    .await;

                previous_errors = errors;
                previous_code = Some(code);
            }
        }

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
        println!("Failed to generate valid Python code");
    }

    if let Some(b) = bus {
        b.stop().await?;
    }

    Ok(())
}

// Helper to categorize errors for metrics
fn categorize_error(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("assertionerror") || lower.contains("assert") {
        "assertion".to_string()
    } else if lower.contains("typeerror") || lower.contains("type") {
        "type_error".to_string()
    } else if lower.contains("syntaxerror") || lower.contains("syntax") {
        "syntax".to_string()
    } else if lower.contains("nameerror") || lower.contains("undefined") {
        "name_error".to_string()
    } else if lower.contains("timeout") || lower.contains("deadlock") {
        "timeout".to_string()
    } else if lower.contains("failed") {
        "test_failure".to_string()
    } else {
        "other".to_string()
    }
}

// Helper to extract location from error string
fn extract_location(error: &str) -> Option<ErrorLocation> {
    // Try to extract file:line pattern like "test_main.py:123"
    for word in error.split_whitespace() {
        if word.contains(".py:") {
            let parts: Vec<&str> = word.split(':').collect();
            if parts.len() >= 2 {
                let file = parts[0].to_string();
                let line = parts[1].parse().ok();
                if file.ends_with(".py") && line.is_some() {
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
