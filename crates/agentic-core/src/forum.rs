//! Durable, fan-out agent forum backed by JetStream KV.
//!
//! Posts are immutable. Each reader owns its cursor: reading never consumes a
//! post for another agent. Retain the bucket until the experiment is archived.
use anyhow::{ensure, Context, Result};
use async_nats::jetstream::kv;
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ForumPost {
    pub id: String,
    pub author: String,
    pub parent_ids: Vec<String>,
    pub body: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ForumEntry {
    pub revision: u64,
    pub post: ForumPost,
}

#[derive(Clone)]
pub struct NatsForum {
    store: kv::Store,
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

impl NatsForum {
    /// Read one immutable post without advancing any reader cursor.
    pub async fn get(&self, id: &str) -> Result<Option<ForumEntry>> {
        ensure!(valid_id(id), "Invalid post identity");
        self.store
            .entry(id)
            .await?
            .map(|e| {
                ensure!(e.operation == kv::Operation::Put, "Post was deleted");
                Ok(ForumEntry {
                    revision: e.revision,
                    post: serde_json::from_slice(&e.value)?,
                })
            })
            .transpose()
    }

    /// A task is a four-stage immutable chain. The deterministic stage key is
    /// created atomically in JetStream: competing claimants cannot both win.
    /// Retrying identical content is idempotent, including after a lost ACK.
    pub async fn task_transition(
        &self,
        id: &str,
        author: &str,
        stage: u8,
        payload: Value,
    ) -> Result<ForumEntry> {
        ensure!(
            valid_id(id) && id.len() <= 80 && stage <= 3,
            "Invalid task stage"
        );
        ensure!(payload.is_object(), "Task payload must be an object");
        let key = format!("{id}_t{stage}");
        let parents = if stage == 0 {
            ensure!(
                payload["question"]
                    .as_str()
                    .is_some_and(|q| !q.trim().is_empty() && q.len() <= 2000),
                "Question required"
            );
            ensure!(
                payload["criterion"]
                    .as_str()
                    .is_some_and(|q| !q.trim().is_empty() && q.len() <= 2000),
                "Acceptance criterion required"
            );
            ensure!(
                payload["search_budget"]
                    .as_u64()
                    .is_some_and(|b| (1..=4).contains(&b)),
                "Search budget must be 1..4"
            );
            serde_json::from_value(
                payload
                    .get("parent_ids")
                    .cloned()
                    .unwrap_or(serde_json::json!([])),
            )?
        } else {
            let request = self
                .get(&format!("{id}_t0"))
                .await?
                .context("Unknown task")?;
            ensure!(request.post.body["kind"] == "work_task", "Not a task");
            if stage == 1 {
                ensure!(
                    !request.post.body["payload"]["excluded_agents"]
                        .as_array()
                        .is_some_and(|a| a.iter().any(|v| v == author)),
                    "Agent excluded from this retry"
                );
                if let Some(until) = payload.get("lease_until") {
                    let date = chrono::DateTime::parse_from_rfc3339(
                        until.as_str().context("Invalid lease")?,
                    )?;
                    let left =
                        (date.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds();
                    ensure!(
                        (1..=600).contains(&left),
                        "Lease must expire within 600 seconds"
                    );
                }
                ensure!(
                    request.post.author != author,
                    "Requester cannot claim own task"
                );
                if let Some(target) = request.post.body["payload"]["target"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                {
                    ensure!(target == author, "Task assigned to another agent");
                }
            } else if stage == 2 {
                let claim = self
                    .get(&format!("{id}_t1"))
                    .await?
                    .context("Task not claimed")?;
                ensure!(claim.post.author == author, "Only the claimant can report");
                if let Some(until) = claim.post.body["payload"]["lease_until"].as_str() {
                    ensure!(
                        chrono::Utc::now() < chrono::DateTime::parse_from_rfc3339(until)?,
                        "Task lease expired"
                    );
                }
                ensure!(
                    ["evidence", "insufficient_evidence", "blocked"]
                        .contains(&payload["outcome"].as_str().unwrap_or("")),
                    "Invalid outcome"
                );
                ensure!(
                    payload["text"]
                        .as_str()
                        .is_some_and(|s| (40..=4000).contains(&s.len())),
                    "Report required"
                );
            } else {
                ensure!(request.post.author == author, "Only requester can resolve");
                ensure!(
                    ["adopted", "rejected", "inconclusive"]
                        .contains(&payload["decision"].as_str().unwrap_or("")),
                    "Invalid decision"
                );
                ensure!(
                    payload["text"]
                        .as_str()
                        .is_some_and(|s| (40..=4000).contains(&s.len())),
                    "Resolution reason required"
                );
            }
            vec![format!("{id}_t{}", stage - 1)]
        };
        let post = ForumPost {
            id: key,
            author: author.into(),
            parent_ids: parents,
            body: serde_json::json!({"kind":"work_task","task_id":id,"stage":stage,"payload":payload,"verified":false}),
        };
        let revision = self.publish(post.clone()).await?;
        Ok(ForumEntry { revision, post })
    }
    /// Resolve an expired reservation using the SAME immutable report key as
    /// the claimant. A late report and expiry can never both win publication.
    pub async fn expire_task(&self, id: &str) -> Result<Option<ForumEntry>> {
        let Some(claim) = self.get(&format!("{id}_t1")).await? else {
            return Ok(None);
        };
        let Some(until) = claim.post.body["payload"]["lease_until"].as_str() else {
            return Ok(None);
        };
        let until = chrono::DateTime::parse_from_rfc3339(until)?;
        if chrono::Utc::now() < until || self.get(&format!("{id}_t2")).await?.is_some() {
            return Ok(None);
        }
        let post = ForumPost {
            id: format!("{id}_t2"),
            author: "runtime".into(),
            parent_ids: vec![claim.post.id],
            body: serde_json::json!({"kind":"work_task","task_id":id,"stage":2,"payload":{"outcome":"blocked","reason":"lease_expired","text":"La réservation a expiré sans résultat durable. Le runtime clôt cette tentative ; aucun résultat de recherche n’est inventé.","sources":[],"source_ids":[],"evidence_seqs":[]},"verified":false}),
        };
        match self.publish(post.clone()).await {
            Ok(revision) => Ok(Some(ForumEntry { revision, post })),
            Err(e) => {
                if self.get(&format!("{id}_t2")).await?.is_some() {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }
    /// Namespace must identify one experiment, never an agent or a queue group.
    pub async fn open(url: &str, namespace: &str) -> Result<Self> {
        ensure!(valid_id(namespace), "Invalid forum namespace");
        let client = async_nats::connect(url).await?;
        let js = async_nats::jetstream::new(client);
        let bucket = format!("forum_{namespace}");
        let store = match js.get_key_value(&bucket).await {
            Ok(store) => store,
            Err(_) => match js
                .create_key_value(kv::Config {
                    bucket: bucket.clone(),
                    description: "Immutable agent forum posts".into(),
                    history: 1,
                    max_value_size: 65536,
                    // File storage, no TTL and no discard-by-size: history must not
                    // silently disappear. Owners explicitly delete archived buckets.
                    ..Default::default()
                })
                .await
            {
                Ok(store) => store,
                // Another participant may have created the same bucket.
                Err(error) => js
                    .get_key_value(&bucket)
                    .await
                    .with_context(|| format!("Cannot open forum: {error}"))?,
            },
        };
        Ok(Self { store })
    }

    /// A successful return means JetStream acknowledged persistence. Retrying
    /// the identical ID/body is safe; changing an existing post is rejected.
    pub async fn publish(&self, post: ForumPost) -> Result<u64> {
        ensure!(
            valid_id(&post.id) && valid_id(&post.author),
            "Invalid post identity"
        );
        ensure!(post.parent_ids.len() <= 16, "Too many parents");
        for parent in &post.parent_ids {
            ensure!(valid_id(parent) && parent != &post.id, "Invalid parent");
            ensure!(self.store.entry(parent).await?.is_some(), "Unknown parent");
        }
        let bytes = serde_json::to_vec(&post)?;
        ensure!(bytes.len() <= 65536, "Forum post exceeds 64 KiB");
        match self.store.create(&post.id, bytes.into()).await {
            Ok(revision) => Ok(revision),
            Err(error) => {
                let existing = self.store.entry(&post.id).await?.context(error)?;
                ensure!(
                    serde_json::from_slice::<ForumPost>(&existing.value)? == post,
                    "Post ID already belongs to different content"
                );
                Ok(existing.revision)
            }
        }
    }

    /// Replay in server revision order. Advance your cursor only to the last
    /// entry actually delivered, never to the current stream head.
    pub async fn read_after(&self, cursor: u64, limit: usize) -> Result<Vec<ForumEntry>> {
        ensure!((1..=256).contains(&limit), "Read limit must be 1..256");
        let keys: Vec<String> = self.store.keys().await?.try_collect().await?;
        let mut entries = Vec::new();
        for key in keys {
            let entry = self
                .store
                .entry(&key)
                .await?
                .context("Forum post disappeared")?;
            ensure!(
                entry.operation == kv::Operation::Put,
                "Forum history was deleted"
            );
            if entry.revision > cursor {
                entries.push(ForumEntry {
                    revision: entry.revision,
                    post: serde_json::from_slice(&entry.value)?,
                });
            }
        }
        entries.sort_by_key(|e| e.revision);
        entries.truncate(limit);
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires a local NATS server with JetStream; NATS_TEST_URL"]
    async fn durable_fanout_and_idempotency() {
        let url = std::env::var("NATS_TEST_URL").unwrap_or_else(|_| "nats://127.0.0.1:4222".into());
        let namespace = uuid::Uuid::new_v4().to_string();
        let a = NatsForum::open(&url, &namespace).await.unwrap();
        let post = ForumPost {
            id: "p1".into(),
            author: "A001".into(),
            parent_ids: vec![],
            body: serde_json::json!({"claim":"unverified"}),
        };
        let revision = a.publish(post.clone()).await.unwrap();
        assert_eq!(a.publish(post.clone()).await.unwrap(), revision);
        let mut collision = post.clone();
        collision.body = Value::Null;
        assert!(a.publish(collision).await.is_err());
        let b = NatsForum::open(&url, &namespace).await.unwrap();
        assert_eq!(a.read_after(0, 1).await.unwrap()[0].post, post);
        assert_eq!(b.read_after(0, 1).await.unwrap()[0].post, post);
        assert!(b.read_after(revision, 1).await.unwrap().is_empty());
        let reply = ForumPost {
            id: "p2".into(),
            author: "A002".into(),
            parent_ids: vec!["p1".into()],
            body: Value::Null,
        };
        b.publish(reply.clone()).await.unwrap();
        assert_eq!(a.read_after(revision, 1).await.unwrap()[0].post, reply);
        let other = NatsForum::open(&url, &uuid::Uuid::new_v4().to_string())
            .await
            .unwrap();
        assert!(other.read_after(0, 1).await.unwrap().is_empty());
        assert!(other
            .publish(ForumPost {
                id: "bad".into(),
                author: "A001".into(),
                parent_ids: vec!["p1".into()],
                body: Value::Null
            })
            .await
            .is_err());
    }
}
