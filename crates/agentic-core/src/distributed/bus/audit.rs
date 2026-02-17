//! Audit Trail - Persistent event logging with replay capability.
//!
//! Provides the 4 observability guarantees:
//! 1. **Replay**: Replay an execution identically from JetStream
//! 2. **Audit**: Explain any decision with full context
//! 3. **Rollback**: Already handled by RollbackManager
//! 4. **Sandbox**: Already handled by agentic-sandbox
//!
//! # Architecture
//!
//! ```text
//! Pipeline ──► AuditCollector ──► NATS JetStream (AGENTIC_AUDIT stream)
//!                                       │
//!                                       ▼
//!                              ┌────────────────┐
//!                              │   Consumer     │
//!                              │  (replay/query)│
//!                              └────────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::bus::{AuditCollector, AuditEvent, AuditEventType};
//!
//! // Create with NATS connection
//! let audit = AuditCollector::new(nats_client).await?;
//!
//! // Emit events
//! audit.emit(AuditEvent::stage_started("trace-123", Stage::Types, 1)).await?;
//! audit.emit(AuditEvent::model_selected("trace-123", ModelTier::Fast, "simple errors")).await?;
//!
//! // Query events for a trace
//! let events = audit.for_trace("trace-123").await?;
//!
//! // Replay from a specific point
//! let replay = audit.replay_from("trace-123", sequence_num).await?;
//! ```

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;

use super::error_fingerprint::ModelTier;
use super::nats_client::{NatsClient, StreamConfig};
use super::types::BusError;

/// Stream name for audit events.
pub const AUDIT_STREAM: &str = "AGENTIC_AUDIT";

/// Subject prefix for audit events.
pub const AUDIT_SUBJECT: &str = "audit";

/// Audit event types for the Progressive Locking pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuditEventType {
    /// Pipeline execution started.
    PipelineStarted {
        task_hash: String,
        task_preview: String,
    },

    /// Pipeline execution completed.
    PipelineCompleted {
        success: bool,
        duration_ms: u64,
        stages_completed: u32,
    },

    /// A stage started.
    StageStarted { stage: String, attempt: u32 },

    /// A stage completed.
    StageCompleted {
        stage: String,
        attempt: u32,
        success: bool,
        code_hash: String,
        code_length: usize,
    },

    /// Model was selected based on error fingerprinting.
    ModelSelected {
        tier: String,
        reason: String,
        error_categories: Vec<String>,
    },

    /// LLM generation requested.
    LlmRequested { model: String, prompt_tokens: usize },

    /// LLM generation completed.
    LlmCompleted {
        model: String,
        completion_tokens: usize,
        duration_ms: u64,
    },

    /// Validation started.
    ValidationStarted { validator: String },

    /// Validation completed.
    ValidationCompleted {
        validator: String,
        passed: bool,
        error_count: usize,
        errors_preview: Vec<String>,
    },

    /// Rollback decision made.
    RollbackDecision {
        decision: String,
        current_score: i32,
        best_score: Option<i32>,
        reason: String,
    },

    /// Code snapshot for replay.
    CodeSnapshot {
        stage: String,
        attempt: u32,
        code_hash: String,
        /// First 500 chars for quick preview
        code_preview: String,
    },

    /// Error encountered.
    Error {
        component: String,
        message: String,
        recoverable: bool,
    },
}

/// A single audit event with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    /// Unique event ID.
    pub id: String,

    /// Trace ID - groups all events for one pipeline execution.
    pub trace_id: String,

    /// Sequence number within the trace.
    pub sequence: u64,

    /// Event timestamp.
    pub timestamp: chrono::DateTime<chrono::Utc>,

    /// The event data.
    pub event: AuditEventType,

    /// Additional context.
    #[serde(default)]
    pub context: serde_json::Value,
}

