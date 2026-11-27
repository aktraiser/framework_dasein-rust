//! What a run reports to a listener: the engine's steps, what executors
//! add, and the model and tool calls of an agent, locally and from a worker.
use async_trait::async_trait;
use dasein_agentic_core::distributed::graph::agent::{Agent, AgentThread, ChatAgent, ChatMessage};
use dasein_agentic_core::distributed::graph::*;
use dasein_agentic_llm::{
    FinishReason, LLMAdapter, LLMError, LLMMessage, LLMResponse, Role, StreamChunk, TokenUsage,
    Tool, ToolCall,
};
use futures::StreamExt;
use serde_json::json;
use std::pin::Pin;
use std::time::Duration;

/// Asks for the `lookup` tool once, then answers with what it returned.
struct Scripted;

#[async_trait]
impl LLMAdapter for Scripted {
    fn provider(&self) -> &str {
        "scripted"
    }
    fn model(&self) -> &str {
        "script-1"
    }
    fn supports_tools(&self) -> bool {
        true
    }
    async fn generate(&self, _: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
        Err(LLMError::ApiError("no tools were offered".into()))
    }
    async fn generate_with_tools(
        &self,
        messages: &[LLMMessage],
        _: &[Tool],
    ) -> Result<LLMResponse, LLMError> {
        let last = messages.last().unwrap();
        let (content, tool_calls) = if last.role == Role::Tool {
            (format!("It says {}", last.content), vec![])
        } else {
            (
                String::new(),
                vec![ToolCall::new("call_1", "lookup", json!({"key": "door"}))],
            )
        };
        Ok(LLMResponse {
            content,
            tokens_used: TokenUsage {
                prompt: 7,
                completion: 3,
                total: 10,
                details: None,
            },
            finish_reason: FinishReason::Stop,
            model: "script-1".into(),
            tool_calls,
        })
    }
    fn generate_stream(
        &self,
        _: &[LLMMessage],
    ) -> Pin<Box<dyn futures::Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>> {
        Box::pin(futures::stream::empty())
    }
    async fn health_check(&self) -> Result<bool, LLMError> {
        Ok(true)
    }
}

fn agent() -> ChatAgent<Scripted> {
    ChatAgent::new("clerk", Scripted).with_tool_handler(
        dasein_agentic_core::distributed::graph::agent::Tool::new("lookup", "Look a key up"),
        |arguments| async move {
            if arguments["key"] == "door" {
                Ok(json!("HIBOU-7302"))
            } else {
                Ok(json!(null))
            }
        },
    )
}

/// A step that runs the agent, reports a note of its own and yields the
/// agent's answer. `"fail"` makes it fail instead.
struct Ask;

#[async_trait]
impl Executor for Ask {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        static ID: std::sync::OnceLock<ExecutorId> = std::sync::OnceLock::new();
        ID.get_or_init(|| "ask".into())
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, input: String, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        ctx.add_event("note", json!({"asked": input})).await?;
        if input == "fail" {
            return Err(ExecutorError::internal("refused on purpose"));
        }
        let answer = agent()
            .run(vec![ChatMessage::user(input)], &mut AgentThread::new())
            .await
            .map_err(|e| ExecutorError::internal(e.to_string()))?;
        ctx.yield_output(answer.content).await
    }
}

/// The first, local step of the remote workflow.
struct Pass;

#[async_trait]
impl Executor for Pass {
    type Input = String;
    type Message = String;
    type Output = String;
    fn id(&self) -> &ExecutorId {
        static ID: std::sync::OnceLock<ExecutorId> = std::sync::OnceLock::new();
        ID.get_or_init(|| "pass".into())
    }
    fn kind(&self) -> ExecutorKind {
        ExecutorKind::Worker
    }
    async fn handle<C>(&self, input: String, ctx: &mut C) -> Result<(), ExecutorError>
    where
        C: ExecutorContext<String, String> + Send,
    {
        ctx.send_message(input).await
    }
}

fn local_workflow() -> Workflow<String, String> {
    let definition = WorkflowBuilder::<String>::new("asking")
        .set_start("ask")
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    registry.register(Ask);
    Workflow::with_config(
        definition,
        registry,
        WorkflowConfig::new().with_max_retries(0),
    )
}

