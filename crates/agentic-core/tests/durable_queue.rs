use dasein_agentic_core::distributed::bus::*;
use serde_json::json;
use std::{sync::Arc, time::Duration};
async fn open(namespace: &str, capacity: i64) -> DurableQueue {
    let nats = Arc::new(
        NatsClient::connect(NatsConfig::new(
            std::env::var("DASEIN_TEST_NATS_URL")
                .unwrap_or_else(|_| "nats://127.0.0.1:4222".into()),
        ))
        .await
        .unwrap(),
    );
    DurableQueue::new(
        nats,
        namespace,
        DurableQueueConfig {
            max_pending: capacity,
            max_queue_bytes: 1024 * 1024,
            max_result_bytes: 1024 * 1024,
            ack_wait: Duration::from_millis(300),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}
fn namespace() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn reconnect_redelivery_idempotency_and_conflict() {
    let namespace = namespace();
    let queue = open(&namespace, 10).await;
    let task = Task::new("one", "worker", json!({"job":1}));
    queue.publish(task.clone()).await.unwrap();
    let worker = queue.worker("worker").await.unwrap();
    let delivery = worker.next(Duration::from_secs(2)).await.unwrap().unwrap();
    assert_eq!(delivery.task.id, "one");
    drop(delivery);
    drop(worker);
    drop(queue);
    let queue = open(&namespace, 10).await;
    let worker = queue.worker("worker").await.unwrap();
    let delivery = worker.next(Duration::from_secs(2)).await.unwrap().unwrap();
    delivery.complete(json!({"answer":42})).await.unwrap();
    queue.publish(task).await.unwrap();
    assert!(worker
        .next(Duration::from_millis(400))
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        queue.outcome("one").await.unwrap(),
        Some(TaskOutcome::Completed(json!({"answer":42})))
    );
    assert!(queue
        .publish(Task::new("one", "worker", json!({"job":2})))
        .await
        .is_err());
}
#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn dependencies_retry_and_failed_dependency() {
    let queue = open(&namespace(), 10).await;
    queue
        .publish(Task::new("child", "worker", json!(2)).with_depends_on(vec!["parent".into()]))
        .await
        .unwrap();
    let worker = queue.worker("worker").await.unwrap();
    assert!(worker
        .next(Duration::from_millis(200))
        .await
        .unwrap()
        .is_none());
    queue
        .publish(Task::new("parent", "worker", json!(1)))
        .await
        .unwrap();
    let parent = worker.next(Duration::from_secs(2)).await.unwrap().unwrap();
    assert_eq!(parent.task.id, "parent");
    parent.retry(Duration::from_millis(100)).await.unwrap();
    let parent = worker.next(Duration::from_secs(2)).await.unwrap().unwrap();
    assert_eq!(parent.task.id, "parent");
    parent.fail("permanent").await.unwrap();
    assert!(worker
        .next(Duration::from_millis(600))
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        queue.outcome("child").await.unwrap(),
        Some(TaskOutcome::Failed(_))
    ));
}
#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn competing_workers_do_not_duplicate_and_capacity_recovers() {
    let namespace = namespace();
    let queue = open(&namespace, 2).await;
    queue
        .publish(Task::new("one", "worker", json!(1)))
        .await
        .unwrap();
    queue
        .publish(Task::new("two", "worker", json!(2)))
        .await
        .unwrap();
    assert!(queue
        .publish(Task::new("three", "worker", json!(3)))
        .await
        .is_err());
    let other_queue = open(&namespace, 2).await;
    let one = queue.worker("worker").await.unwrap();
    let two = other_queue.worker("worker").await.unwrap();
    let (a, b) = tokio::join!(
        one.next(Duration::from_secs(2)),
        two.next(Duration::from_secs(2))
    );
    let a = a.unwrap().unwrap();
    let b = b.unwrap().unwrap();
    assert_ne!(a.task.id, b.task.id);
    a.complete(json!(true)).await.unwrap();
    b.complete(json!(true)).await.unwrap();
    queue
        .publish(Task::new("three", "worker", json!(3)))
        .await
        .unwrap();
    let delivery = one.next(Duration::from_secs(2)).await.unwrap().unwrap();
    assert_eq!(delivery.task.id, "three");
    delivery.complete(json!(true)).await.unwrap();
}

async fn connect() -> Arc<NatsClient> {
    Arc::new(
        NatsClient::connect(NatsConfig::new(
            std::env::var("DASEIN_TEST_NATS_URL")
                .unwrap_or_else(|_| "nats://127.0.0.1:4222".into()),
        ))
        .await
        .unwrap(),
    )
}

