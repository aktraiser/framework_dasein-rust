//! Dynamic executor allocation between supervisors.
//!
//! # Quick Start
//!
//! ```rust
//! // Request help
//! let request = AllocationRequest::new("sup-code", 2)
//!     .reason(AllocationReason::Overloaded { queue_depth: 15 });
//!
//! // Grant executors
//! let grant = AllocationGrant::new("sup-data", "sup-code", vec!["exe-001", "exe-002"])
//!     .duration_secs(60);
//! ```

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Reason for requesting allocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AllocationReason {
    /// Queue is too deep
    Overloaded { queue_depth: usize },
    /// Need specific capability
    SpecializedTask { capability: String },
    /// Urgent deadline
    Deadline { remaining_ms: u64 },
    /// Traffic burst
    TrafficBurst { requests_per_sec: f64 },
}

/// Request for executor allocation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationRequest {
    /// Request ID
    pub id: String,
    /// Requesting supervisor
    pub from_supervisor: String,
    /// Reason
    pub reason: AllocationReason,
    /// Number of executors needed
    pub executors_needed: usize,
    /// Max duration in ms
    pub max_duration_ms: u64,
    /// Priority (higher = more urgent)
    pub priority: i32,
    /// Created at
    pub created_at: DateTime<Utc>,
}

impl AllocationRequest {
    /// Create a new allocation request.
    pub fn new(from_supervisor: impl Into<String>, executors_needed: usize) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            from_supervisor: from_supervisor.into(),
            reason: AllocationReason::Overloaded { queue_depth: 10 },
            executors_needed,
            max_duration_ms: 60_000,
            priority: 0,
            created_at: Utc::now(),
        }
    }

    /// Set reason.
    pub fn reason(mut self, reason: AllocationReason) -> Self {
        self.reason = reason;
        self
    }

    /// Set priority.
    pub fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    /// Set max duration.
    pub fn duration_ms(mut self, ms: u64) -> Self {
        self.max_duration_ms = ms;
        self
    }

    /// Set duration in seconds.
    pub fn duration_secs(self, secs: u64) -> Self {
        self.duration_ms(secs * 1000)
    }
}

/// Offer of executors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationOffer {
    /// Offer ID
    pub id: String,
    /// Offering supervisor
    pub from_supervisor: String,
    /// To supervisor
    pub to_supervisor: String,
    /// Request ID being answered
    pub request_id: String,
    /// Available executor IDs
    pub executor_ids: Vec<String>,
    /// Max duration offered
    pub max_duration_ms: u64,
    /// Can be revoked early
    pub revocable: bool,
}

impl AllocationOffer {
    /// Create a new offer.
    pub fn new(
        from_supervisor: impl Into<String>,
        to_supervisor: impl Into<String>,
        request_id: impl Into<String>,
        executor_ids: Vec<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            from_supervisor: from_supervisor.into(),
            to_supervisor: to_supervisor.into(),
            request_id: request_id.into(),
            executor_ids,
            max_duration_ms: 60_000,
            revocable: true,
        }
    }

    /// Set duration.
    pub fn duration_ms(mut self, ms: u64) -> Self {
        self.max_duration_ms = ms;
        self
    }

    /// Set non-revocable.
    pub fn non_revocable(mut self) -> Self {
        self.revocable = false;
        self
    }
}

/// Granted allocation (lease).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationGrant {
    /// Lease ID
    pub lease_id: String,
    /// From supervisor (lender)
    pub from_supervisor: String,
    /// To supervisor (borrower)
    pub to_supervisor: String,
    /// Executor IDs
    pub executor_ids: Vec<String>,
    /// Granted at
    pub granted_at: DateTime<Utc>,
    /// Expires at
    pub expires_at: DateTime<Utc>,
    /// Is revocable
    pub revocable: bool,
}

impl AllocationGrant {
    /// Create a new grant.
    pub fn new(
        from_supervisor: impl Into<String>,
        to_supervisor: impl Into<String>,
        executor_ids: Vec<String>,
    ) -> Self {
        let now = Utc::now();
        Self {
            lease_id: Uuid::new_v4().to_string(),
            from_supervisor: from_supervisor.into(),
            to_supervisor: to_supervisor.into(),
            executor_ids,
            granted_at: now,
            expires_at: now + Duration::seconds(60),
            revocable: true,
        }
    }

    /// Set duration in seconds.
    pub fn duration_secs(mut self, secs: i64) -> Self {
        self.expires_at = self.granted_at + Duration::seconds(secs);
        self
    }

    /// Check if expired.
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Time remaining in ms.
    pub fn remaining_ms(&self) -> i64 {
        (self.expires_at - Utc::now()).num_milliseconds().max(0)
    }
}

/// Release of allocated executors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationRelease {
    /// Lease ID
    pub lease_id: String,
    /// Reason for release
    pub reason: ReleaseReason,
    /// Released at
    pub released_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseReason {
    /// Lease expired
    Expired,
    /// Returned early by borrower
    ReturnedEarly,
    /// Revoked by lender
    Revoked,
    /// Error
    Error { message: String },
}

