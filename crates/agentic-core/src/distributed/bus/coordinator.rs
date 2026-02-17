//! Bus Coordinator - Real NATS-based coordination for distributed agents.
//!
//! The BusCoordinator provides a unified interface to:
//! - Sequencer: Task ordering and prioritization
//! - Arbiter: Best-of-N proposal selection
//! - Deduplicator: Prevent duplicate processing
//! - LogCollector: Centralized logging
//!
//! All backed by real NATS JetStream for persistence and distribution.
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use dasein_agentic_core::distributed::bus::BusCoordinator;
//!
//! let coordinator = BusCoordinator::connect("nats://localhost:4222").await?;
//! coordinator.start().await?;
//!
//! // Publish a task
//! coordinator.publish_task(task).await?;
//! ```

use std::sync::Arc;
use tracing::{debug, info};

use super::arbiter::{Arbiter, ArbiterConfig, Proposal, ScoredProposal};
use super::deduplicator::{Deduplicator, DeduplicatorConfig};
use super::log_collector::{LogCollector, LogCollectorConfig, LogEntry, LogQuery};
use super::nats_client::{NatsClient, NatsConfig, StreamConfig};
use super::sequencer::{Sequencer, SequencerConfig};
use super::types::{BusError, Task};

/// Stream names for JetStream.
pub const STREAM_TASKS: &str = "AGENTIC_TASKS";
pub const STREAM_PROPOSALS: &str = "AGENTIC_PROPOSALS";
pub const STREAM_LOGS: &str = "AGENTIC_LOGS";
pub const STREAM_AUDIT: &str = "AGENTIC_AUDIT";

/// Subject prefixes.
pub const SUBJECT_TASKS: &str = "agentic.tasks";
pub const SUBJECT_PROPOSALS: &str = "agentic.proposals";
pub const SUBJECT_LOGS: &str = "agentic.logs";
pub const SUBJECT_AUDIT: &str = "agentic.audit";

/// Bus Coordinator configuration.
#[derive(Debug, Clone)]
pub struct BusCoordinatorConfig {
    /// NATS configuration
    pub nats: NatsConfig,
    /// Sequencer configuration
    pub sequencer: SequencerConfig,
    /// Arbiter configuration
    pub arbiter: ArbiterConfig,
    /// Deduplicator configuration
    pub deduplicator: DeduplicatorConfig,
    /// Log collector configuration
    pub log_collector: LogCollectorConfig,
    /// Auto-create JetStream streams
    pub auto_create_streams: bool,
}

impl Default for BusCoordinatorConfig {
    fn default() -> Self {
        Self {
            nats: NatsConfig::default(),
            sequencer: SequencerConfig::default(),
            arbiter: ArbiterConfig::default(),
            deduplicator: DeduplicatorConfig::default(),
            log_collector: LogCollectorConfig::default(),
            auto_create_streams: true,
        }
    }
}

/// The Bus Coordinator - real NATS-based coordination.
pub struct BusCoordinator {
    config: BusCoordinatorConfig,
    nats: Arc<NatsClient>,
    sequencer: Arc<Sequencer>,
    arbiter: Arc<Arbiter>,
    deduplicator: Arc<Deduplicator>,
    log_collector: Arc<LogCollector>,
    running: std::sync::atomic::AtomicBool,
}

impl BusCoordinator {
    /// Connect to NATS and create coordinator.
    pub async fn connect(url: &str) -> Result<Self, BusError> {
        let config = BusCoordinatorConfig {
            nats: NatsConfig::new(url),
            ..Default::default()
        };
        Self::with_config(config).await
    }

