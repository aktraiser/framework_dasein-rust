//! Waiting for a NATS server that is away.

use std::time::Duration;

/// Whether an error from NATS is of the kind that goes away by itself: the
/// server or the network is not there right now, or the store is full for
/// the moment. Told from its text, the only thing the client gives for most
/// of them. A refusal of who we are is never of that kind.
pub(crate) fn passing(error: &dyn std::fmt::Display) -> bool {
    let text = error.to_string().to_lowercase();
    let refused = [
        "authorization",
        "authentication",
        "permission",
        "not allowed",
    ];
    if refused.iter().any(|sign| text.contains(sign)) {
        return false;
    }
    [
        "timed out",
        "timeout",
        "no responders",
        "connection refused",
        "connection reset",
        "connection closed",
        "connection error",
        "connection aborted",
        "disconnect",
        "not connected",
        "broken pipe",
        "unavailable",
        // A store that is full for now: the result ledger empties as its
        // records expire, the task queue as workers take tasks.
        "as records expire",
        "maximum bytes exceeded",
        "maximum messages exceeded",
        // A state listing cut short.
        "listing stalled",
        "listing ended early",
    ]
    .iter()
    .any(|sign| text.contains(sign))
}

/// How long a caller goes on trying through errors that pass.
pub(crate) struct Patience {
    until: tokio::time::Instant,
    pause: std::sync::atomic::AtomicU64,
}

impl Patience {
    pub(crate) fn new(time: Duration) -> Self {
        Self::until(tokio::time::Instant::now() + time)
    }

    pub(crate) fn until(until: tokio::time::Instant) -> Self {
        Self {
            until,
            pause: std::sync::atomic::AtomicU64::new(100),
        }
    }

    /// For an error that passes, with time left: how long to wait before
    /// trying again (longer each time, two seconds at most). `None` when
    /// the error will not pass or the time is up.
    pub(crate) fn pause_for(&self, error: &dyn std::fmt::Display) -> Option<Duration> {
        use std::sync::atomic::Ordering;
        let pause = Duration::from_millis(self.pause.load(Ordering::Relaxed));
        if !passing(error) || tokio::time::Instant::now() + pause >= self.until {
            return None;
        }
        tracing::debug!(%error, retry_in = ?pause, "NATS is not answering, trying again");
        let next = u64::try_from(pause.as_millis())
            .unwrap_or(2000)
            .saturating_mul(2)
            .min(2000);
        self.pause.store(next, Ordering::Relaxed);
        Some(pause)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_that_pass_are_told_from_the_others() {
        for passes in [
            "timed out: didn't receive ack in time",
            "request timeout",
            "no responders available for request",
            "Connection refused (os error 61)",
            "client disconnected",
            "Could not record the task (x); a full result ledger takes new tasks again as records expire",
            "NATS shared state: listing stalled",
            // What NATS answers when a result is written to a full ledger.
            "failed getting entry: publish failed: maximum bytes exceeded (code 503, error code 10077)",
        ] {
            assert!(passing(&passes), "{passes}");
        }
        for stays in [
            "Task ID is already assigned to different work",
            "maximum message size exceeded",
            "Existing queue configuration differs; migrate it explicitly",
            // Never worth waiting for, whatever else the message says.
            "connection refused: authorization violation",
            "permissions violation for publish, request timed out",
            "maximum connections exceeded",
        ] {
            assert!(!passing(&stays), "{stays}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn patience_grows_its_pauses_and_ends() {
        let patience = Patience::new(Duration::from_secs(10));
        // An error that will not pass is not waited for.
        assert!(patience.pause_for(&"different work").is_none());
        let mut pauses = Vec::new();
        while let Some(pause) = patience.pause_for(&"timed out") {
            pauses.push(pause.as_millis());
            tokio::time::sleep(pause).await;
        }
        assert_eq!(&pauses[..6], [100, 200, 400, 800, 1600, 2000]);
        // It stops before the time is over, never after.
        assert!(pauses.iter().sum::<u128>() < 10_000);
    }
}
