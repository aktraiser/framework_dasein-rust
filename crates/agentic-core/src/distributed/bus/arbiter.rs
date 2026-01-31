//! Arbiter - Best-of-N proposal selection.
//!
//! When multiple Executors can handle a task, the Arbiter:
//! 1. Broadcasts a request for proposals
//! 2. Collects N proposals (or waits for timeout)
//! 3. Scores and selects the best one
//!
//! Strategies:
//! - BestOfN: Wait for N proposals, select highest score
//! - FirstValid: Accept first proposal that passes validation
//! - Voting: Multiple scorers vote on proposals

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use super::types::BusError;

/// Arbiter configuration.
#[derive(Debug, Clone)]
pub struct ArbiterConfig {
    /// Default number of proposals to request
    pub default_proposals: usize,
    /// Timeout waiting for proposals (ms)
    pub proposal_timeout_ms: u64,
    /// Selection strategy
    pub strategy: ArbiterStrategy,
    /// Stream name in JetStream
    pub stream_name: String,
}

impl Default for ArbiterConfig {
    fn default() -> Self {
        Self {
            default_proposals: 3,
            proposal_timeout_ms: 5000,
            strategy: ArbiterStrategy::BestOfN { n: 3 },
            stream_name: "PROPOSALS".to_string(),
        }
    }
}

/// Strategy for selecting proposals.
#[derive(Debug, Clone)]
pub enum ArbiterStrategy {
    /// Wait for N proposals, select highest score
    BestOfN { n: usize },
    /// Accept first proposal that passes validation
    FirstValid,
    /// Collect all within timeout, select best
    BestWithinTimeout { timeout_ms: u64 },
}

/// A proposal from an Executor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    /// Unique proposal ID
    pub id: String,
    /// Request ID this proposal responds to
    pub request_id: String,
    /// Executor that generated this proposal
    pub executor_id: String,
    /// The proposed content/code
    pub content: String,
    /// Self-reported quality score (0-100)
    pub quality_score: f64,
    /// Confidence level (0-1)
    pub confidence: f64,
    /// Generation latency in ms
    pub latency_ms: u64,
    /// Additional metadata
    pub metadata: serde_json::Value,
    /// Timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Proposal {
    pub fn new(
        request_id: impl Into<String>,
        executor_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            request_id: request_id.into(),
            executor_id: executor_id.into(),
            content: content.into(),
            quality_score: 0.0,
            confidence: 0.0,
            latency_ms: 0,
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
        }
    }

    pub fn with_scores(mut self, quality: f64, confidence: f64) -> Self {
        self.quality_score = quality;
        self.confidence = confidence;
        self
    }

    pub fn with_latency(mut self, latency_ms: u64) -> Self {
        self.latency_ms = latency_ms;
        self
    }
}

/// Request for proposals.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalRequest {
    /// Unique request ID
    pub id: String,
    /// The task/prompt
    pub prompt: String,
    /// Number of proposals requested
    pub count: usize,
    /// Target supervisors (empty = broadcast to all)
    pub target_supervisors: Vec<String>,
    /// Timeout in ms
    pub timeout_ms: u64,
    /// Trace ID
    pub trace_id: Option<String>,
    /// Timestamp
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl ProposalRequest {
    pub fn new(prompt: impl Into<String>, count: usize) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            prompt: prompt.into(),
            count,
            target_supervisors: vec![],
            timeout_ms: 5000,
            trace_id: None,
            created_at: chrono::Utc::now(),
        }
    }
}

/// Scored proposal (after evaluation).
#[derive(Debug, Clone)]
pub struct ScoredProposal {
    pub proposal: Proposal,
    pub final_score: f64,
}

/// The Arbiter for proposal selection.
pub struct Arbiter {
    config: ArbiterConfig,
    /// Pending requests awaiting proposals
    pending_requests: Arc<RwLock<HashMap<String, ProposalRequest>>>,
    /// Collected proposals by request ID
    proposals: Arc<RwLock<HashMap<String, Vec<Proposal>>>>,
    /// Custom scorer function
    scorer: Arc<dyn Fn(&Proposal) -> f64 + Send + Sync>,
}

impl Arbiter {
    /// Create a new Arbiter with default config.
    pub fn new() -> Self {
        Self::with_config(ArbiterConfig::default())
    }

    /// Create with custom config.
    pub fn with_config(config: ArbiterConfig) -> Self {
        Self {
            config,
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            proposals: Arc::new(RwLock::new(HashMap::new())),
            scorer: Arc::new(default_scorer),
        }
    }

    /// Set a custom scorer function.
    pub fn with_scorer<F>(mut self, scorer: F) -> Self
    where
        F: Fn(&Proposal) -> f64 + Send + Sync + 'static,
    {
        self.scorer = Arc::new(scorer);
        self
    }

