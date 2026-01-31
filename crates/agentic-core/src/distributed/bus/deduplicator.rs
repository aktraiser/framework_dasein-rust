//! Deduplicator - Prevent duplicate task processing.
//!
//! Uses a time-windowed approach to track processed items:
//! - Content hash deduplication
//! - Idempotency key support
//! - Configurable retention window

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use super::types::BusError;

/// Deduplicator configuration.
#[derive(Debug, Clone)]
pub struct DeduplicatorConfig {
    /// Time window for deduplication
    pub window: Duration,
    /// Deduplication strategy
    pub strategy: DedupeStrategy,
    /// Maximum entries to track
    pub max_entries: usize,
}

impl Default for DeduplicatorConfig {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(300), // 5 minutes
            strategy: DedupeStrategy::ContentHash,
            max_entries: 10000,
        }
    }
}

/// Strategy for detecting duplicates.
#[derive(Debug, Clone)]
pub enum DedupeStrategy {
    /// Hash the full content
    ContentHash,
    /// Use provided idempotency key
    IdempotencyKey,
    /// Hash specific fields
    FieldHash { fields: Vec<String> },
}

/// Entry tracking a processed item.
#[derive(Debug, Clone)]
struct DedupeEntry {
    processed_at: std::time::Instant,
    count: usize,
}

/// Deduplicator for preventing duplicate processing.
pub struct Deduplicator {
    config: DeduplicatorConfig,
    /// Processed items by hash
    entries: Arc<RwLock<HashMap<u64, DedupeEntry>>>,
}

impl Deduplicator {
    /// Create a new Deduplicator with default config.
    pub fn new() -> Self {
        Self::with_config(DeduplicatorConfig::default())
    }

    /// Create with custom config.
    pub fn with_config(config: DeduplicatorConfig) -> Self {
        Self {
            config,
            entries: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if content is a duplicate.
    pub async fn is_duplicate(&self, content: &str) -> bool {
        let hash = self.compute_hash(content);
        self.is_duplicate_hash(hash).await
    }

    /// Check if a hash is a duplicate.
    pub async fn is_duplicate_hash(&self, hash: u64) -> bool {
        let entries = self.entries.read().await;

        if let Some(entry) = entries.get(&hash) {
            // Check if within window
            if entry.processed_at.elapsed() < self.config.window {
                return true;
            }
        }

        false
    }

    /// Check with idempotency key.
    pub async fn is_duplicate_key(&self, key: &str) -> bool {
        let hash = self.hash_string(key);
        self.is_duplicate_hash(hash).await
    }

    /// Mark content as processed.
    pub async fn mark_processed(&self, content: &str) -> Result<(), BusError> {
        let hash = self.compute_hash(content);
        self.mark_processed_hash(hash).await
    }

    /// Mark a hash as processed.
    pub async fn mark_processed_hash(&self, hash: u64) -> Result<(), BusError> {
        let mut entries = self.entries.write().await;

        // Cleanup old entries if at capacity
        if entries.len() >= self.config.max_entries {
            self.cleanup_old_entries(&mut entries);
        }

        let now = std::time::Instant::now();

        entries
            .entry(hash)
            .and_modify(|e| {
                e.count += 1;
                e.processed_at = now;
            })
            .or_insert(DedupeEntry {
                processed_at: now,
                count: 1,
            });

        Ok(())
    }

    /// Mark with idempotency key.
    pub async fn mark_processed_key(&self, key: &str) -> Result<(), BusError> {
        let hash = self.hash_string(key);
        self.mark_processed_hash(hash).await
    }

    /// Check and mark atomically (returns true if was duplicate).
    pub async fn check_and_mark(&self, content: &str) -> bool {
        let hash = self.compute_hash(content);

        let mut entries = self.entries.write().await;
        let now = std::time::Instant::now();

        if let Some(entry) = entries.get_mut(&hash) {
            if entry.processed_at.elapsed() < self.config.window {
                entry.count += 1;
                return true; // Was duplicate
            }
            // Expired, treat as new
            entry.processed_at = now;
            entry.count = 1;
            return false;
        }

        // New entry
        entries.insert(
            hash,
            DedupeEntry {
                processed_at: now,
                count: 1,
            },
        );

        false
    }

    /// Compute hash based on strategy.
    fn compute_hash(&self, content: &str) -> u64 {
        match &self.config.strategy {
            DedupeStrategy::ContentHash => self.hash_string(content),
            DedupeStrategy::IdempotencyKey => self.hash_string(content),
            DedupeStrategy::FieldHash { .. } => {
                // For JSON content, we'd parse and hash specific fields
                // For now, just hash the whole thing
                self.hash_string(content)
            }
        }
    }

    /// Hash a string.
    fn hash_string(&self, s: &str) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }

    /// Cleanup expired entries.
    fn cleanup_old_entries(&self, entries: &mut HashMap<u64, DedupeEntry>) {
        let window = self.config.window;
        entries.retain(|_, entry| entry.processed_at.elapsed() < window);
    }

    /// Force cleanup of expired entries.
    pub async fn cleanup(&self) {
        let mut entries = self.entries.write().await;
        self.cleanup_old_entries(&mut entries);
    }

    /// Get statistics.
    pub async fn stats(&self) -> DeduplicatorStats {
        let entries = self.entries.read().await;
        let total_duplicates: usize = entries.values().map(|e| e.count.saturating_sub(1)).sum();

        DeduplicatorStats {
            tracked_entries: entries.len(),
            total_duplicates_prevented: total_duplicates,
        }
    }
}

impl Default for Deduplicator {
    fn default() -> Self {
        Self::new()
    }
}

/// Deduplicator statistics.
#[derive(Debug, Clone)]
pub struct DeduplicatorStats {
    pub tracked_entries: usize,
    pub total_duplicates_prevented: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_basic_deduplication() {
        let dedup = Deduplicator::new();

        let content = "Hello, world!";

        // First time: not duplicate
        assert!(!dedup.is_duplicate(content).await);

        // Mark as processed
        dedup.mark_processed(content).await.unwrap();

        // Second time: duplicate
        assert!(dedup.is_duplicate(content).await);
    }

    #[tokio::test]
    async fn test_check_and_mark() {
        let dedup = Deduplicator::new();

        let content = "Test content";

        // First call: not duplicate, marks it
        assert!(!dedup.check_and_mark(content).await);

        // Second call: duplicate
        assert!(dedup.check_and_mark(content).await);
    }

    #[tokio::test]
    async fn test_expiration() {
        let dedup = Deduplicator::with_config(DeduplicatorConfig {
            window: Duration::from_millis(50),
            ..Default::default()
        });

        let content = "Expiring content";

        dedup.mark_processed(content).await.unwrap();
        assert!(dedup.is_duplicate(content).await);

        // Wait for expiration
        tokio::time::sleep(Duration::from_millis(60)).await;

        // Should no longer be duplicate
        assert!(!dedup.is_duplicate(content).await);
    }

    #[tokio::test]
    async fn test_idempotency_key() {
        let dedup = Deduplicator::with_config(DeduplicatorConfig {
            strategy: DedupeStrategy::IdempotencyKey,
            ..Default::default()
        });

        let key = "request-12345";

        assert!(!dedup.is_duplicate_key(key).await);
        dedup.mark_processed_key(key).await.unwrap();
        assert!(dedup.is_duplicate_key(key).await);
    }
}