    /// Create with custom config.
    pub async fn with_config(config: BusCoordinatorConfig) -> Result<Self, BusError> {
        info!("Connecting Bus Coordinator to NATS...");

        let nats = NatsClient::connect(config.nats.clone()).await?;
        let nats = Arc::new(nats);

        Ok(Self {
            sequencer: Arc::new(Sequencer::with_config(config.sequencer.clone())),
            arbiter: Arc::new(Arbiter::with_config(config.arbiter.clone())),
            deduplicator: Arc::new(Deduplicator::with_config(config.deduplicator.clone())),
            log_collector: Arc::new(LogCollector::with_config(config.log_collector.clone())),
            nats,
            config,
            running: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Create a builder.
    pub fn builder() -> BusCoordinatorBuilder {
        BusCoordinatorBuilder::new()
    }

    /// Start the coordinator and create JetStream streams.
    pub async fn start(&self) -> Result<(), BusError> {
        info!("Starting Bus Coordinator...");

        if self.config.auto_create_streams {
            self.create_streams().await?;
        }

        self.running
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // Log startup
        self.log(LogEntry::info("bus-coordinator", "Bus Coordinator started"))
            .await?;

        info!("Bus Coordinator started successfully");
        Ok(())
    }

    /// Get reference to the NATS client for external use (e.g., StateStore).
    pub fn nats_client(&self) -> Arc<NatsClient> {
        self.nats.clone()
    }

    /// Create required JetStream streams.
    async fn create_streams(&self) -> Result<(), BusError> {
        info!("Creating JetStream streams...");

        // Tasks stream (work queue)
        let tasks_config = StreamConfig::new(STREAM_TASKS)
            .subjects(vec![format!("{}.>", SUBJECT_TASKS)])
            .work_queue();
        self.nats.ensure_stream(tasks_config).await?;
        info!("  Created stream: {}", STREAM_TASKS);

        // Proposals stream
        let proposals_config =
            StreamConfig::new(STREAM_PROPOSALS).subjects(vec![format!("{}.>", SUBJECT_PROPOSALS)]);
        self.nats.ensure_stream(proposals_config).await?;
        info!("  Created stream: {}", STREAM_PROPOSALS);

        // Logs stream
        let logs_config =
            StreamConfig::new(STREAM_LOGS).subjects(vec![format!("{}.>", SUBJECT_LOGS)]);
        self.nats.ensure_stream(logs_config).await?;
        info!("  Created stream: {}", STREAM_LOGS);

        // Audit stream
        let audit_config =
            StreamConfig::new(STREAM_AUDIT).subjects(vec![format!("{}.>", SUBJECT_AUDIT)]);
        self.nats.ensure_stream(audit_config).await?;
        info!("  Created stream: {}", STREAM_AUDIT);

        Ok(())
    }

    /// Stop the coordinator.
    pub async fn stop(&self) -> Result<(), BusError> {
        info!("Stopping Bus Coordinator...");
        self.running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        self.log(LogEntry::info("bus-coordinator", "Bus Coordinator stopped"))
            .await?;
        Ok(())
    }

    /// Check if running.
    pub fn is_running(&self) -> bool {
        self.running.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Check if connected to NATS.
    pub fn is_connected(&self) -> bool {
        self.nats.is_connected()
    }

    // === Task Operations (via NATS) ===

    /// Publish a task to NATS.
    pub async fn publish_task(&self, task: Task) -> Result<(), BusError> {
        // Check for duplicates first
        let task_json = serde_json::to_string(&task.payload)
            .map_err(|e| BusError::SerializationFailed(e.to_string()))?;

        if self.deduplicator.check_and_mark(&task_json).await {
            debug!("Task deduplicated: {}", task.id);
            return Ok(());
        }

        // Build subject based on priority
        let subject = format!(
            "{}.{}.{}",
            SUBJECT_TASKS,
            task.priority.subject_suffix(),
            task.supervisor
        );

        // Publish to NATS JetStream
        self.nats.publish_jetstream(&subject, &task).await?;

        // Also track in local sequencer for stats
        self.sequencer.publish(task.clone()).await?;

        info!("Published task {} to {}", task.id, subject);
        Ok(())
    }

    /// Subscribe to tasks for a supervisor.
    pub async fn subscribe_tasks(
        &self,
        supervisor_id: &str,
    ) -> Result<async_nats::Subscriber, BusError> {
        let subject = format!("{}.*.{}", SUBJECT_TASKS, supervisor_id);
        info!("Subscribing to tasks: {}", subject);
        self.nats.subscribe(&subject).await
    }

    /// Subscribe to tasks with queue group (for load balancing).
    pub async fn subscribe_tasks_queue(
        &self,
        supervisor_id: &str,
        queue_group: &str,
    ) -> Result<async_nats::Subscriber, BusError> {
        let subject = format!("{}.*.{}", SUBJECT_TASKS, supervisor_id);
        info!(
            "Subscribing to tasks with queue group: {} -> {}",
            subject, queue_group
        );
        self.nats.queue_subscribe(&subject, queue_group).await
    }

    /// Create a JetStream consumer for tasks.
    pub async fn create_task_consumer(
        &self,
        consumer_name: &str,
        supervisor_id: &str,
    ) -> Result<
        async_nats::jetstream::consumer::Consumer<async_nats::jetstream::consumer::pull::Config>,
        BusError,
    > {
        let filter = format!("{}.*.{}", SUBJECT_TASKS, supervisor_id);
        self.nats
            .create_consumer(STREAM_TASKS, consumer_name, Some(&filter))
            .await
    }

    /// Mark a task as completed.
    pub async fn complete_task(&self, task_id: &str) -> Result<(), BusError> {
        self.sequencer.complete(task_id).await?;

        // Publish completion event
        let event = serde_json::json!({
            "type": "task_completed",
            "task_id": task_id,
            "timestamp": chrono::Utc::now()
        });
        self.nats
            .publish(&format!("{}.completed", SUBJECT_AUDIT), &event)
            .await?;

        Ok(())
    }

    // === Proposal Operations (via NATS) ===

    /// Request proposals for a task.
    pub async fn request_proposals(
        &self,
        prompt: impl Into<String>,
        count: usize,
    ) -> Result<String, BusError> {
        let request_id = self.arbiter.request_proposals(prompt, count).await?;

        // Broadcast request via NATS
        let request = serde_json::json!({
            "request_id": &request_id,
            "count": count,
            "timestamp": chrono::Utc::now()
        });
        self.nats
            .publish(&format!("{}.request", SUBJECT_PROPOSALS), &request)
            .await?;

        Ok(request_id)
    }

    /// Submit a proposal via NATS.
    pub async fn submit_proposal(&self, proposal: Proposal) -> Result<(), BusError> {
        // Publish to NATS
        let subject = format!("{}.{}", SUBJECT_PROPOSALS, proposal.request_id);
        self.nats.publish(&subject, &proposal).await?;

        // Also track locally
        self.arbiter.submit_proposal(proposal).await
    }

    /// Subscribe to proposal requests.
    pub async fn subscribe_proposal_requests(&self) -> Result<async_nats::Subscriber, BusError> {
        self.nats
            .subscribe(&format!("{}.request", SUBJECT_PROPOSALS))
            .await
    }

    /// Subscribe to proposals for a request.
    pub async fn subscribe_proposals(
        &self,
        request_id: &str,
    ) -> Result<async_nats::Subscriber, BusError> {
        self.nats
            .subscribe(&format!("{}.{}", SUBJECT_PROPOSALS, request_id))
            .await
    }

    /// Select the best proposal.
    pub async fn select_best_proposal(&self, request_id: &str) -> Result<ScoredProposal, BusError> {
        self.arbiter.select_best(request_id).await
    }

    // === Deduplication ===

    /// Check if content is a duplicate.
    pub async fn is_duplicate(&self, content: &str) -> bool {
        self.deduplicator.is_duplicate(content).await
    }

    /// Check and mark atomically.
    pub async fn check_and_mark_duplicate(&self, content: &str) -> bool {
        self.deduplicator.check_and_mark(content).await
    }

    // === Logging (via NATS) ===

    /// Log an entry to NATS.
    pub async fn log(&self, entry: LogEntry) -> Result<(), BusError> {
        // Publish to NATS
        let subject = format!("{}.{}", SUBJECT_LOGS, entry.agent_id);
        self.nats.publish(&subject, &entry).await?;

        // Also store locally
        self.log_collector.log(entry).await
    }

    /// Log info.
    pub async fn log_info(
        &self,
        agent_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<(), BusError> {
        self.log(LogEntry::info(agent_id, message)).await
    }

    /// Log error.
    pub async fn log_error(
        &self,
        agent_id: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<(), BusError> {
        self.log(LogEntry::error(agent_id, message)).await
    }

    /// Subscribe to logs.
    pub async fn subscribe_logs(&self) -> Result<async_nats::Subscriber, BusError> {
        self.nats.subscribe(&format!("{}.>", SUBJECT_LOGS)).await
    }

    /// Subscribe to logs for a specific agent.
    pub async fn subscribe_agent_logs(
        &self,
        agent_id: &str,
    ) -> Result<async_nats::Subscriber, BusError> {
        self.nats
            .subscribe(&format!("{}.{}", SUBJECT_LOGS, agent_id))
            .await
    }

    /// Query local logs.
    pub async fn query_logs(&self, query: LogQuery) -> Vec<LogEntry> {
        self.log_collector.query(query).await
    }

    // === Audit Events ===

    /// Publish an audit event.
    pub async fn audit(&self, event_type: &str, data: serde_json::Value) -> Result<(), BusError> {
        let event = serde_json::json!({
            "type": event_type,
            "data": data,
            "timestamp": chrono::Utc::now()
        });
        self.nats
            .publish(&format!("{}.{}", SUBJECT_AUDIT, event_type), &event)
            .await
    }

    /// Subscribe to audit events.
    pub async fn subscribe_audit(&self) -> Result<async_nats::Subscriber, BusError> {
        self.nats.subscribe(&format!("{}.>", SUBJECT_AUDIT)).await
    }

    // === Component Access ===

    /// Get the NATS client.
    pub fn nats(&self) -> &Arc<NatsClient> {
        &self.nats
    }

    /// Get the sequencer.
    pub fn sequencer(&self) -> &Arc<Sequencer> {
        &self.sequencer
    }

    /// Get the arbiter.
    pub fn arbiter(&self) -> &Arc<Arbiter> {
        &self.arbiter
    }

    /// Get the deduplicator.
    pub fn deduplicator(&self) -> &Arc<Deduplicator> {
        &self.deduplicator
    }

    /// Get the log collector.
    pub fn log_collector(&self) -> &Arc<LogCollector> {
        &self.log_collector
    }

    /// Get sequencer stats.
    pub async fn stats(&self) -> CoordinatorStats {
        let sequencer_stats = self.sequencer.stats().await;
        let dedup_stats = self.deduplicator.stats().await;
        let log_stats = self.log_collector.stats().await;

        CoordinatorStats {
            connected: self.is_connected(),
            running: self.is_running(),
            tasks_pending: sequencer_stats.pending_count,
            tasks_completed: sequencer_stats.completed_count,
            tasks_blocked: sequencer_stats.blocked_count,
            duplicates_prevented: dedup_stats.total_duplicates_prevented,
            logs_collected: log_stats.total_entries,
        }
    }
}

/// Coordinator statistics.
#[derive(Debug, Clone)]
pub struct CoordinatorStats {
    pub connected: bool,
    pub running: bool,
    pub tasks_pending: usize,
    pub tasks_completed: usize,
    pub tasks_blocked: usize,
    pub duplicates_prevented: usize,
    pub logs_collected: usize,
}

/// Builder for BusCoordinator.
pub struct BusCoordinatorBuilder {
    config: BusCoordinatorConfig,
}

impl BusCoordinatorBuilder {
    pub fn new() -> Self {
        Self {
            config: BusCoordinatorConfig::default(),
        }
    }

    /// Set NATS URL.
    pub fn nats_url(mut self, url: impl Into<String>) -> Self {
        self.config.nats = NatsConfig::new(url);
        self
    }

    /// Set NATS config.
    pub fn nats_config(mut self, config: NatsConfig) -> Self {
        self.config.nats = config;
        self
    }

    /// Set sequencer config.
    pub fn sequencer_config(mut self, config: SequencerConfig) -> Self {
        self.config.sequencer = config;
        self
    }

    /// Set arbiter config.
    pub fn arbiter_config(mut self, config: ArbiterConfig) -> Self {
        self.config.arbiter = config;
        self
    }

    /// Set deduplicator config.
    pub fn dedup_config(mut self, config: DeduplicatorConfig) -> Self {
        self.config.deduplicator = config;
        self
    }

    /// Set log collector config.
    pub fn log_config(mut self, config: LogCollectorConfig) -> Self {
        self.config.log_collector = config;
        self
    }

    /// Disable auto-creation of streams.
    pub fn no_auto_streams(mut self) -> Self {
        self.config.auto_create_streams = false;
        self
    }

    /// Build and connect the coordinator.
    pub async fn build(self) -> Result<BusCoordinator, BusError> {
        BusCoordinator::with_config(self.config).await
    }

    /// Build, connect, and start the coordinator.
    pub async fn build_and_start(self) -> Result<BusCoordinator, BusError> {
        let coordinator = self.build().await?;
        coordinator.start().await?;
        Ok(coordinator)
    }
}

impl Default for BusCoordinatorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // These tests require a running NATS server
    // Run with: docker run -d --name nats -p 4222:4222 nats:latest -js

    #[tokio::test]
    #[ignore = "requires NATS server"]
    async fn test_coordinator_connect() {
        let coordinator = BusCoordinator::connect("nats://localhost:4222")
            .await
            .unwrap();

        assert!(coordinator.is_connected());
    }

    #[tokio::test]
    #[ignore = "requires NATS server"]
    async fn test_coordinator_start() {
        let coordinator = BusCoordinator::builder()
            .nats_url("nats://localhost:4222")
            .build_and_start()
            .await
            .unwrap();

        assert!(coordinator.is_running());
        assert!(coordinator.is_connected());

        coordinator.stop().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires NATS server"]
    async fn test_publish_task() {
        let coordinator = BusCoordinator::builder()
            .nats_url("nats://localhost:4222")
            .build_and_start()
            .await
            .unwrap();

        let task = Task::new("task-1", "sup-1", json!({"action": "test"}));
        coordinator.publish_task(task).await.unwrap();

        let stats = coordinator.stats().await;
        assert!(stats.tasks_pending > 0 || stats.tasks_completed > 0);

        coordinator.stop().await.unwrap();
    }

    #[tokio::test]
    #[ignore = "requires NATS server"]
    async fn test_logging() {
        let coordinator = BusCoordinator::builder()
            .nats_url("nats://localhost:4222")
            .build_and_start()
            .await
            .unwrap();

        coordinator
            .log_info("test-agent", "Test message")
            .await
            .unwrap();

        let stats = coordinator.stats().await;
        assert!(stats.logs_collected > 0);

        coordinator.stop().await.unwrap();
    }
}
