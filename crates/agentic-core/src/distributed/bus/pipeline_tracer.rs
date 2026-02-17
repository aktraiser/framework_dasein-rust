//! Pipeline Tracer - Unified JSON document for pipeline execution tracking.
//!
//! Replaces event-based audit with a single structured JSON document that
//! captures the entire pipeline execution with rich metadata.
//!
//! # Architecture
//!
//! ```text
//! Pipeline Start ──► PipelineTracer::new()
//!        │
//!        ▼
//! Attempt 1 ──────► tracer.start_attempt(...)
//!   │                     │
//!   ├─ Generation ───────► tracer.record_generation(...)
//!   ├─ Validation ───────► tracer.record_validation(...)
//!   └─ Decision ─────────► tracer.record_decision(...)
//!        │
//!        ▼
//! Attempt N...
//!        │
//!        ▼
//! Pipeline End ───► tracer.complete() ──► JSON Document
//!                                              │
//!                                    ┌────────┴────────┐
//!                                    ▼                 ▼
//!                               NATS/File          Analytics
//! ```
//!
//! # Example
//!
//! ```rust,no_run
//! let tracer = PipelineTracer::new("python", "Create a rate limiter...");
//!
//! tracer.start_attempt(1, ModelTier::Fast);
//! tracer.record_generation(GenerationRecord { ... });
//! tracer.record_validation(ValidationRecord { ... });
//! tracer.record_decision(Decision::Continue { ... });
//!
//! let trace = tracer.complete(true);
//! println!("{}", serde_json::to_string_pretty(&trace)?);
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::error_fingerprint::ModelTier;
use super::nats_client::NatsClient;
use super::types::BusError;

/// Schema version for forward compatibility.
pub const TRACE_SCHEMA_VERSION: &str = "1.0";

// =============================================================================
// Main Trace Document
// =============================================================================

/// Complete trace of a pipeline execution.
/// This is the main JSON document that captures everything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineTrace {
    /// Schema version for compatibility
    pub version: String,

    /// Unique trace identifier
    pub trace_id: String,

    /// When the pipeline started
    pub started_at: DateTime<Utc>,

    /// When the pipeline completed (None if still running)
    pub completed_at: Option<DateTime<Utc>>,

    /// Final status
    pub status: TraceStatus,

    /// Pipeline configuration
    pub config: TraceConfig,

    /// All attempts made during execution
    pub attempts: Vec<AttemptRecord>,

    /// Final result summary
    pub result: Option<TraceResult>,

    /// Aggregated metrics
    pub metrics: TraceMetrics,

    /// Additional metadata (extensible)
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Pipeline execution status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceStatus {
    /// Pipeline is currently running
    Running,
    /// Pipeline completed successfully
    Success,
    /// Pipeline completed with partial success (best effort returned)
    PartialSuccess,
    /// Pipeline failed completely
    Failed,
    /// Pipeline was cancelled
    Cancelled,
}

impl TraceStatus {
    pub fn is_terminal(&self) -> bool {
        !matches!(self, TraceStatus::Running)
    }
}

// =============================================================================
// Configuration
// =============================================================================

/// Pipeline configuration snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceConfig {
    /// The task/prompt being executed
    pub task: String,

    /// Task hash for deduplication
    pub task_hash: String,

    /// Target programming language
    pub language: String,

    /// Maximum retry attempts
    pub max_retries: u32,

    /// Model configurations
    pub models: ModelConfig,

    /// Validators in the pipeline
    pub validators: Vec<String>,

    /// Additional config options
    #[serde(default)]
    pub options: HashMap<String, serde_json::Value>,
}

/// Model tier configurations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub fast: ModelInfo,
    pub smart: ModelInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expert: Option<ModelInfo>,
}

/// Individual model info.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub provider: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

// =============================================================================
// Attempt Records
// =============================================================================

/// Record of a single attempt in the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptRecord {
    /// Attempt number (1-indexed)
    pub number: u32,

    /// Which model tier was used
    pub model_tier: String,

    /// When this attempt started
    pub started_at: DateTime<Utc>,

    /// When this attempt ended
    pub ended_at: Option<DateTime<Utc>>,

    /// Duration in milliseconds
    pub duration_ms: Option<u64>,

    /// Generation details
    pub generation: Option<GenerationRecord>,

    /// Validation results
    pub validation: Option<ValidationRecord>,

    /// Decision made after this attempt
    pub decision: Option<DecisionRecord>,

    /// Code snapshot (hash + preview)
    pub code_snapshot: Option<CodeSnapshot>,
}