/// One short word per event, for comparing sequences.
fn kinds(events: &[WorkflowStreamEvent]) -> Vec<&'static str> {
    events
        .iter()
        .map(|event| match event {
            WorkflowStreamEvent::Started { .. } => "started",
            WorkflowStreamEvent::SuperstepStarted { .. } => "superstep",
            WorkflowStreamEvent::ExecutorStarted { .. } => "executor",
            WorkflowStreamEvent::ExecutorCompleted { .. } => "executor done",
            WorkflowStreamEvent::ExecutorFailed { .. } => "executor failed",
            WorkflowStreamEvent::SuperstepCompleted { .. } => "superstep done",
            WorkflowStreamEvent::CheckpointCreated { .. } => "checkpoint",
            WorkflowStreamEvent::Output { .. } => "output",
            WorkflowStreamEvent::CustomEvent { .. } => "custom",
            WorkflowStreamEvent::ModelCalled { .. } => "model",
            WorkflowStreamEvent::ToolCalled { .. } => "tool",
            WorkflowStreamEvent::Completed { .. } => "completed",
            WorkflowStreamEvent::Failed { .. } => "failed",
        })
        .collect()
}

#[tokio::test]
async fn a_run_reports_its_steps_and_the_calls_of_its_agent_in_order() {
    let workflow = local_workflow();
    let events: Vec<_> = workflow
        .run_stream("the door code".to_string())
        .collect()
        .await;
    assert_eq!(
        kinds(&events),
        [
            "started",
            "superstep",
            "executor",
            "model",
            "tool",
            "model",
            "custom",
            "output",
            "executor done",
            "superstep done",
            "completed",
        ]
    );
    // The calls say what a reader of the run wants to know.
    let WorkflowStreamEvent::ModelCalled {
        executor_id,
        input_index,
        agent,
        provider,
        model,
        prompt_tokens,
        completion_tokens,
        tool_calls,
        error,
        ..
    } = &events[3]
    else {
        unreachable!()
    };
    assert_eq!(
        (
            provider.as_str(),
            model.as_str(),
            *prompt_tokens,
            *completion_tokens,
            *tool_calls
        ),
        ("scripted", "script-1", 7, 3, 1)
    );
    assert!(error.is_none());
    // Which step and which agent the call belongs to.
    assert_eq!(
        (
            executor_id.as_ref().map(ExecutorId::as_str),
            *input_index,
            agent.as_str()
        ),
        (Some("ask"), Some(0), "clerk")
    );
    let WorkflowStreamEvent::ToolCalled {
        executor_id: Some(tool_step),
        tool,
        arguments,
        result,
        success,
        ..
    } = &events[4]
    else {
        unreachable!()
    };
    assert_eq!(
        (tool.as_str(), arguments, result.as_str(), *success),
        ("lookup", &json!({"key": "door"}), "HIBOU-7302", true)
    );
    assert_eq!(tool_step.as_str(), "ask");
    let WorkflowStreamEvent::CustomEvent {
        executor_id,
        event_type,
        data,
    } = &events[6]
    else {
        unreachable!()
    };
    assert_eq!(
        (executor_id.as_str(), event_type.as_str(), data),
        ("ask", "note", &json!({"asked": "the door code"}))
    );
    let WorkflowStreamEvent::Output { data, .. } = &events[7] else {
        unreachable!()
    };
    assert_eq!(data, &json!("It says \"HIBOU-7302\""));
    assert!(matches!(
        events.last(),
        Some(WorkflowStreamEvent::Completed {
            output_count: 1,
            ..
        })
    ));
}

#[tokio::test]
async fn a_failed_run_reports_what_failed_and_ends_with_the_failure() {
    let events: Vec<_> = local_workflow()
        .run_stream("fail".to_string())
        .collect()
        .await;
    // What the executor reported before failing is not lost.
    assert_eq!(
        kinds(&events),
        [
            "started",
            "superstep",
            "executor",
            "custom",
            "executor failed",
            "failed"
        ]
    );
    let WorkflowStreamEvent::ExecutorFailed { error, .. } = &events[4] else {
        unreachable!()
    };
    assert!(error.contains("refused on purpose"), "{error}");
}

