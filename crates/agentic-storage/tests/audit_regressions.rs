use dasein_agentic_storage::{RedisStore, StateStore};
#[tokio::test]
#[ignore = "Requires disposable NATS/Redis; see scripts/test-services.sh"]
async fn audit_redis_subsecond_ttl() {
    let store = RedisStore::connect(
        &std::env::var("DASEIN_TEST_REDIS_URL")
            .unwrap_or_else(|_| "redis://127.0.0.1:6379/".into()),
    )
    .await
    .unwrap();
    let result = store.set("audit:ttl:500", &"value", Some(500)).await;
    assert!(result.is_ok(), "500ms TTL rejected: {result:?}");
    assert_eq!(
        store.get::<String>("audit:ttl:500").await.unwrap(),
        Some("value".into())
    );
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    assert_eq!(store.get::<String>("audit:ttl:500").await.unwrap(), None);
}
