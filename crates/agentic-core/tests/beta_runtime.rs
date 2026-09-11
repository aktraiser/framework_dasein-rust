use async_trait::async_trait;
use dasein_agentic_core::{distributed::graph::*, patterns::*};
use std::sync::Arc;

struct Speaker {
    id: ExecutorId,
}
#[async_trait]
impl Executor for Speaker {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        &self.id
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, input: String, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        let history: GroupChatState = ctx
            .get_shared_state("group_chat")
            .await?
            .expect("shared history");
        assert_eq!(history.conversation.len(), history.current_round + 1);
        assert_eq!(history.last_content(), Some(input.as_str()));
        ctx.yield_output(format!("{input}/{}", self.id)).await
    }
}
fn registry() -> ExecutorRegistry<String, String> {
    let mut registry = ExecutorRegistry::new();
    for id in ["a", "b"] {
        registry.register(Speaker { id: id.into() });
    }
    registry
}
fn definition() -> WorkflowDefinition<String> {
    GroupChatBuilder::<String>::new("group")
        .participants(vec!["a", "b"])
        .with_orchestrator_func(round_robin_selector)
        .with_max_iterations(4)
        .build()
        .unwrap()
}
#[tokio::test]
async fn group_chat_alternates_and_restores_completed_checkpoint() {
    let checkpoints = Arc::new(InMemoryPersistentBackend::new());
    let flow = Workflow::new(definition(), registry()).with_persistent_backend(checkpoints.clone());
    let task = TaskId::new("chat");
    let result = flow
        .run_with_resume(task.clone(), None, "hello")
        .await
        .unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(
        result.outputs,
        vec!["hello/a", "hello/a/b", "hello/a/b/a", "hello/a/b/a/b"]
    );
    let saved = checkpoints
        .load_persistent(flow.id(), &task)
        .await
        .unwrap()
        .unwrap();
    let resumed = Workflow::new(definition(), registry())
        .with_persistent_backend(checkpoints)
        .run_with_resume(task, None, "do not use")
        .await
        .unwrap();
    assert!(resumed.success);
    assert_eq!(resumed.outputs, result.outputs);
    assert!(!saved.shared_state.is_empty());
}
#[tokio::test]
async fn group_chat_termination_and_selector_validation() {
    let definition = GroupChatBuilder::<String>::new("stop")
        .participants(vec!["a", "b"])
        .with_orchestrator_func(round_robin_selector)
        .with_termination_condition(|state| state.current_round == 2)
        .build()
        .unwrap();
    let result = Workflow::new(definition, registry())
        .run("hello")
        .await
        .unwrap();
    assert!(result.success);
    assert_eq!(result.outputs.len(), 2);
    let definition = GroupChatBuilder::<String>::new("invalid")
        .participants(vec!["a"])
        .with_orchestrator_func(|_| "outsider".into())
        .build()
        .unwrap();
    let result = Workflow::new(definition, registry()).run("hello").await;
    assert!(result.is_err() || !result.unwrap().success);
}
#[tokio::test]
async fn task_state_isolation_survives_concurrent_chats() {
    let state = Arc::new(InMemoryStateBackend::new());
    let flow = Workflow::new(definition(), registry()).with_state_backend(state);
    let (one, two) = tokio::join!(
        flow.run_with_resume(TaskId::new("one"), None, "one"),
        flow.run_with_resume(TaskId::new("two"), None, "two")
    );
    assert_eq!(
        one.unwrap().outputs.last(),
        Some(&"one/a/b/a/b".to_string())
    );
    assert_eq!(
        two.unwrap().outputs.last(),
        Some(&"two/a/b/a/b".to_string())
    );
}
#[test]
fn group_chat_rejects_invalid_policies() {
    assert!(GroupChatBuilder::<String>::new("zero")
        .participants(vec!["a"])
        .with_orchestrator_func(round_robin_selector)
        .with_max_iterations(0)
        .build()
        .is_err());
    assert!(GroupChatBuilder::<String>::new("duplicate")
        .participants(vec!["a", "a"])
        .with_orchestrator_func(round_robin_selector)
        .build()
        .is_err());
}

#[tokio::test]
async fn group_chat_resumes_mid_conversation() {
    let checkpoints = Arc::new(InMemoryPersistentBackend::new());
    let task = TaskId::new("interrupted");
    let flow = Workflow::with_config(
        definition(),
        registry(),
        WorkflowConfig::new()
            .with_max_supersteps(2)
            .with_checkpoint_interval(1),
    )
    .with_persistent_backend(checkpoints.clone());
    let first = flow.run_with_resume(task.clone(), None, "hello").await;
    assert!(first.is_err() || !first.unwrap().success);
    let flow = Workflow::new(definition(), registry()).with_persistent_backend(checkpoints);
    let result = flow.run_with_resume(task, None, "unused").await.unwrap();
    assert!(result.success);
    assert_eq!(
        result.outputs,
        vec!["hello/a", "hello/a/b", "hello/a/b/a", "hello/a/b/a/b"]
    );
}