/// LLM generation details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerationRecord {
    /// Model used
    pub model: String,

    /// Tokens consumed
    pub tokens: TokenUsage,

    /// Characters generated
    pub chars_generated: usize,

    /// Whether output was truncated
    pub truncated: bool,

    /// Generation duration in ms
    pub duration_ms: u64,
}

/// Token usage breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
    pub total: u32,
}

/// Validation results from all validators.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationRecord {
    /// Overall pass/fail
    pub passed: bool,

    /// Quality score
    pub score: i32,

    /// Results from each validator
    pub stages: Vec<ValidatorStageRecord>,

    /// Total validation duration
    pub duration_ms: u64,
}

/// Result from a single validator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorStageRecord {
    /// Validator name
    pub validator: String,

    /// Did this validator pass?
    pub passed: bool,

    /// Duration for this validator
    pub duration_ms: u64,

    /// Errors found (if any)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<EnrichedError>,

    /// Recommendations generated
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommendations: Vec<String>,

    /// Documentation fetched (if any)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub documentation: Vec<DocReference>,
}

/// Enriched error with full context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichedError {
    /// Unique error ID within this trace
    pub id: String,

    /// Error severity
    pub severity: ErrorSeverity,

    /// Error category for fingerprinting
    pub category: String,

    /// Location information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<ErrorLocation>,

    /// Original error message
    pub message: String,

    /// Analyzed problem description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis: Option<ErrorAnalysis>,

    /// Actionable hints
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
}

/// Error severity levels.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorSeverity {
    Critical,
    Error,
    Warning,
    Info,
}

/// Error location in code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorLocation {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub function: Option<String>,
}

/// Error analysis with expected/actual values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorAnalysis {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

/// Reference to fetched documentation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocReference {
    pub source: String,
    pub topic: String,
    pub url: Option<String>,
    /// Preview of the doc content
    pub preview: String,
}

/// Decision made after an attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum DecisionRecord {
    /// Continue to next attempt
    Continue { reason: String, next_tier: String },
    /// Rollback to best previous attempt
    Rollback {
        reason: String,
        rollback_to_attempt: u32,
        best_score: i32,
    },
    /// Accept the result
    Accept { final_score: i32 },
    /// Reject and stop
    Reject { reason: String },
}

/// Code snapshot for replay/debugging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeSnapshot {
    /// MD5 hash of the code
    pub hash: String,
    /// Code length in bytes
    pub length: usize,
    /// First N characters for preview
    pub preview: String,
    /// Full code (optional, for replay mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_code: Option<String>,
}

// =============================================================================
// Result & Metrics
// =============================================================================

/// Final result summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceResult {
    /// Did the pipeline succeed?
    pub success: bool,

    /// Which attempt produced the final result
    pub final_attempt: u32,

    /// Final quality score
    pub final_score: i32,

    /// Number of remaining errors (0 if success)
    pub remaining_errors: usize,

    /// Final code hash
    pub code_hash: String,

    /// Final code (if store_code is enabled)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_code: Option<String>,
}

/// Aggregated metrics for the pipeline execution.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TraceMetrics {
    /// Total execution time in ms
    pub total_duration_ms: u64,

    /// Total tokens used across all attempts
    pub total_tokens: u32,

    /// Token breakdown by tier
    pub tokens_by_tier: HashMap<String, u32>,

    /// Number of attempts by tier
    pub attempts_by_tier: HashMap<String, u32>,

    /// Number of rollbacks
    pub rollback_count: u32,

    /// Error categories encountered
    pub error_categories: HashMap<String, u32>,

    /// Validators that failed
    pub failed_validators: HashMap<String, u32>,
}

// =============================================================================
// Pipeline Tracer (Builder)
// =============================================================================

/// Builder for constructing a PipelineTrace incrementally.
pub struct PipelineTracer {
    trace: Arc<RwLock<PipelineTrace>>,
    current_attempt: Arc<RwLock<Option<AttemptRecord>>>,
    store_full_code: bool,
}

