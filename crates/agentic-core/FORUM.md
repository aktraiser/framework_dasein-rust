# NATS agent forum

`dasein_agentic_core::forum::NatsForum` is a durable fan-out forum. It uses a
JetStream KV bucket per experiment, immutable posts and independent reader
cursors. It does not use a work queue: every participant can read every post.

```rust,no_run
use dasein_agentic_core::forum::{NatsForum, ForumPost};
use serde_json::json;

# async fn example() -> anyhow::Result<()> {
let forum = NatsForum::open("nats://127.0.0.1:4222", "experiment_42").await?;
let revision = forum.publish(ForumPost {
    id: "finding_1".into(),
    author: "A001".into(),
    parent_ids: vec![],
    body: json!({"hypothesis": "Unverified claim", "sources": []}),
}).await?;
let received = forum.read_after(0, 8).await?;
// Advance only after passing these entries to the recipient.
let cursor = received.last().map(|e| e.revision).unwrap_or(0);
assert_eq!(cursor, revision);
# Ok(())
# }
```

Retries of identical post ID/content return the original revision. An ID
collision with different content or a missing parent is an error. Posts are
limited to 64 KiB; page size is 1–256. File storage has no TTL or automatic
size-based eviction. Owners must archive and remove experiment buckets.
KV key enumeration is intended for bounded experiments; use dedicated durable
stream consumers for very large forums.

This module provides transport and persistence, not automatic reasoning.
The caller must deliver posts to model contexts, scope authors/namespaces,
enforce source provenance, record receipts and implement collaboration policy.
It assumes trusted clients. Separate NATS accounts/permissions are needed for
mutually untrusted remote agents. Bound connection/read deadlines at the
application boundary and handle unavailable NATS explicitly.

`ChatAgent::with_completion_check` optionally enforces an application completion
contract. Rejected final answers receive feedback, consume the same bounded
round budget and fail when that budget is exhausted. Streaming follows the same
contract. The default conversational behavior is unchanged.
`with_tool_timeout` configures a per-tool timeout (default 30 seconds), including
any queue wait inside the handler.

The laboratory integration provides automatic incremental delivery, sourced
hypotheses, replies, a separate review phase for shared news investigations and
explicit insufficient-evidence conclusions. A received message, a citation and
an agreeing review remain distinct from independent verification.

Tests:

```sh
cargo test -p dasein-agentic-core --lib chat_agent
NATS_TEST_URL=nats://127.0.0.1:4222 cargo test -p dasein-agentic-core --lib forum::tests -- --ignored
```

## Terminal tools and changing tool availability

`ChatAgent::with_terminal_tool("finish")` marks a registered tool as a terminal
operation. A successful handler result (without an `error` property), accepted
by the optional completion check, ends the invocation immediately. A string
`conclusion` field becomes the answer; otherwise the result is serialized.
No extra model request occurs. Remaining calls in the same batch receive
skipped results without executing. Failed finalization can retry within the
existing round limit. The final round permits terminal calls only. Truncated
batches (`FinishReason::Length`) execute no tools and request a shorter retry.

`with_tool_filter(|name, round| ...)` chooses which registered tools are offered
and accepted in each round (zero based). The original registry is preserved.
This can reserve the last round for a finish tool or hide exhausted quotas;
handlers must still validate quotas and permissions because a batch can contain
multiple calls. Neither feature changes the default non-terminal tool behavior.

The lab uses ordinary `ForumPost` JSON bodies for working questions, answers,
requests, observations and decisions; replies carry parent IDs. It implements
thread reads and bounded waits that release an execution semaphore permit.
Messages summarize an agent's work and uncertainties; transport does not expose
hidden model reasoning or guarantee that a message will receive a useful answer.
