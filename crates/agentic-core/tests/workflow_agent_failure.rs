//! A failed workflow must surface as an agent error, not as a normal reply.
use async_trait::async_trait;
use dasein_agentic_core::distributed::graph::agent::{
    Agent, AgentThread, ChatMessage, WorkflowAgent,
};
use dasein_agentic_core::distributed::graph::*;

struct AlwaysFails(ExecutorId);

#[async_trait]
impl Executor for AlwaysFails {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        &self.0
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, _: String, _: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        Err(ExecutorError::internal("disk full"))
    }
}

#[tokio::test]
async fn failed_workflow_is_an_agent_error() {
    let definition = WorkflowBuilder::<String>::new("failing")
        .set_start("only")
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    registry.register(AlwaysFails("only".into()));
    let agent = WorkflowAgent::new("wf", Workflow::new(definition, registry));

    let mut thread = AgentThread::new();
    let result = agent.run(vec![ChatMessage::user("go")], &mut thread).await;

    let error = result.expect_err("a failed workflow reported success");
    assert!(error.to_string().contains("disk full"), "{error}");
}