#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn many_waits_share_one_subscription() {
    let namespace = namespace();
    let queue = open(&namespace, 1000).await;
    let worker = queue.worker("worker").await.unwrap();
    let mut waits = Vec::new();
    for n in 0..60 {
        let id = format!("task-{n}");
        queue
            .publish(Task::new(id.clone(), "worker", json!(n)))
            .await
            .unwrap();
        let queue = queue.clone();
        waits.push(tokio::spawn(async move {
            queue.wait_outcome(&id, Duration::from_secs(20)).await
        }));
    }
    // One subscription serves them all, however many there are.
    let nats = connect().await;
    let stream = nats
        .jetstream()
        .unwrap()
        .get_stream(format!("KV_DASEIN_RESULTS_{namespace}"))
        .await
        .unwrap();
    let mut consumers = 0;
    for _ in 0..50 {
        consumers = stream.clone().info().await.unwrap().state.consumer_count;
        assert!(consumers <= 1, "{consumers} subscriptions for the waits");
        if consumers == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(consumers, 1, "no subscription was made");

    for _ in 0..60 {
        let delivery = worker.next(Duration::from_secs(5)).await.unwrap().unwrap();
        let answer = delivery.task.payload.clone();
        delivery.complete(answer).await.unwrap();
    }
    for (n, wait) in waits.into_iter().enumerate() {
        assert_eq!(
            wait.await.unwrap().unwrap(),
            Some(TaskOutcome::Completed(json!(n)))
        );
    }
    // A wait for a task that is already done answers at once.
    let started = std::time::Instant::now();
    assert_eq!(
        queue
            .wait_outcome("task-3", Duration::from_secs(20))
            .await
            .unwrap(),
        Some(TaskOutcome::Completed(json!(3)))
    );
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn a_task_whose_record_expired_runs_again() {
    let queue = DurableQueue::new(
        connect().await,
        &namespace(),
        DurableQueueConfig {
            ack_wait: Duration::from_millis(300),
            result_ttl: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let worker = queue.worker("worker").await.unwrap();
    let task = Task::new("one", "worker", json!(1));
    queue.publish(task.clone()).await.unwrap();
    let delivery = worker.next(Duration::from_secs(2)).await.unwrap().unwrap();
    delivery.complete(json!("first")).await.unwrap();

    // The ledger does not grow for ever: the record goes after its lifetime.
    let mut gone = false;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if queue.outcome("one").await.unwrap().is_none() {
            gone = true;
            break;
        }
    }
    assert!(gone, "the record did not expire");

    // And the forgotten task is accepted and run like a new one.
    queue.publish(task).await.unwrap();
    let delivery = worker.next(Duration::from_secs(5)).await.unwrap().unwrap();
    delivery.complete(json!("second")).await.unwrap();
    assert_eq!(
        queue.outcome("one").await.unwrap(),
        Some(TaskOutcome::Completed(json!("second")))
    );
}

#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn a_full_ledger_refuses_a_task_with_a_reason() {
    let queue = open(&namespace(), 10).await;
    let large = "x".repeat(400 * 1024);
    for id in ["a", "b"] {
        queue
            .publish(Task::new(id, "worker", json!(large)))
            .await
            .unwrap();
    }
    let refusal = queue
        .publish(Task::new("c", "worker", json!(large)))
        .await
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("Could not record the task"), "{refusal}");
    assert!(refusal.contains("ledger"), "{refusal}");
}

#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn a_ledger_without_expiry_is_given_one() {
    use async_nats::jetstream::{kv, stream};
    let namespace = namespace();
    let nats = connect().await;
    // What an earlier version created: records kept for ever.
    nats.jetstream()
        .unwrap()
        .create_key_value(kv::Config {
            bucket: format!("DASEIN_RESULTS_{namespace}"),
            history: 1,
            max_bytes: 1024 * 1024,
            storage: stream::StorageType::File,
            ..Default::default()
        })
        .await
        .unwrap();
    open(&namespace, 10).await;
    let config = nats
        .jetstream()
        .unwrap()
        .get_stream(format!("KV_DASEIN_RESULTS_{namespace}"))
        .await
        .unwrap()
        .info()
        .await
        .unwrap()
        .config
        .clone();
    assert_eq!(config.max_age, DurableQueueConfig::default().result_ttl);
}

#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn a_task_that_outlives_its_record_still_completes() {
    let queue = DurableQueue::new(
        connect().await,
        &namespace(),
        DurableQueueConfig {
            ack_wait: Duration::from_secs(30),
            result_ttl: Duration::from_secs(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let worker = queue.worker("worker").await.unwrap();
    queue
        .publish(Task::new("slow", "worker", json!(1)))
        .await
        .unwrap();
    let delivery = worker.next(Duration::from_secs(2)).await.unwrap().unwrap();
    // The work takes longer than the record is kept.
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if queue.outcome("slow").await.unwrap().is_none() {
            break;
        }
    }
    assert!(queue.outcome("slow").await.unwrap().is_none());
    delivery.complete(json!("done")).await.unwrap();
    assert_eq!(
        queue.outcome("slow").await.unwrap(),
        Some(TaskOutcome::Completed(json!("done")))
    );
}

#[tokio::test]
#[ignore = "Requires disposable NATS JetStream"]
async fn a_process_that_disagrees_leaves_the_queue_as_it_was() {
    let namespace = namespace();
    let nats = connect().await;
    let config = |result_ttl| DurableQueueConfig {
        result_ttl,
        ..Default::default()
    };
    let hour = Duration::from_secs(60 * 60);
    DurableQueue::new(nats.clone(), &namespace, config(hour))
        .await
        .unwrap();
    assert!(
        DurableQueue::new(nats.clone(), &namespace, config(hour * 5))
            .await
            .is_err()
    );
    let window = nats
        .jetstream()
        .unwrap()
        .get_stream(format!("DASEIN_TASKS_{namespace}"))
        .await
        .unwrap()
        .cached_info()
        .config
        .duplicate_window;
    assert_eq!(window, hour);
}
