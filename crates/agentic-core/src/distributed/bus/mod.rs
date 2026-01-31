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
//! ```rust,no_run
//! use agentic_core::distributed::bus::BusCoordinator;
//!
//! let coordinator = BusCoordinator::builder()
//!     .nats_url("nats://localhost:4222")
//!     .build()
//!     .await?;
//!
//! coordinator.start().await?;
//! ```

mod coordinator;
mod sequencer;
mod arbiter;
mod deduplicator;
mod log_collector;
mod types;
mod nats_client;
mod state_store;
mod linter;
mod rollback;
mod error_fingerprint;
mod audit;
mod pipeline_tracer;

pub use coordinator::{BusCoordinator, BusCoordinatorBuilder, CoordinatorStats};
pub use sequencer::{Sequencer, SequencerConfig};
pub use arbiter::{Arbiter, ArbiterConfig, ArbiterStrategy, Proposal, ProposalRequest};
pub use deduplicator::{Deduplicator, DeduplicatorConfig, DedupeStrategy};
pub use log_collector::{LogCollector, LogCollectorConfig, LogEntry, LogLevel, LogQuery};
pub use types::{Task, TaskPriority, TimeRange, BusError, BusResult};
pub use nats_client::{NatsClient, NatsConfig, StreamConfig, StreamStorage, StreamRetention, deserialize_message};
pub use state_store::{StateStore, TypeDef, FunctionSig, extract_types, extract_imports};
pub use linter::{BusLinter, LintResult, LintError, LintSeverity};
pub use rollback::{RollbackManager, RollbackDecision, AttemptScore, Attempt, RollbackStats};
pub use error_fingerprint::{ErrorFingerprinter, ErrorFingerprint, ErrorCategory, ModelTier, AnalysisResult};
pub use audit::{AuditCollector, AuditEvent, AuditEventType, AuditReport, TraceSequencer, AUDIT_STREAM, AUDIT_SUBJECT};
pub use pipeline_tracer::{
    PipelineTracer, PipelineTrace, TraceStatus, TraceConfig, TraceResult, TraceMetrics,
    AttemptRecord, GenerationRecord, ValidationRecord, ValidatorStageRecord,
    DecisionRecord, CodeSnapshot, EnrichedError, ErrorSeverity, ErrorLocation,
    ErrorAnalysis, DocReference, TokenUsage, ModelConfig, ModelInfo,
    TRACE_SCHEMA_VERSION,
};
