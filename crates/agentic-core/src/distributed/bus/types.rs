//! Common types for the bus coordinator.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Time range for queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Option<chrono::DateTime<chrono::Utc>>,
    pub end: Option<chrono::DateTime<chrono::Utc>>,
}

impl TimeRange {
    /// Last N hours.
    pub fn last_hours(hours: u64) -> Self {
        let now = chrono::Utc::now();
        Self {
            start: Some(now - chrono::Duration::hours(hours as i64)),
            end: Some(now),
        }
    }

    /// Last N minutes.
    pub fn last_minutes(minutes: u64) -> Self {
        let now = chrono::Utc::now();
        Self {
            start: Some(now - chrono::Duration::minutes(minutes as i64)),
            end: Some(now),
        }
    }

    /// Last N days.
    pub fn last_days(days: u64) -> Self {
        let now = chrono::Utc::now();
        Self {
            start: Some(now - chrono::Duration::days(days as i64)),
            end: Some(now),
        }
    }
}

/// Task for the sequencer queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    /// Unique task ID
    pub id: String,
    /// Priority level
    pub priority: TaskPriority,
    /// Target supervisor
    pub supervisor: String,
    /// Task payload (JSON)
    pub payload: serde_json::Value,
    /// Dependencies (task IDs that must complete first)
    pub depends_on: Vec<String>,
    /// Creation timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Trace ID for distributed tracing
    pub trace_id: Option<String>,
}

impl Task {
    /// Create a new task.
    pub fn new(
        id: impl Into<String>,
        supervisor: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            priority: TaskPriority::Normal,
            supervisor: supervisor.into(),
            payload,
            depends_on: vec![],
            created_at: chrono::Utc::now(),
            trace_id: None,
        }
    }

    /// Set priority.
    pub fn with_priority(mut self, priority: TaskPriority) -> Self {
        self.priority = priority;
        self
    }

    /// Set dependencies.
    pub fn with_depends_on(mut self, deps: Vec<String>) -> Self {
        self.depends_on = deps;
        self
    }

    /// Set trace ID.
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }
}

/// Task priority levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

impl Default for TaskPriority {
    fn default() -> Self {
        Self::Normal
    }
}

impl TaskPriority {
    /// Get NATS subject suffix for priority routing.
    pub fn subject_suffix(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }
}

/// Result of a bus operation.
#[derive(Debug, Clone)]
pub enum BusResult<T> {
    Ok(T),
    Timeout,
    Duplicate,
    Error(String),
}

impl<T> BusResult<T> {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok(_))
    }

    pub fn unwrap(self) -> T {
        match self {
            Self::Ok(t) => t,
            Self::Timeout => panic!("called unwrap on Timeout"),
            Self::Duplicate => panic!("called unwrap on Duplicate"),
            Self::Error(e) => panic!("called unwrap on Error: {}", e),
        }
    }
}

/// Error type for bus operations.
#[derive(Debug, Clone)]
pub enum BusError {
    ConnectionFailed(String),
    StreamNotFound(String),
    PublishFailed(String),
    SubscribeFailed(String),
    Timeout,
    SerializationFailed(String),
    DeserializationFailed(String),
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionFailed(e) => write!(f, "Connection failed: {}", e),
            Self::StreamNotFound(s) => write!(f, "Stream not found: {}", s),
            Self::PublishFailed(e) => write!(f, "Publish failed: {}", e),
            Self::SubscribeFailed(e) => write!(f, "Subscribe failed: {}", e),
            Self::Timeout => write!(f, "Operation timed out"),
            Self::SerializationFailed(e) => write!(f, "Serialization failed: {}", e),
            Self::DeserializationFailed(e) => write!(f, "Deserialization failed: {}", e),
        }
    }
}

impl std::error::Error for BusError {}