impl PipelineTracer {
    /// Create a new tracer for a pipeline execution.
    pub fn new(language: &str, task: &str) -> Self {
        let trace_id = uuid::Uuid::new_v4().to_string();
        let task_hash = format!("{:x}", md5::compute(task.as_bytes()));

        let trace = PipelineTrace {
            version: TRACE_SCHEMA_VERSION.to_string(),
            trace_id,
            started_at: Utc::now(),
            completed_at: None,
            status: TraceStatus::Running,
            config: TraceConfig {
                task: task.to_string(),
                task_hash,
                language: language.to_string(),
                max_retries: 8,
                models: ModelConfig {
                    fast: ModelInfo {
                        provider: "unknown".to_string(),
                        model: "unknown".to_string(),
                        temperature: None,
                        max_tokens: None,
                    },
                    smart: ModelInfo {
                        provider: "unknown".to_string(),
                        model: "unknown".to_string(),
                        temperature: None,
                        max_tokens: None,
                    },
                    expert: None,
                },
                validators: vec![],
                options: HashMap::new(),
            },
            attempts: vec![],
            result: None,
            metrics: TraceMetrics::default(),
            metadata: HashMap::new(),
        };

        Self {
            trace: Arc::new(RwLock::new(trace)),
            current_attempt: Arc::new(RwLock::new(None)),
            store_full_code: false,
        }
    }

    /// Enable storing full code in snapshots (for replay).
    pub fn with_full_code(mut self) -> Self {
        self.store_full_code = true;
        self
    }

    /// Get the trace ID.
    pub async fn trace_id(&self) -> String {
        self.trace.read().await.trace_id.clone()
    }

    /// Configure models.
    pub async fn set_models(&self, fast: ModelInfo, smart: ModelInfo) {
        let mut trace = self.trace.write().await;
        trace.config.models.fast = fast;
        trace.config.models.smart = smart;
    }

    /// Configure validators.
    pub async fn set_validators(&self, validators: Vec<String>) {
        let mut trace = self.trace.write().await;
        trace.config.validators = validators;
    }

    /// Set max retries.
    pub async fn set_max_retries(&self, max: u32) {
        let mut trace = self.trace.write().await;
        trace.config.max_retries = max;
    }

    /// Add metadata.
    pub async fn add_metadata(&self, key: &str, value: serde_json::Value) {
        let mut trace = self.trace.write().await;
        trace.metadata.insert(key.to_string(), value);
    }

    // =========================================================================
    // Attempt Recording
    // =========================================================================

    /// Start a new attempt.
    pub async fn start_attempt(&self, number: u32, tier: ModelTier) {
        // Finalize previous attempt if any
        self.finalize_current_attempt().await;

        let attempt = AttemptRecord {
            number,
            model_tier: tier.as_str().to_string(),
            started_at: Utc::now(),
            ended_at: None,
            duration_ms: None,
            generation: None,
            validation: None,
            decision: None,
            code_snapshot: None,
        };

        *self.current_attempt.write().await = Some(attempt);

        // Update metrics
        let mut trace = self.trace.write().await;
        *trace
            .metrics
            .attempts_by_tier
            .entry(tier.as_str().to_string())
            .or_insert(0) += 1;
    }

    /// Record generation result.
    pub async fn record_generation(&self, record: GenerationRecord) {
        if let Some(ref mut attempt) = *self.current_attempt.write().await {
            // Update metrics
            {
                let mut trace = self.trace.write().await;
                trace.metrics.total_tokens += record.tokens.total;
                *trace
                    .metrics
                    .tokens_by_tier
                    .entry(attempt.model_tier.clone())
                    .or_insert(0) += record.tokens.total;
            }

            attempt.generation = Some(record);
        }
    }

    /// Record validation result.
    pub async fn record_validation(&self, record: ValidationRecord) {
        if let Some(ref mut attempt) = *self.current_attempt.write().await {
            // Update error category metrics
            {
                let mut trace = self.trace.write().await;
                for stage in &record.stages {
                    if !stage.passed {
                        *trace
                            .metrics
                            .failed_validators
                            .entry(stage.validator.clone())
                            .or_insert(0) += 1;
                    }
                    for error in &stage.errors {
                        *trace
                            .metrics
                            .error_categories
                            .entry(error.category.clone())
                            .or_insert(0) += 1;
                    }
                }
            }

            attempt.validation = Some(record);
        }
    }