impl AuditEvent {
    /// Create a new audit event.
    pub fn new(trace_id: impl Into<String>, sequence: u64, event: AuditEventType) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            trace_id: trace_id.into(),
            sequence,
            timestamp: chrono::Utc::now(),
            event,
            context: serde_json::Value::Null,
        }
    }

    /// Add context to the event.
    pub fn with_context(mut self, context: serde_json::Value) -> Self {
        self.context = context;
        self
    }

    // === Convenience constructors ===

    pub fn pipeline_started(trace_id: &str, seq: u64, task: &str) -> Self {
        let task_hash = format!("{:x}", md5::compute(task.as_bytes()));
        let task_preview = if task.len() > 100 {
            format!("{}...", &task[..100])
        } else {
            task.to_string()
        };

        Self::new(
            trace_id,
            seq,
            AuditEventType::PipelineStarted {
                task_hash,
                task_preview,
            },
        )
    }

    pub fn pipeline_completed(
        trace_id: &str,
        seq: u64,
        success: bool,
        duration_ms: u64,
        stages: u32,
    ) -> Self {
        Self::new(
            trace_id,
            seq,
            AuditEventType::PipelineCompleted {
                success,
                duration_ms,
                stages_completed: stages,
            },
        )
    }

    pub fn stage_started(trace_id: &str, seq: u64, stage: &str, attempt: u32) -> Self {
        Self::new(
            trace_id,
            seq,
            AuditEventType::StageStarted {
                stage: stage.to_string(),
                attempt,
            },
        )
    }

    pub fn stage_completed(
        trace_id: &str,
        seq: u64,
        stage: &str,
        attempt: u32,
        success: bool,
        code: &str,
    ) -> Self {
        let code_hash = format!("{:x}", md5::compute(code.as_bytes()));
        Self::new(
            trace_id,
            seq,
            AuditEventType::StageCompleted {
                stage: stage.to_string(),
                attempt,
                success,
                code_hash,
                code_length: code.len(),
            },
        )
    }

    pub fn model_selected(
        trace_id: &str,
        seq: u64,
        tier: ModelTier,
        reason: &str,
        categories: Vec<String>,
    ) -> Self {
        Self::new(
            trace_id,
            seq,
            AuditEventType::ModelSelected {
                tier: tier.as_str().to_string(),
                reason: reason.to_string(),
                error_categories: categories,
            },
        )
    }

    pub fn llm_requested(trace_id: &str, seq: u64, model: &str, prompt_tokens: usize) -> Self {
        Self::new(
            trace_id,
            seq,
            AuditEventType::LlmRequested {
                model: model.to_string(),
                prompt_tokens,
            },
        )
    }

    pub fn llm_completed(
        trace_id: &str,
        seq: u64,
        model: &str,
        completion_tokens: usize,
        duration_ms: u64,
    ) -> Self {
        Self::new(
            trace_id,
            seq,
            AuditEventType::LlmCompleted {
                model: model.to_string(),
                completion_tokens,
                duration_ms,
            },
        )
    }

    pub fn validation_started(trace_id: &str, seq: u64, validator: &str) -> Self {
        Self::new(
            trace_id,
            seq,
            AuditEventType::ValidationStarted {
                validator: validator.to_string(),
            },
        )
    }

    pub fn validation_completed(
        trace_id: &str,
        seq: u64,
        validator: &str,
        passed: bool,
        errors: &[String],
    ) -> Self {
        let errors_preview: Vec<String> = errors
            .iter()
            .take(3)
            .map(|e| {
                if e.len() > 100 {
                    format!("{}...", &e[..100])
                } else {
                    e.clone()
                }
            })
            .collect();

        Self::new(
            trace_id,
            seq,
            AuditEventType::ValidationCompleted {
                validator: validator.to_string(),
                passed,
                error_count: errors.len(),
                errors_preview,
            },
        )
    }

    pub fn rollback_decision(
        trace_id: &str,
        seq: u64,
        decision: &str,
        current: i32,
        best: Option<i32>,
        reason: &str,
    ) -> Self {
        Self::new(
            trace_id,
            seq,
            AuditEventType::RollbackDecision {
                decision: decision.to_string(),
                current_score: current,
                best_score: best,
                reason: reason.to_string(),
            },
        )
    }

    pub fn code_snapshot(trace_id: &str, seq: u64, stage: &str, attempt: u32, code: &str) -> Self {
        let code_hash = format!("{:x}", md5::compute(code.as_bytes()));
        let code_preview = if code.len() > 500 {
            format!("{}...", &code[..500])
        } else {
            code.to_string()
        };

        Self::new(
            trace_id,
            seq,
            AuditEventType::CodeSnapshot {
                stage: stage.to_string(),
                attempt,
                code_hash,
                code_preview,
            },
        )
    }

    pub fn error(
        trace_id: &str,
        seq: u64,
        component: &str,
        message: &str,
        recoverable: bool,
    ) -> Self {
        Self::new(
            trace_id,
            seq,
            AuditEventType::Error {
                component: component.to_string(),
                message: message.to_string(),
                recoverable,
            },
        )
    }
}