    /// Request proposals for a task.
    pub async fn request_proposals(
        &self,
        prompt: impl Into<String>,
        count: usize,
    ) -> Result<String, BusError> {
        let request = ProposalRequest::new(prompt, count);
        let request_id = request.id.clone();

        // Store the pending request
        let mut pending = self.pending_requests.write().await;
        pending.insert(request_id.clone(), request);

        // Initialize proposals storage
        let mut proposals = self.proposals.write().await;
        proposals.insert(request_id.clone(), Vec::new());

        // In a real implementation, we would publish to NATS:
        // nats.publish("bus.proposals.request", &request).await?;

        Ok(request_id)
    }

    /// Submit a proposal (called by Executors).
    pub async fn submit_proposal(&self, proposal: Proposal) -> Result<(), BusError> {
        let mut proposals = self.proposals.write().await;

        if let Some(list) = proposals.get_mut(&proposal.request_id) {
            list.push(proposal);
            Ok(())
        } else {
            Err(BusError::PublishFailed(format!(
                "Unknown request ID: {}",
                proposal.request_id
            )))
        }
    }

    /// Select the best proposal for a request.
    pub async fn select_best(&self, request_id: &str) -> Result<ScoredProposal, BusError> {
        // Wait for proposals based on strategy
        let proposals = self.wait_for_proposals(request_id).await?;

        if proposals.is_empty() {
            return Err(BusError::Timeout);
        }

        // Score all proposals
        let mut scored: Vec<ScoredProposal> = proposals
            .into_iter()
            .map(|p| {
                let score = (self.scorer)(&p);
                ScoredProposal {
                    proposal: p,
                    final_score: score,
                }
            })
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.final_score.partial_cmp(&a.final_score).unwrap());

        // Return the best one
        Ok(scored.remove(0))
    }

    /// Wait for proposals based on strategy.
    async fn wait_for_proposals(&self, request_id: &str) -> Result<Vec<Proposal>, BusError> {
        let timeout = Duration::from_millis(self.config.proposal_timeout_ms);
        let start = std::time::Instant::now();

        let expected_count = match &self.config.strategy {
            ArbiterStrategy::BestOfN { n } => *n,
            ArbiterStrategy::FirstValid => 1,
            ArbiterStrategy::BestWithinTimeout { .. } => usize::MAX,
        };

        loop {
            let proposals = self.proposals.read().await;
            if let Some(list) = proposals.get(request_id) {
                if list.len() >= expected_count {
                    return Ok(list.clone());
                }
            }
            drop(proposals);

            if start.elapsed() >= timeout {
                // Return what we have
                let proposals = self.proposals.read().await;
                return Ok(proposals.get(request_id).cloned().unwrap_or_default());
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Get all proposals for a request.
    pub async fn get_proposals(&self, request_id: &str) -> Vec<Proposal> {
        let proposals = self.proposals.read().await;
        proposals.get(request_id).cloned().unwrap_or_default()
    }

    /// Clean up a completed request.
    pub async fn cleanup(&self, request_id: &str) {
        let mut pending = self.pending_requests.write().await;
        pending.remove(request_id);

        let mut proposals = self.proposals.write().await;
        proposals.remove(request_id);
    }
}

impl Default for Arbiter {
    fn default() -> Self {
        Self::new()
    }
}

/// Default scorer: weighted combination of quality, confidence, and latency.
fn default_scorer(proposal: &Proposal) -> f64 {
    proposal.quality_score * 0.5 + proposal.confidence * 100.0 * 0.3 - proposal.latency_ms as f64 * 0.001
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_arbiter_best_of_n() {
        let arbiter = Arbiter::with_config(ArbiterConfig {
            strategy: ArbiterStrategy::BestOfN { n: 2 },
            proposal_timeout_ms: 1000,
            ..Default::default()
        });

        // Request proposals
        let request_id = arbiter.request_proposals("Test task", 2).await.unwrap();

        // Submit proposals
        arbiter
            .submit_proposal(
                Proposal::new(&request_id, "exe-1", "Code A").with_scores(80.0, 0.9),
            )
            .await
            .unwrap();

        arbiter
            .submit_proposal(
                Proposal::new(&request_id, "exe-2", "Code B").with_scores(95.0, 0.85),
            )
            .await
            .unwrap();

        // Select best
        let best = arbiter.select_best(&request_id).await.unwrap();
        assert_eq!(best.proposal.executor_id, "exe-2"); // Higher quality
    }

    #[tokio::test]
    async fn test_custom_scorer() {
        let arbiter = Arbiter::new().with_scorer(|p| {
            // Only care about confidence
            p.confidence * 100.0
        });

        let request_id = arbiter.request_proposals("Test", 2).await.unwrap();

        arbiter
            .submit_proposal(
                Proposal::new(&request_id, "exe-1", "A").with_scores(100.0, 0.5), // Low confidence
            )
            .await
            .unwrap();

        arbiter
            .submit_proposal(
                Proposal::new(&request_id, "exe-2", "B").with_scores(50.0, 0.99), // High confidence
            )
            .await
            .unwrap();

        let best = arbiter.select_best(&request_id).await.unwrap();
        assert_eq!(best.proposal.executor_id, "exe-2"); // Higher confidence wins
    }
}