    /// Record decision.
    pub async fn record_decision(&self, decision: DecisionRecord) {
        if let Some(ref mut attempt) = *self.current_attempt.write().await {
            // Update rollback count
            if matches!(decision, DecisionRecord::Rollback { .. }) {
                let mut trace = self.trace.write().await;
                trace.metrics.rollback_count += 1;
            }

            attempt.decision = Some(decision);
        }
    }

    /// Record code snapshot.
    pub async fn record_code(&self, code: &str) {
        if let Some(ref mut attempt) = *self.current_attempt.write().await {
            let hash = format!("{:x}", md5::compute(code.as_bytes()));
            let preview = if code.len() > 500 {
                format!("{}...", &code[..500])
            } else {
                code.to_string()
            };

            attempt.code_snapshot = Some(CodeSnapshot {
                hash,
                length: code.len(),
                preview,
                full_code: if self.store_full_code {
                    Some(code.to_string())
                } else {
                    None
                },
            });
        }
    }

    /// Finalize current attempt and add to trace.
    async fn finalize_current_attempt(&self) {
        let mut current = self.current_attempt.write().await;
        if let Some(mut attempt) = current.take() {
            attempt.ended_at = Some(Utc::now());
            attempt.duration_ms =
                Some((attempt.ended_at.unwrap() - attempt.started_at).num_milliseconds() as u64);

            let mut trace = self.trace.write().await;
            trace.attempts.push(attempt);
        }
    }

    // =========================================================================
    // Completion
    // =========================================================================

    /// Complete the pipeline and return the final trace.
    pub async fn complete(&self, success: bool, final_code: Option<&str>) -> PipelineTrace {
        // Finalize any pending attempt
        self.finalize_current_attempt().await;

        let mut trace = self.trace.write().await;

        trace.completed_at = Some(Utc::now());
        trace.status = if success {
            TraceStatus::Success
        } else if trace.attempts.iter().any(|a| {
            a.validation
                .as_ref()
                .map(|v| v.score > -1000)
                .unwrap_or(false)
        }) {
            TraceStatus::PartialSuccess
        } else {
            TraceStatus::Failed
        };

        // Calculate total duration
        trace.metrics.total_duration_ms =
            (trace.completed_at.unwrap() - trace.started_at).num_milliseconds() as u64;

        // Build result
        if let Some(code) = final_code {
            let best_attempt = trace
                .attempts
                .iter()
                .filter(|a| a.validation.is_some())
                .max_by_key(|a| a.validation.as_ref().map(|v| v.score).unwrap_or(i32::MIN));

            let final_attempt = best_attempt.map(|a| a.number).unwrap_or(1);
            let final_score = best_attempt
                .and_then(|a| a.validation.as_ref())
                .map(|v| v.score)
                .unwrap_or(0);
            let remaining_errors = best_attempt
                .and_then(|a| a.validation.as_ref())
                .map(|v| v.stages.iter().flat_map(|s| &s.errors).count())
                .unwrap_or(0);

            trace.result = Some(TraceResult {
                success,
                final_attempt,
                final_score,
                remaining_errors,
                code_hash: format!("{:x}", md5::compute(code.as_bytes())),
                final_code: if self.store_full_code {
                    Some(code.to_string())
                } else {
                    None
                },
            });
        }

        trace.clone()
    }

    /// Cancel the pipeline.
    pub async fn cancel(&self, reason: &str) -> PipelineTrace {
        self.finalize_current_attempt().await;

        let mut trace = self.trace.write().await;
        trace.completed_at = Some(Utc::now());
        trace.status = TraceStatus::Cancelled;
        trace
            .metadata
            .insert("cancel_reason".to_string(), serde_json::json!(reason));
        trace.metrics.total_duration_ms =
            (trace.completed_at.unwrap() - trace.started_at).num_milliseconds() as u64;

        trace.clone()
    }

    /// Get current trace state (for debugging/monitoring).
    pub async fn current_state(&self) -> PipelineTrace {
        self.trace.read().await.clone()
    }

    // =========================================================================
    // Persistence
    // =========================================================================