/// Audit collector that persists to NATS JetStream.
pub struct AuditCollector {
    /// NATS client (optional for in-memory mode).
    nats: Option<Arc<NatsClient>>,
    /// In-memory cache for quick queries.
    cache: tokio::sync::RwLock<Vec<AuditEvent>>,
    /// Maximum events to cache.
    max_cache: usize,
}

impl AuditCollector {
    /// Create a new AuditCollector and ensure the stream exists.
    pub async fn new(nats: Arc<NatsClient>) -> Result<Self, BusError> {
        // Ensure the audit stream exists
        let stream_config =
            StreamConfig::new(AUDIT_STREAM).subjects(vec![format!("{}.>", AUDIT_SUBJECT)]);

        nats.ensure_stream(stream_config).await?;

        Ok(Self {
            nats: Some(nats),
            cache: tokio::sync::RwLock::new(Vec::with_capacity(1000)),
            max_cache: 10000,
        })
    }

    /// Create without NATS (in-memory only, for testing or standalone mode).
    pub fn in_memory() -> Self {
        Self {
            nats: None,
            cache: tokio::sync::RwLock::new(Vec::with_capacity(1000)),
            max_cache: 10000,
        }
    }

    /// Check if NATS persistence is enabled.
    pub fn has_persistence(&self) -> bool {
        self.nats.is_some()
    }

    /// Emit an audit event.
    /// If NATS is connected, publishes to JetStream. Always caches locally.
    pub async fn emit(&self, event: AuditEvent) -> Result<(), BusError> {
        // Publish to JetStream for persistence (if connected)
        if let Some(ref nats) = self.nats {
            let subject = format!("{}.{}", AUDIT_SUBJECT, event.trace_id);
            nats.publish_jetstream(&subject, &event).await?;
        }

        // Always cache locally
        let mut cache = self.cache.write().await;
        if cache.len() >= self.max_cache {
            cache.remove(0);
        }
        cache.push(event);

        Ok(())
    }

    /// Get all events for a trace from cache.
    pub async fn for_trace(&self, trace_id: &str) -> Vec<AuditEvent> {
        let cache = self.cache.read().await;
        cache
            .iter()
            .filter(|e| e.trace_id == trace_id)
            .cloned()
            .collect()
    }

    /// Get all events for a trace from JetStream (full history).
    /// Returns error if NATS is not connected.
    pub async fn for_trace_persistent(&self, trace_id: &str) -> Result<Vec<AuditEvent>, BusError> {
        let nats = self.nats.as_ref().ok_or_else(|| {
            BusError::ConnectionFailed("NATS not connected (in-memory mode)".to_string())
        })?;

        let consumer = nats
            .create_consumer(
                AUDIT_STREAM,
                &format!("audit-query-{}", trace_id),
                Some(&format!("{}.{}", AUDIT_SUBJECT, trace_id)),
            )
            .await?;

        let mut events = Vec::new();
        let mut messages = consumer
            .messages()
            .await
            .map_err(|e| BusError::SubscribeFailed(e.to_string()))?;

        use futures::StreamExt;
        while let Ok(Some(msg)) =
            tokio::time::timeout(Duration::from_millis(100), messages.next()).await
        {
            if let Ok(msg) = msg {
                if let Ok(event) = serde_json::from_slice::<AuditEvent>(&msg.payload) {
                    events.push(event);
                }
                let _ = msg.ack().await;
            }
        }

        events.sort_by_key(|e| e.sequence);
        Ok(events)
    }

    /// Get recent events from cache.
    pub async fn recent(&self, count: usize) -> Vec<AuditEvent> {
        let cache = self.cache.read().await;
        cache.iter().rev().take(count).cloned().collect()
    }