#[tokio::test]
async fn the_result_is_still_returned_when_someone_listens_and_nothing_is_needed_when_nobody_does()
{
    let workflow = local_workflow();
    // Nobody listens: the run is as before.
    let result = workflow.run("the door code".to_string()).await.unwrap();
    assert_eq!(result.outputs, ["It says \"HIBOU-7302\""]);

    // Result and events together.
    let (listener, mut events) = tokio::sync::mpsc::unbounded_channel();
    let result = with_events(listener, workflow.run("the door code".to_string()))
        .await
        .unwrap();
    assert!(result.success);
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    assert_eq!(kinds(&seen).first(), Some(&"started"));
    assert_eq!(kinds(&seen).last(), Some(&"completed"));

    // A listener that went away does not disturb the run.
    let (listener, events) = tokio::sync::mpsc::unbounded_channel();
    drop(events);
    assert!(
        with_events(listener, workflow.run("the door code".to_string()))
            .await
            .unwrap()
            .success
    );
}

#[tokio::test]
async fn an_agent_outside_a_workflow_can_be_listened_to() {
    let (listener, mut events) = tokio::sync::mpsc::unbounded_channel();
    let answer = with_events(listener, async {
        agent()
            .run(vec![ChatMessage::user("code?")], &mut AgentThread::new())
            .await
    })
    .await
    .unwrap();
    assert_eq!(answer.content, "It says \"HIBOU-7302\"");
    let mut seen = Vec::new();
    while let Ok(event) = events.try_recv() {
        seen.push(event);
    }
    assert_eq!(kinds(&seen), ["model", "tool", "model"]);
    // No workflow, no step.
    assert!(matches!(
        &seen[0],
        WorkflowStreamEvent::ModelCalled {
            executor_id: None,
            input_index: None,
            ..
        }
    ));
}

#[tokio::test]
async fn what_a_remote_step_reports_comes_back_with_its_result() {
    let queue = InMemoryTaskQueue::new();
    let mut hosted = ExecutorRegistry::new();
    hosted.register(Ask);
    let worker = RemoteWorker::<String, String>::new(hosted).with_event_reporting(true);
    let source = queue.worker("clerks");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(async move {
        worker
            .serve(&source, async {
                let _ = stopped.await;
            })
            .await;
    });

    let definition = WorkflowBuilder::<String>::new("asking-remotely")
        .set_start("pass")
        .add_edge(Edge::direct("pass", "ask"))
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    registry.register(Pass);
    registry.register(
        RemoteExecutor::<String, String, String, _>::new("ask", queue, "clerks")
            .with_wait(Duration::from_secs(10)),
    );
    let workflow = Workflow::new(definition, registry);

    let events: Vec<_> = workflow
        .run_stream("the door code".to_string())
        .collect()
        .await;
    drop(stop);
    serving.await.unwrap();
    // The worker's model and tool calls and its note, between the start and
    // the end of the remote step.
    assert_eq!(
        kinds(&events),
        [
            "started",
            "superstep",
            "executor",
            "executor done",
            "superstep done",
            "superstep",
            "executor",
            "model",
            "tool",
            "model",
            "custom",
            "output",
            "executor done",
            "superstep done",
            "completed",
        ]
    );
    let WorkflowStreamEvent::ToolCalled {
        result,
        executor_id,
        ..
    } = &events[8]
    else {
        unreachable!()
    };
    assert_eq!(result, "HIBOU-7302");
    assert_eq!(executor_id.as_ref().map(ExecutorId::as_str), Some("ask"));
}

/// A queue, a worker hosting `Ask` and the workflow pass -> ask (remote).
fn remote_setup(
    worker: impl FnOnce(RemoteWorker<String, String>) -> RemoteWorker<String, String>,
) -> (
    Workflow<String, String>,
    impl FnOnce() -> tokio::task::JoinHandle<()>,
) {
    let queue = InMemoryTaskQueue::new();
    let mut hosted = ExecutorRegistry::new();
    hosted.register(Ask);
    let worker = worker(RemoteWorker::<String, String>::new(hosted));
    let source = queue.worker("clerks");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(async move {
        worker
            .serve(&source, async {
                let _ = stopped.await;
            })
            .await;
    });
    let definition = WorkflowBuilder::<String>::new("asking-remotely")
        .set_start("pass")
        .add_edge(Edge::direct("pass", "ask"))
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    registry.register(Pass);
    registry.register(
        RemoteExecutor::<String, String, String, _>::new("ask", queue, "clerks")
            .with_wait(Duration::from_secs(10)),
    );
    let workflow = Workflow::with_config(
        definition,
        registry,
        WorkflowConfig::new().with_max_retries(0),
    );
    (workflow, move || {
        drop(stop);
        serving
    })
}