/// Manages allocations for a supervisor.
#[derive(Debug)]
pub struct AllocationManager {
    /// Supervisor ID
    supervisor_id: String,
    /// Active grants where we are the lender
    lent_grants: Arc<RwLock<HashMap<String, AllocationGrant>>>,
    /// Active grants where we are the borrower
    borrowed_grants: Arc<RwLock<HashMap<String, AllocationGrant>>>,
    /// Max executors we can lend
    max_lend: usize,
    /// Max executors we can borrow
    max_borrow: usize,
}

impl AllocationManager {
    /// Create a new allocation manager.
    pub fn new(supervisor_id: impl Into<String>) -> Self {
        Self {
            supervisor_id: supervisor_id.into(),
            lent_grants: Arc::new(RwLock::new(HashMap::new())),
            borrowed_grants: Arc::new(RwLock::new(HashMap::new())),
            max_lend: 4,
            max_borrow: 4,
        }
    }

    /// Set max lend.
    pub fn max_lend(mut self, n: usize) -> Self {
        self.max_lend = n;
        self
    }

    /// Set max borrow.
    pub fn max_borrow(mut self, n: usize) -> Self {
        self.max_borrow = n;
        self
    }

    /// Record a grant where we are the lender.
    pub async fn record_lent(&self, grant: AllocationGrant) {
        self.lent_grants.write().await.insert(grant.lease_id.clone(), grant);
    }

    /// Record a grant where we are the borrower.
    pub async fn record_borrowed(&self, grant: AllocationGrant) {
        self.borrowed_grants.write().await.insert(grant.lease_id.clone(), grant);
    }

    /// Remove a grant.
    pub async fn remove_grant(&self, lease_id: &str) {
        self.lent_grants.write().await.remove(lease_id);
        self.borrowed_grants.write().await.remove(lease_id);
    }

    /// Get count of lent executors.
    pub async fn lent_count(&self) -> usize {
        self.lent_grants.read().await.values()
            .map(|g| g.executor_ids.len())
            .sum()
    }

    /// Get count of borrowed executors.
    pub async fn borrowed_count(&self) -> usize {
        self.borrowed_grants.read().await.values()
            .map(|g| g.executor_ids.len())
            .sum()
    }

    /// Can we lend more?
    pub async fn can_lend(&self, count: usize) -> bool {
        self.lent_count().await + count <= self.max_lend
    }

    /// Can we borrow more?
    pub async fn can_borrow(&self, count: usize) -> bool {
        self.borrowed_count().await + count <= self.max_borrow
    }

    /// Get expired grants.
    pub async fn get_expired_grants(&self) -> Vec<AllocationGrant> {
        let mut expired = Vec::new();

        for grant in self.lent_grants.read().await.values() {
            if grant.is_expired() {
                expired.push(grant.clone());
            }
        }

        for grant in self.borrowed_grants.read().await.values() {
            if grant.is_expired() {
                expired.push(grant.clone());
            }
        }

        expired
    }

    /// Clean up expired grants.
    pub async fn cleanup_expired(&self) -> Vec<String> {
        let mut removed = Vec::new();

        {
            let mut lent = self.lent_grants.write().await;
            lent.retain(|id, grant| {
                if grant.is_expired() {
                    removed.push(id.clone());
                    false
                } else {
                    true
                }
            });
        }

        {
            let mut borrowed = self.borrowed_grants.write().await;
            borrowed.retain(|id, grant| {
                if grant.is_expired() {
                    removed.push(id.clone());
                    false
                } else {
                    true
                }
            });
        }

        removed
    }

    /// Get all borrowed executor IDs.
    pub async fn borrowed_executor_ids(&self) -> Vec<String> {
        self.borrowed_grants.read().await.values()
            .flat_map(|g| g.executor_ids.clone())
            .collect()
    }

    /// Get all lent executor IDs.
    pub async fn lent_executor_ids(&self) -> Vec<String> {
        self.lent_grants.read().await.values()
            .flat_map(|g| g.executor_ids.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocation_request() {
        let req = AllocationRequest::new("sup-code", 2)
            .reason(AllocationReason::Overloaded { queue_depth: 15 })
            .priority(1)
            .duration_secs(120);

        assert_eq!(req.from_supervisor, "sup-code");
        assert_eq!(req.executors_needed, 2);
        assert_eq!(req.max_duration_ms, 120_000);
    }

    #[test]
    fn test_allocation_grant() {
        let grant = AllocationGrant::new("sup-data", "sup-code", vec!["exe-001".into()])
            .duration_secs(60);

        assert!(!grant.is_expired());
        assert!(grant.remaining_ms() > 0);
    }

    #[tokio::test]
    async fn test_allocation_manager() {
        let manager = AllocationManager::new("sup-001")
            .max_lend(4)
            .max_borrow(4);

        assert!(manager.can_lend(2).await);
        assert!(manager.can_borrow(2).await);

        let grant = AllocationGrant::new("sup-001", "sup-002", vec!["exe-001".into()]);
        manager.record_lent(grant).await;

        assert_eq!(manager.lent_count().await, 1);
    }
}
