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
