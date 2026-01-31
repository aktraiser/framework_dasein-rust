//! Sequencer - Task ordering and prioritization via NATS JetStream.
//!
//! The Sequencer provides:
//! - Priority-based ordering (Critical > High > Normal > Low)
//! - Dependency tracking (task B waits for task A)
//! - Rate limiting
//! - Work queue semantics (each task delivered to one consumer)

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use super::types::{BusError, Task, TaskPriority};

/// Sequencer configuration.
#[derive(Debug, Clone)]
pub struct SequencerConfig {
    /// Maximum pending tasks in the queue
    pub max_pending: usize,
    /// Rate limit (tasks per second)
    pub rate_limit_per_sec: Option<u32>,
    /// Stream name in JetStream
    pub stream_name: String,
    /// Subject prefix for tasks
    pub subject_prefix: String,
}

impl Default for SequencerConfig {
    fn default() -> Self {
        Self {
            max_pending: 1000,
            rate_limit_per_sec: None,
            stream_name: "TASKS".to_string(),
            subject_prefix: "bus.tasks".to_string(),
        }
    }
}

/// Sequencer for task ordering and prioritization.
pub struct Sequencer {
    config: SequencerConfig,
    // In a real implementation, this would be async_nats::Client
    // For now we use a mock in-memory queue
    pending_tasks: Arc<RwLock<Vec<Task>>>,
    completed_tasks: Arc<RwLock<HashSet<String>>>,
    blocked_tasks: Arc<RwLock<HashMap<String, Task>>>,
}

impl Sequencer {
    /// Create a new Sequencer with default config.
    pub fn new() -> Self {
        Self::with_config(SequencerConfig::default())
    }

    /// Create with custom config.
    pub fn with_config(config: SequencerConfig) -> Self {
        Self {
            config,
            pending_tasks: Arc::new(RwLock::new(Vec::new())),
            completed_tasks: Arc::new(RwLock::new(HashSet::new())),
            blocked_tasks: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Publish a task to the queue.
    pub async fn publish(&self, task: Task) -> Result<(), BusError> {
        // Check if we're at capacity
        let pending_count = self.pending_tasks.read().await.len();
        if pending_count >= self.config.max_pending {
            return Err(BusError::PublishFailed(format!(
                "Queue full: {} pending tasks",
                pending_count
            )));
        }

        // Check if dependencies are satisfied
        if !task.depends_on.is_empty() {
            let completed = self.completed_tasks.read().await;
            let unsatisfied: Vec<_> = task
                .depends_on
                .iter()
                .filter(|dep| !completed.contains(*dep))
                .collect();

            if !unsatisfied.is_empty() {
                // Task is blocked, store it separately
                let mut blocked = self.blocked_tasks.write().await;
                blocked.insert(task.id.clone(), task);
                return Ok(());
            }
        }

        // Add to pending queue, sorted by priority
        let mut pending = self.pending_tasks.write().await;
        pending.push(task);
        pending.sort_by(|a, b| b.priority.cmp(&a.priority)); // Higher priority first

        Ok(())
    }

    /// Get the next task from the queue.
    pub async fn next(&self) -> Option<Task> {
        let mut pending = self.pending_tasks.write().await;
        if pending.is_empty() {
            return None;
        }
        Some(pending.remove(0))
    }

    /// Get the next task with timeout.
    pub async fn next_with_timeout(&self, timeout: Duration) -> Option<Task> {
        let start = std::time::Instant::now();

        loop {
            if let Some(task) = self.next().await {
                return Some(task);
            }

            if start.elapsed() >= timeout {
                return None;
            }

            // Wait a bit before checking again
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Mark a task as completed.
    pub async fn complete(&self, task_id: &str) -> Result<(), BusError> {
        let mut completed = self.completed_tasks.write().await;
        completed.insert(task_id.to_string());

        // Check if any blocked tasks can now be unblocked
        let mut blocked = self.blocked_tasks.write().await;
        let mut to_unblock = Vec::new();

        for (id, task) in blocked.iter() {
            let all_deps_satisfied = task.depends_on.iter().all(|dep| completed.contains(dep));
            if all_deps_satisfied {
                to_unblock.push(id.clone());
            }
        }

        // Move unblocked tasks to pending
        let mut pending = self.pending_tasks.write().await;
        for id in to_unblock {
            if let Some(task) = blocked.remove(&id) {
                pending.push(task);
            }
        }
        pending.sort_by(|a, b| b.priority.cmp(&a.priority));

        Ok(())
    }

    /// Get queue statistics.
    pub async fn stats(&self) -> SequencerStats {
        let pending = self.pending_tasks.read().await;
        let completed = self.completed_tasks.read().await;
        let blocked = self.blocked_tasks.read().await;

        let mut by_priority = HashMap::new();
        for task in pending.iter() {
            *by_priority.entry(task.priority).or_insert(0) += 1;
        }

        SequencerStats {
            pending_count: pending.len(),
            completed_count: completed.len(),
            blocked_count: blocked.len(),
            by_priority,
        }
    }

    /// Get subject for a task based on priority.
    pub fn subject_for(&self, task: &Task) -> String {
        format!(
            "{}.{}.{}",
            self.config.subject_prefix,
            task.priority.subject_suffix(),
            task.supervisor
        )
    }
}

impl Default for Sequencer {
    fn default() -> Self {
        Self::new()
    }
}

/// Sequencer statistics.
#[derive(Debug, Clone)]
pub struct SequencerStats {
    pub pending_count: usize,
    pub completed_count: usize,
    pub blocked_count: usize,
    pub by_priority: HashMap<TaskPriority, usize>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_priority_ordering() {
        let seq = Sequencer::new();

        // Publish tasks in random order
        seq.publish(Task::new("t1", "sup", json!({})).with_priority(TaskPriority::Low))
            .await
            .unwrap();
        seq.publish(Task::new("t2", "sup", json!({})).with_priority(TaskPriority::Critical))
            .await
            .unwrap();
        seq.publish(Task::new("t3", "sup", json!({})).with_priority(TaskPriority::Normal))
            .await
            .unwrap();

        // Should come out in priority order
        assert_eq!(seq.next().await.unwrap().id, "t2"); // Critical
        assert_eq!(seq.next().await.unwrap().id, "t3"); // Normal
        assert_eq!(seq.next().await.unwrap().id, "t1"); // Low
    }

    #[tokio::test]
    async fn test_dependencies() {
        let seq = Sequencer::new();

        // Task B depends on Task A
        seq.publish(Task::new("b", "sup", json!({})).with_depends_on(vec!["a".to_string()]))
            .await
            .unwrap();

        // B should be blocked
        assert!(seq.next().await.is_none());

        // Publish A
        seq.publish(Task::new("a", "sup", json!({}))).await.unwrap();

        // A should be available
        let task_a = seq.next().await.unwrap();
        assert_eq!(task_a.id, "a");

        // Complete A
        seq.complete("a").await.unwrap();

        // Now B should be available
        let task_b = seq.next().await.unwrap();
        assert_eq!(task_b.id, "b");
    }
}