    /// Generate an audit report for a trace.
    pub async fn report(&self, trace_id: &str) -> AuditReport {
        let events = self.for_trace(trace_id).await;

        let mut report = AuditReport {
            trace_id: trace_id.to_string(),
            total_events: events.len(),
            stages: Vec::new(),
            model_selections: Vec::new(),
            rollbacks: Vec::new(),
            errors: Vec::new(),
            duration_ms: None,
            success: None,
        };

        for event in events {
            match &event.event {
                AuditEventType::StageCompleted {
                    stage,
                    attempt,
                    success,
                    ..
                } => {
                    report.stages.push(format!(
                        "{} attempt {} → {}",
                        stage,
                        attempt,
                        if *success { "OK" } else { "FAIL" }
                    ));
                }
                AuditEventType::ModelSelected { tier, reason, .. } => {
                    report
                        .model_selections
                        .push(format!("{}: {}", tier, reason));
                }
                AuditEventType::RollbackDecision {
                    decision, reason, ..
                } => {
                    report.rollbacks.push(format!("{}: {}", decision, reason));
                }
                AuditEventType::Error {
                    component, message, ..
                } => {
                    report.errors.push(format!("[{}] {}", component, message));
                }
                AuditEventType::PipelineCompleted {
                    success,
                    duration_ms,
                    ..
                } => {
                    report.success = Some(*success);
                    report.duration_ms = Some(*duration_ms);
                }
                _ => {}
            }
        }

        report
    }
}

/// Summary report for an audit trail.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub trace_id: String,
    pub total_events: usize,
    pub stages: Vec<String>,
    pub model_selections: Vec<String>,
    pub rollbacks: Vec<String>,
    pub errors: Vec<String>,
    pub duration_ms: Option<u64>,
    pub success: Option<bool>,
}

impl std::fmt::Display for AuditReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "=== Audit Report: {} ===", self.trace_id)?;
        writeln!(f, "Events: {}", self.total_events)?;

        if let Some(success) = self.success {
            writeln!(f, "Result: {}", if success { "SUCCESS" } else { "FAILED" })?;
        }
        if let Some(duration) = self.duration_ms {
            writeln!(f, "Duration: {}ms", duration)?;
        }

        if !self.stages.is_empty() {
            writeln!(f, "\nStages:")?;
            for s in &self.stages {
                writeln!(f, "  - {}", s)?;
            }
        }

        if !self.model_selections.is_empty() {
            writeln!(f, "\nModel Selections:")?;
            for s in &self.model_selections {
                writeln!(f, "  - {}", s)?;
            }
        }

        if !self.rollbacks.is_empty() {
            writeln!(f, "\nRollbacks:")?;
            for r in &self.rollbacks {
                writeln!(f, "  - {}", r)?;
            }
        }

        if !self.errors.is_empty() {
            writeln!(f, "\nErrors:")?;
            for e in &self.errors {
                writeln!(f, "  - {}", e)?;
            }
        }

        Ok(())
    }
}

/// Sequence counter for a trace.
pub struct TraceSequencer {
    trace_id: String,
    sequence: std::sync::atomic::AtomicU64,
}

impl TraceSequencer {
    /// Create a new sequencer for a trace.
    pub fn new(trace_id: impl Into<String>) -> Self {
        Self {
            trace_id: trace_id.into(),
            sequence: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Generate a new trace ID.
    pub fn new_trace() -> Self {
        Self::new(uuid::Uuid::new_v4().to_string())
    }

    /// Get the trace ID.
    pub fn trace_id(&self) -> &str {
        &self.trace_id
    }

    /// Get the next sequence number.
    pub fn next(&self) -> u64 {
        self.sequence
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_event_creation() {
        let event = AuditEvent::pipeline_started("trace-123", 0, "Test task");
        assert_eq!(event.trace_id, "trace-123");
        assert_eq!(event.sequence, 0);

        match event.event {
            AuditEventType::PipelineStarted { task_preview, .. } => {
                assert_eq!(task_preview, "Test task");
            }
            _ => panic!("Wrong event type"),
        }
    }

    #[test]
    fn test_trace_sequencer() {
        let seq = TraceSequencer::new("trace-123");
        assert_eq!(seq.next(), 0);
        assert_eq!(seq.next(), 1);
        assert_eq!(seq.next(), 2);
    }

    #[tokio::test]
    async fn test_audit_report() {
        let collector = AuditCollector::in_memory();

        collector
            .emit(AuditEvent::pipeline_started("t1", 0, "task"))
            .await
            .unwrap();
        collector
            .emit(AuditEvent::stage_started("t1", 1, "Types", 1))
            .await
            .unwrap();
        collector
            .emit(AuditEvent::stage_completed(
                "t1", 2, "Types", 1, true, "code",
            ))
            .await
            .unwrap();
        collector
            .emit(AuditEvent::pipeline_completed("t1", 3, true, 1000, 1))
            .await
            .unwrap();

        let report = collector.report("t1").await;
        assert_eq!(report.total_events, 4);
        assert_eq!(report.success, Some(true));
        assert_eq!(report.duration_ms, Some(1000));
    }
}