    /// Publish the trace to NATS.
    pub async fn publish_to_nats(&self, nats: &NatsClient) -> Result<(), BusError> {
        let trace = self.trace.read().await;
        let subject = format!("pipeline.trace.{}", trace.trace_id);
        let ack = nats.publish_jetstream(&subject, &*trace).await?;
        // Wait for acknowledgment
        ack.await
            .map_err(|e| BusError::PublishFailed(e.to_string()))?;
        Ok(())
    }

    /// Export trace as pretty JSON string.
    pub async fn to_json(&self) -> Result<String, serde_json::Error> {
        let trace = self.trace.read().await;
        serde_json::to_string_pretty(&*trace)
    }

    /// Export trace as compact JSON.
    pub async fn to_json_compact(&self) -> Result<String, serde_json::Error> {
        let trace = self.trace.read().await;
        serde_json::to_string(&*trace)
    }
}

// =============================================================================
// Helper implementations
// =============================================================================

impl From<&str> for ErrorSeverity {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "critical" | "fatal" => ErrorSeverity::Critical,
            "error" => ErrorSeverity::Error,
            "warning" | "warn" => ErrorSeverity::Warning,
            _ => ErrorSeverity::Info,
        }
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_tracer_basic_flow() {
        let tracer = PipelineTracer::new("python", "Create a hello world");

        // Configure
        tracer
            .set_models(
                ModelInfo {
                    provider: "gemini".to_string(),
                    model: "gemini-2.0-flash".to_string(),
                    temperature: Some(0.2),
                    max_tokens: Some(8192),
                },
                ModelInfo {
                    provider: "anthropic".to_string(),
                    model: "claude-haiku".to_string(),
                    temperature: Some(0.2),
                    max_tokens: Some(8192),
                },
            )
            .await;

        // Attempt 1
        tracer.start_attempt(1, ModelTier::Fast).await;
        tracer
            .record_generation(GenerationRecord {
                model: "gemini-2.0-flash".to_string(),
                tokens: TokenUsage {
                    prompt: 100,
                    completion: 50,
                    total: 150,
                },
                chars_generated: 500,
                truncated: false,
                duration_ms: 1000,
            })
            .await;
        tracer
            .record_validation(ValidationRecord {
                passed: true,
                score: 0,
                stages: vec![],
                duration_ms: 500,
            })
            .await;
        tracer
            .record_decision(DecisionRecord::Accept { final_score: 0 })
            .await;
        tracer.record_code("print('hello')").await;

        // Complete
        let trace = tracer.complete(true, Some("print('hello')")).await;

        assert_eq!(trace.status, TraceStatus::Success);
        assert_eq!(trace.attempts.len(), 1);
        assert_eq!(trace.metrics.total_tokens, 150);
        assert!(trace.result.is_some());

        // Verify JSON serialization
        let json = serde_json::to_string_pretty(&trace).unwrap();
        assert!(json.contains("\"version\": \"1.0\""));
        assert!(json.contains("\"language\": \"python\""));
    }

    #[tokio::test]
    async fn test_tracer_with_errors() {
        let tracer = PipelineTracer::new("rust", "Create a scheduler");

        tracer.start_attempt(1, ModelTier::Fast).await;
        tracer
            .record_validation(ValidationRecord {
                passed: false,
                score: -300,
                stages: vec![ValidatorStageRecord {
                    validator: "sandbox".to_string(),
                    passed: false,
                    duration_ms: 5000,
                    errors: vec![EnrichedError {
                        id: "err-001".to_string(),
                        severity: ErrorSeverity::Error,
                        category: "type_system".to_string(),
                        location: Some(ErrorLocation {
                            file: Some("lib.rs".to_string()),
                            line: Some(42),
                            column: None,
                            function: Some("main".to_string()),
                        }),
                        message: "mismatched types".to_string(),
                        analysis: None,
                        hints: vec!["Check return type".to_string()],
                    }],
                    recommendations: vec![],
                    documentation: vec![],
                }],
                duration_ms: 5000,
            })
            .await;

        let trace = tracer.complete(false, None).await;

        // Score -300 > -1000 means PartialSuccess (some progress was made)
        assert_eq!(trace.status, TraceStatus::PartialSuccess);
        assert_eq!(
            *trace.metrics.error_categories.get("type_system").unwrap(),
            1
        );
    }
}
