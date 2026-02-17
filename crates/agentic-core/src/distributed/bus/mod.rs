//! NATS Bus Coordinator - Inter-supervisor message coordination.
//!
//! Provides core responsibilities:
//! - **Coordinator**: Central NATS JetStream management
//! - **StateStore**: KV store for sharing validated artifacts
//! - **Linter**: Fast pre-validation (2-3ms vs 25s compile)
//! - **Rollback**: Track and restore best attempts
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::bus::BusCoordinator;
//!
//! let coordinator = BusCoordinator::builder()
//!     .nats_url("nats://localhost:4222")
//!     .build()
//!     .await?;
//!
//! coordinator.start().await?;
//! ```

mod arbiter;
mod audit;
mod coordinator;
mod deduplicator;
mod error_fingerprint;
mod linter;
mod log_collector;
mod nats_client;
mod pipeline_tracer;
mod rollback;
mod sequencer;
mod state_store;
mod types;

pub use arbiter::{Arbiter, ArbiterConfig, ArbiterStrategy, Proposal, ProposalRequest};
pub use audit::{
    AuditCollector, AuditEvent, AuditEventType, AuditReport, TraceSequencer, AUDIT_STREAM,
    AUDIT_SUBJECT,
};
pub use coordinator::{BusCoordinator, BusCoordinatorBuilder, CoordinatorStats};
pub use deduplicator::{DedupeStrategy, Deduplicator, DeduplicatorConfig};
pub use error_fingerprint::{
    AnalysisResult, ErrorCategory, ErrorFingerprint, ErrorFingerprinter, ModelTier,
};
pub use linter::{BusLinter, LintError, LintResult, LintSeverity};
pub use log_collector::{LogCollector, LogCollectorConfig, LogEntry, LogLevel, LogQuery};
pub use nats_client::{
    deserialize_message, NatsClient, NatsConfig, StreamConfig, StreamRetention, StreamStorage,
};
pub use pipeline_tracer::{
    AttemptRecord, CodeSnapshot, DecisionRecord, DocReference, EnrichedError, ErrorAnalysis,
    ErrorLocation, ErrorSeverity, GenerationRecord, ModelConfig, ModelInfo, PipelineTrace,
    PipelineTracer, TokenUsage, TraceConfig, TraceMetrics, TraceResult, TraceStatus,
    ValidationRecord, ValidatorStageRecord, TRACE_SCHEMA_VERSION,
};
pub use rollback::{Attempt, AttemptScore, RollbackDecision, RollbackManager, RollbackStats};
pub use sequencer::{Sequencer, SequencerConfig};
pub use state_store::{extract_imports, extract_types, FunctionSig, StateStore, TypeDef};
pub use types::{BusError, BusResult, Task, TaskPriority, TimeRange};