#[tokio::test]
async fn a_worker_reports_events_only_when_asked_and_never_at_the_cost_of_the_result() {
    // By default nothing of the worker's is sent back.
    let (workflow, stop) = remote_setup(|worker| worker);
    let events: Vec<_> = workflow
        .run_stream("the door code".to_string())
        .collect()
        .await;
    stop().await.unwrap();
    assert!(!kinds(&events).contains(&"model") && !kinds(&events).contains(&"custom"));
    assert_eq!(kinds(&events).last(), Some(&"completed"));

    // Asked to, with room for the result and little more: the result comes
    // back whole, with the events that fit, rather than failing as too
    // large. The note the step adds repeats its long input and does not fit.
    let (workflow, stop) = remote_setup(|worker| {
        worker
            .with_event_reporting(true)
            .with_max_result_bytes(5500)
    });
    let long_question = "door ".repeat(600);
    let (listener, mut reported) = tokio::sync::mpsc::unbounded_channel();
    let result = with_events(listener, workflow.run(long_question))
        .await
        .unwrap();
    stop().await.unwrap();
    assert!(result.success, "{:?}", result.error);
    assert_eq!(result.outputs, ["It says \"HIBOU-7302\""]);
    let mut seen = Vec::new();
    while let Ok(event) = reported.try_recv() {
        seen.push(event);
    }
    assert!(kinds(&seen).contains(&"tool"), "{:?}", kinds(&seen));
    assert!(!kinds(&seen).contains(&"custom"), "{:?}", kinds(&seen));
}

#[tokio::test]
async fn a_failure_says_whether_it_is_tried_again_and_a_checkpoint_is_reported() {
    /// Fails with an error that may be retried, every time.
    struct Flaky;
    #[async_trait]
    impl Executor for Flaky {
        type Input = String;
        type Message = String;
        type Output = String;
        fn id(&self) -> &ExecutorId {
            static ID: std::sync::OnceLock<ExecutorId> = std::sync::OnceLock::new();
            ID.get_or_init(|| "flaky".into())
        }
        fn kind(&self) -> ExecutorKind {
            ExecutorKind::Worker
        }
        async fn handle<C>(&self, _: String, _: &mut C) -> Result<(), ExecutorError>
        where
            C: ExecutorContext<String, String> + Send,
        {
            Err(ExecutorError::new("flaky", "try again"))
        }
    }
    let definition = WorkflowBuilder::<String>::new("flaky")
        .set_start("flaky")
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    registry.register(Flaky);
    let workflow = Workflow::with_config(
        definition,
        registry,
        WorkflowConfig::new()
            .with_max_retries(2)
            .with_retry_backoff(1, 2),
    );
    let events: Vec<_> = workflow.run_stream("x".to_string()).collect().await;
    let retried: Vec<bool> = events
        .iter()
        .filter_map(|event| match event {
            WorkflowStreamEvent::ExecutorFailed { will_retry, .. } => Some(*will_retry),
            _ => None,
        })
        .collect();
    // Two retries, then the failure that ends it.
    assert_eq!(retried, [true, true, false]);
    assert!(matches!(
        events.last(),
        Some(WorkflowStreamEvent::Failed { workflow_id, .. }) if workflow_id.as_str() == "flaky"
    ));

    // A run that saves checkpoints says so.
    let definition = WorkflowBuilder::<String>::new("asking")
        .set_start("ask")
        .build()
        .unwrap();
    let mut registry = ExecutorRegistry::new();
    registry.register(Ask);
    let workflow = Workflow::with_config(
        definition,
        registry,
        WorkflowConfig::new()
            .with_checkpointing(true)
            .with_checkpoint_interval(1),
    );
    let events: Vec<_> = workflow
        .run_stream("the door code".to_string())
        .collect()
        .await;
    assert!(
        kinds(&events).contains(&"checkpoint"),
        "{:?}",
        kinds(&events)
    );
    assert!(matches!(
        events.last(),
        Some(WorkflowStreamEvent::Completed { workflow_id, .. }) if workflow_id.as_str() == "asking"
    ));
}
