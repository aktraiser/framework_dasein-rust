//! Per-key rate limits for the LLM endpoints.
//!
//! Limits come from the API key record (`rate_limit_rpm`, `rate_limit_tpm`)
//! and are counted in memory over one-minute windows, so they apply to one
//! gateway process and restart with it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);
/// Idle keys are dropped once the table grows past this size.
const MAX_KEYS: usize = 10_000;

#[derive(Clone, Copy)]
struct Usage {
    since: Instant,
    requests: u32,
    tokens: u64,
}

/// Request and token counters for each API key.
#[derive(Default)]
pub struct KeyLimits {
    usage: Mutex<HashMap<String, Usage>>,
}

impl KeyLimits {
    fn usage(&self) -> std::sync::MutexGuard<'_, HashMap<String, Usage>> {
        self.usage
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Count one request for `key_id`, unless its limits for the current
    /// minute are used up. A limit of `None`, zero or less means unlimited.
    ///
    /// # Errors
    /// Returns how long to wait before the window resets.
    pub fn admit(&self, key_id: &str, rpm: Option<i32>, tpm: Option<i32>) -> Result<(), Duration> {
        let limit =
            |value: Option<i32>| value.and_then(|v| u64::try_from(v).ok()).filter(|v| *v > 0);
        let now = Instant::now();
        let mut usage = self.usage();
        if usage.len() >= MAX_KEYS {
            usage.retain(|_, entry| now.duration_since(entry.since) < WINDOW);
        }
        let entry = usage.entry(key_id.to_owned()).or_insert(Usage {
            since: now,
            requests: 0,
            tokens: 0,
        });
        if now.duration_since(entry.since) >= WINDOW {
            *entry = Usage {
                since: now,
                requests: 0,
                tokens: 0,
            };
        }
        let over_requests = limit(rpm).is_some_and(|max| u64::from(entry.requests) >= max);
        // Tokens are only known after a response: the request that crosses
        // the limit goes through, the following ones wait.
        let over_tokens = limit(tpm).is_some_and(|max| entry.tokens >= max);
        if over_requests || over_tokens {
            return Err(WINDOW.saturating_sub(now.duration_since(entry.since)));
        }
        entry.requests += 1;
        Ok(())
    }

    /// Add the tokens a completed request consumed to the key's window.
    pub fn record_tokens(&self, key_id: &str, tokens: u64) {
        if let Some(entry) = self.usage().get_mut(key_id) {
            entry.tokens = entry.tokens.saturating_add(tokens);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_per_minute_are_capped_per_key() {
        let limits = KeyLimits::default();
        assert!(limits.admit("a", Some(2), None).is_ok());
        assert!(limits.admit("a", Some(2), None).is_ok());
        let wait = limits.admit("a", Some(2), None).unwrap_err();
        assert!(wait <= WINDOW && wait > Duration::from_secs(50));
        assert!(
            limits.admit("b", Some(2), None).is_ok(),
            "keys are independent"
        );
    }

    #[test]
    fn tokens_per_minute_block_following_requests() {
        let limits = KeyLimits::default();
        assert!(limits.admit("a", None, Some(100)).is_ok());
        limits.record_tokens("a", 150);
        assert!(limits.admit("a", None, Some(100)).is_err());
    }

    #[test]
    fn no_limit_means_unlimited() {
        let limits = KeyLimits::default();
        for _ in 0..1000 {
            assert!(limits.admit("a", None, None).is_ok());
            assert!(limits.admit("b", Some(0), Some(0)).is_ok());
        }
    }
}
