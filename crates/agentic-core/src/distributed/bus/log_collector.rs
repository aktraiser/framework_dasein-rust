//! Log Collector - Centralized structured logging via NATS.
//!
//! Collects logs from all agents in a distributed system:
//! - Structured log entries with trace context
//! - Query logs by agent, level, time range
//! - Configurable retention
//! - Real-time log streaming

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use super::types::{BusError, TimeRange};

/// Log collector configuration.
#[derive(Debug, Clone)]
pub struct LogCollectorConfig {
    /// Retention period for logs
    pub retention: Duration,
    /// Maximum logs to keep in memory
    pub max_entries: usize,
    /// Batch size for writing
    pub batch_size: usize,
    /// Stream name in JetStream
    pub stream_name: String,
}

impl Default for LogCollectorConfig {
    fn default() -> Self {
        Self {
            retention: Duration::from_secs(7 * 24 * 60 * 60), // 7 days
            max_entries: 100000,
            batch_size: 100,
            stream_name: "LOGS".to_string(),
        }
    }
}

/// Log level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

impl Default for LogLevel {
    fn default() -> Self {
        Self::Info
    }
}

/// A structured log entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Unique log ID
    pub id: String,
    /// Log level
    pub level: LogLevel,
    /// Agent ID that generated this log
    pub agent_id: String,
    /// Agent role (supervisor, executor, validator)
    pub agent_role: Option<String>,
    /// Trace ID for distributed tracing
    pub trace_id: Option<String>,
    /// Span ID
    pub span_id: Option<String>,
    /// Log message
    pub message: String,
    /// Additional structured data
    pub metadata: serde_json::Value,
    /// Timestamp
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl LogEntry {
    /// Create a new log entry.
    pub fn new(level: LogLevel, agent_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            level,
            agent_id: agent_id.into(),
            agent_role: None,
            trace_id: None,
            span_id: None,
            message: message.into(),
            metadata: serde_json::Value::Null,
            timestamp: chrono::Utc::now(),
        }
    }

    /// Set agent role.
    pub fn with_role(mut self, role: impl Into<String>) -> Self {
        self.agent_role = Some(role.into());
        self
    }

    /// Set trace ID.
    pub fn with_trace_id(mut self, trace_id: impl Into<String>) -> Self {
        self.trace_id = Some(trace_id.into());
        self
    }

    /// Set span ID.
    pub fn with_span_id(mut self, span_id: impl Into<String>) -> Self {
        self.span_id = Some(span_id.into());
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Convenience constructors.
    pub fn info(agent_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Info, agent_id, message)
    }

    pub fn warn(agent_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Warn, agent_id, message)
    }

    pub fn error(agent_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Error, agent_id, message)
    }

    pub fn debug(agent_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self::new(LogLevel::Debug, agent_id, message)
    }
}

/// Query for logs.
#[derive(Debug, Clone, Default)]
pub struct LogQuery {
    /// Filter by agent ID
    pub agent_id: Option<String>,
    /// Filter by agent role
    pub agent_role: Option<String>,
    /// Filter by trace ID
    pub trace_id: Option<String>,
    /// Filter by minimum level
    pub min_level: Option<LogLevel>,
    /// Filter by time range
    pub time_range: Option<TimeRange>,
    /// Search in message
    pub message_contains: Option<String>,
    /// Maximum results
    pub limit: usize,
    /// Skip N results
    pub offset: usize,
}

impl LogQuery {
    pub fn new() -> Self {
        Self {
            limit: 100,
            ..Default::default()
        }
    }

    pub fn for_trace(trace_id: impl Into<String>) -> Self {
        Self {
            trace_id: Some(trace_id.into()),
            limit: 1000,
            ..Default::default()
        }
    }

    pub fn for_agent(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: Some(agent_id.into()),
            limit: 100,
            ..Default::default()
        }
    }

    pub fn errors_only(mut self) -> Self {
        self.min_level = Some(LogLevel::Error);
        self
    }
}

/// Log collector for centralized logging.
pub struct LogCollector {
    config: LogCollectorConfig,
    /// In-memory log storage (ring buffer)
    logs: Arc<RwLock<VecDeque<LogEntry>>>,
}

impl LogCollector {
    /// Create a new LogCollector with default config.
    pub fn new() -> Self {
        Self::with_config(LogCollectorConfig::default())
    }

    /// Create with custom config.
    pub fn with_config(config: LogCollectorConfig) -> Self {
        Self {
            config,
            logs: Arc::new(RwLock::new(VecDeque::with_capacity(10000))),
        }
    }

    /// Log an entry.
    pub async fn log(&self, entry: LogEntry) -> Result<(), BusError> {
        let mut logs = self.logs.write().await;

        // Maintain max size
        while logs.len() >= self.config.max_entries {
            logs.pop_front();
        }

        logs.push_back(entry);

        // In a real implementation, we would also publish to NATS:
        // nats.publish(format!("bus.logs.{}", entry.agent_id), &entry).await?;

        Ok(())
    }

    /// Convenience method for info log.
    pub async fn info(
        &self,
        agent_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<(), BusError> {
        self.log(LogEntry::info(agent_id, message)).await
    }

    /// Convenience method for error log.
    pub async fn error(
        &self,
        agent_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<(), BusError> {
        self.log(LogEntry::error(agent_id, message)).await
    }

    /// Query logs.
    pub async fn query(&self, query: LogQuery) -> Vec<LogEntry> {
        let logs = self.logs.read().await;

        let filtered: Vec<LogEntry> = logs
            .iter()
            .filter(|entry| {
                // Agent ID filter
                if let Some(ref agent_id) = query.agent_id {
                    if &entry.agent_id != agent_id {
                        return false;
                    }
                }

                // Agent role filter
                if let Some(ref role) = query.agent_role {
                    if entry.agent_role.as_ref() != Some(role) {
                        return false;
                    }
                }

                // Trace ID filter
                if let Some(ref trace_id) = query.trace_id {
                    if entry.trace_id.as_ref() != Some(trace_id) {
                        return false;
                    }
                }

                // Level filter
                if let Some(min_level) = query.min_level {
                    if entry.level < min_level {
                        return false;
                    }
                }

                // Time range filter
                if let Some(ref range) = query.time_range {
                    if let Some(start) = range.start {
                        if entry.timestamp < start {
                            return false;
                        }
                    }
                    if let Some(end) = range.end {
                        if entry.timestamp > end {
                            return false;
                        }
                    }
                }

                // Message contains filter
                if let Some(ref contains) = query.message_contains {
                    if !entry.message.contains(contains) {
                        return false;
                    }
                }

                true
            })
            .skip(query.offset)
            .take(query.limit)
            .cloned()
            .collect();

        filtered
    }

    /// Get recent logs.
    pub async fn recent(&self, count: usize) -> Vec<LogEntry> {
        let logs = self.logs.read().await;
        logs.iter().rev().take(count).cloned().collect()
    }

    /// Get logs for a trace.
    pub async fn for_trace(&self, trace_id: &str) -> Vec<LogEntry> {
        self.query(LogQuery::for_trace(trace_id)).await
    }

    /// Get error logs.
    pub async fn errors(&self, limit: usize) -> Vec<LogEntry> {
        self.query(LogQuery::new().errors_only()).await
    }

    /// Get statistics.
    pub async fn stats(&self) -> LogCollectorStats {
        let logs = self.logs.read().await;

        let mut by_level = std::collections::HashMap::new();
        let mut by_agent = std::collections::HashMap::new();

        for entry in logs.iter() {
            *by_level.entry(entry.level).or_insert(0) += 1;
            *by_agent.entry(entry.agent_id.clone()).or_insert(0) += 1;
        }

        LogCollectorStats {
            total_entries: logs.len(),
            by_level,
            by_agent,
        }
    }

    /// Clear all logs.
    pub async fn clear(&self) {
        let mut logs = self.logs.write().await;
        logs.clear();
    }
}

impl Default for LogCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Log collector statistics.
#[derive(Debug, Clone)]
pub struct LogCollectorStats {
    pub total_entries: usize,
    pub by_level: std::collections::HashMap<LogLevel, usize>,
    pub by_agent: std::collections::HashMap<String, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_logging() {
        let collector = LogCollector::new();

        collector.info("agent-1", "Test message").await.unwrap();
        collector.error("agent-1", "Error message").await.unwrap();

        let logs = collector.recent(10).await;
        assert_eq!(logs.len(), 2);
    }

    #[tokio::test]
    async fn test_query_by_agent() {
        let collector = LogCollector::new();

        collector.info("agent-1", "From agent 1").await.unwrap();
        collector.info("agent-2", "From agent 2").await.unwrap();
        collector.info("agent-1", "Another from agent 1").await.unwrap();

        let logs = collector.query(LogQuery::for_agent("agent-1")).await;
        assert_eq!(logs.len(), 2);
    }

    #[tokio::test]
    async fn test_query_by_level() {
        let collector = LogCollector::new();

        collector.info("agent-1", "Info").await.unwrap();
        collector.error("agent-1", "Error").await.unwrap();
        collector
            .log(LogEntry::debug("agent-1", "Debug"))
            .await
            .unwrap();

        let errors = collector.query(LogQuery::new().errors_only()).await;
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].level, LogLevel::Error);
    }

    #[tokio::test]
    async fn test_query_by_trace() {
        let collector = LogCollector::new();

        collector
            .log(LogEntry::info("agent-1", "Step 1").with_trace_id("trace-abc"))
            .await
            .unwrap();
        collector
            .log(LogEntry::info("agent-2", "Step 2").with_trace_id("trace-abc"))
            .await
            .unwrap();
        collector
            .log(LogEntry::info("agent-1", "Other trace").with_trace_id("trace-xyz"))
            .await
            .unwrap();

        let logs = collector.for_trace("trace-abc").await;
        assert_eq!(logs.len(), 2);
    }
}
