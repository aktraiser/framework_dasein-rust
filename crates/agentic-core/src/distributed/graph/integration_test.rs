//! Integration tests for Executor + WorkflowContext.
//!
//! These tests verify that the graph components work together correctly.

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use crate::distributed::graph::{
        Executor, ExecutorContext, ExecutorError, ExecutorId, ExecutorKind, TaskId,
        ValidationResult, WorkflowContext, WorkflowId,
    };

    // ========================================================================
    // TEST EXECUTOR: Simple string transformer
    // ========================================================================

    struct UppercaseExecutor {
        id: ExecutorId,
    }

    impl UppercaseExecutor {
        fn new(id: &str) -> Self {
            Self {
                id: ExecutorId::new(id),
            }
        }
    }

    #[async_trait]
    impl Executor for UppercaseExecutor {
        type Input = String;
        type Message = String;
        type Output = String;

        fn id(&self) -> &ExecutorId {
            &self.id
        }

        fn kind(&self) -> ExecutorKind {
            ExecutorKind::Worker
        }

        async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
        where
            Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
        {
            // Transform input
            let result = input.to_uppercase();

            // Send message to next executor in chain
            ctx.send_message(result.clone()).await?;

            // Yield output visible to caller
            ctx.yield_output(result).await?;

            // Log what we did
            ctx.add_event(
                "transform",
                serde_json::json!({
                    "input": input,
                    "operation": "uppercase"
                }),
            )
            .await?;

            Ok(())
        }

        fn name(&self) -> &str {
            "UppercaseExecutor"
        }

        fn description(&self) -> Option<&str> {
            Some("Transforms input to uppercase")
        }
    }

    // ========================================================================
    // TEST EXECUTOR: Validator that checks string length
    // ========================================================================

    struct LengthValidator {
        id: ExecutorId,
        min_length: usize,
        max_length: usize,
    }

    impl LengthValidator {
        fn new(id: &str, min: usize, max: usize) -> Self {
            Self {
                id: ExecutorId::new(id),
                min_length: min,
                max_length: max,
            }
        }
    }

    #[async_trait]
    impl Executor for LengthValidator {
        type Input = String;
        type Message = ValidationResult;
        type Output = ();

        fn id(&self) -> &ExecutorId {
            &self.id
        }

        fn kind(&self) -> ExecutorKind {
            ExecutorKind::Validator
        }

        async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
        where
            Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
        {
            let len = input.len();
            let mut errors = Vec::new();

            if len < self.min_length {
                errors.push(format!(
                    "String too short: {} < {} (min)",
                    len, self.min_length
                ));
            }
            if len > self.max_length {
                errors.push(format!(
                    "String too long: {} > {} (max)",
                    len, self.max_length
                ));
            }

            let result = if errors.is_empty() {
                ValidationResult::success()
            } else {
                ValidationResult::failure(errors).with_feedback(format!(
                    "String length must be between {} and {}",
                    self.min_length, self.max_length
                ))
            };

            // Send validation result to next executor
            ctx.send_message(result).await?;

            Ok(())
        }
    }

    // ========================================================================
    // TEST EXECUTOR: Orchestrator that coordinates sub-tasks
    // ========================================================================

    struct SplitOrchestrator {
        id: ExecutorId,
    }

    impl SplitOrchestrator {
        fn new(id: &str) -> Self {
            Self {
                id: ExecutorId::new(id),
            }
        }
    }

    #[async_trait]
    impl Executor for SplitOrchestrator {
        type Input = String;
        type Message = String; // Each word becomes a message
        type Output = usize; // Output is word count

        fn id(&self) -> &ExecutorId {
            &self.id
        }

        fn kind(&self) -> ExecutorKind {
            ExecutorKind::Orchestrator
        }

        async fn handle<Ctx>(&self, input: Self::Input, ctx: &mut Ctx) -> Result<(), ExecutorError>
        where
            Ctx: ExecutorContext<Self::Message, Self::Output> + Send,
        {
            let words: Vec<&str> = input.split_whitespace().collect();
            let count = words.len();

            // Send each word as a separate message (fan-out pattern)
            for word in words {
                ctx.send_message(word.to_string()).await?;
            }

            // Yield the count as output
            ctx.yield_output(count).await?;

            Ok(())
        }
    }

    // ========================================================================
    // INTEGRATION TESTS
    // ========================================================================

    #[tokio::test]
    async fn test_worker_executor_with_context() {
        let executor = UppercaseExecutor::new("upper-1");
        let mut ctx: WorkflowContext<String, String> = WorkflowContext::in_memory(
            WorkflowId::new("wf-test"),
            TaskId::new("task-1"),
            executor.id().clone(),
        );

        // Execute
        let result = executor.handle("hello world".to_string(), &mut ctx).await;
        assert!(result.is_ok());

        // Check messages
        let messages = ctx.drain_messages();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], "HELLO WORLD");

        // Check outputs
        let outputs = ctx.drain_outputs();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0], "HELLO WORLD");

        // Check events
        let events = ctx.drain_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "transform");
    }

    #[tokio::test]
    async fn test_validator_executor_success() {
        let validator = LengthValidator::new("len-val", 3, 20);
        let mut ctx: WorkflowContext<ValidationResult, ()> = WorkflowContext::in_memory(
            WorkflowId::new("wf-test"),
            TaskId::new("task-1"),
            validator.id().clone(),
        );

        // Valid input
        let result = validator.handle("hello".to_string(), &mut ctx).await;
        assert!(result.is_ok());

        let messages = ctx.drain_messages();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].passed);
        assert!(messages[0].errors.is_empty());
    }

    #[tokio::test]
    async fn test_validator_executor_failure() {
        let validator = LengthValidator::new("len-val", 10, 20);
        let mut ctx: WorkflowContext<ValidationResult, ()> = WorkflowContext::in_memory(
            WorkflowId::new("wf-test"),
            TaskId::new("task-1"),
            validator.id().clone(),
        );

        // Invalid input (too short)
        let result = validator.handle("hi".to_string(), &mut ctx).await;
        assert!(result.is_ok());

        let messages = ctx.drain_messages();
        assert_eq!(messages.len(), 1);
        assert!(!messages[0].passed);
        assert!(!messages[0].errors.is_empty());
        assert!(messages[0].feedback.is_some());
    }

    #[tokio::test]
    async fn test_orchestrator_fan_out() {
        let orchestrator = SplitOrchestrator::new("splitter");
        let mut ctx: WorkflowContext<String, usize> = WorkflowContext::in_memory(
            WorkflowId::new("wf-test"),
            TaskId::new("task-1"),
            orchestrator.id().clone(),
        );

        // Execute with multi-word input
        let result = orchestrator
            .handle("hello world from rust".to_string(), &mut ctx)
            .await;
        assert!(result.is_ok());

        // Check fan-out messages
        let messages = ctx.drain_messages();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0], "hello");
        assert_eq!(messages[1], "world");
        assert_eq!(messages[2], "from");
        assert_eq!(messages[3], "rust");

        // Check output (word count)
        let outputs = ctx.drain_outputs();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0], 4);
    }

    #[tokio::test]
    async fn test_shared_state_between_contexts() {
        use crate::distributed::graph::InMemoryStateBackend;
        use std::sync::Arc;

        // Shared state backend (simulates what would be NATS KV in production)
        let shared_backend = Arc::new(InMemoryStateBackend::new());

        let wf_id = WorkflowId::new("wf-shared");

        // First executor sets state
        let ctx1: WorkflowContext<(), ()> = WorkflowContext::new(
            wf_id.clone(),
            TaskId::new("task-1"),
            ExecutorId::new("exe-1"),
            shared_backend.clone(),
        );

        ctx1.set_shared_state("counter", 42i32).await.unwrap();
        ctx1.set_shared_state("status", "processing").await.unwrap();

        // Second executor reads state (simulates next step in workflow)
        let ctx2: WorkflowContext<(), ()> = WorkflowContext::new(
            wf_id.clone(),
            TaskId::new("task-2"),
            ExecutorId::new("exe-2"),
            shared_backend.clone(),
        );

        let counter: Option<i32> = ctx2.get_shared_state("counter").await.unwrap();
        let status: Option<String> = ctx2.get_shared_state("status").await.unwrap();

        assert_eq!(counter, Some(42));
        assert_eq!(status, Some("processing".to_string()));
    }

    #[tokio::test]
    async fn test_previous_errors_available_in_retry() {
        let executor = UppercaseExecutor::new("upper-1");

        // Simulate retry scenario with previous errors
        let mut ctx: WorkflowContext<String, String> = WorkflowContext::in_memory(
            WorkflowId::new("wf-test"),
            TaskId::new("task-1"),
            executor.id().clone(),
        );

        // Add errors from "previous attempts"
        ctx.add_previous_errors(vec![
            ExecutorError::validation("First attempt failed: timeout"),
            ExecutorError::validation("Second attempt failed: rate limit"),
        ]);

        // Executor can access previous errors
        assert_eq!(ctx.previous_errors().len(), 2);

        // Execute still works
        let result = executor.handle("test".to_string(), &mut ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_executor_metadata() {
        let executor = UppercaseExecutor::new("my-executor");

        assert_eq!(executor.id().as_str(), "my-executor");
        assert_eq!(executor.kind(), ExecutorKind::Worker);
        assert_eq!(executor.name(), "UppercaseExecutor");
        assert_eq!(
            executor.description(),
            Some("Transforms input to uppercase")
        );
    }

    #[tokio::test]
    async fn test_executor_kind_variants() {
        let worker = UppercaseExecutor::new("w1");
        let validator = LengthValidator::new("v1", 1, 100);
        let orchestrator = SplitOrchestrator::new("o1");

        assert_eq!(worker.kind(), ExecutorKind::Worker);
        assert_eq!(validator.kind(), ExecutorKind::Validator);
        assert_eq!(orchestrator.kind(), ExecutorKind::Orchestrator);

        // Display
        assert_eq!(format!("{}", worker.kind()), "Worker");
        assert_eq!(format!("{}", validator.kind()), "Validator");
        assert_eq!(format!("{}", orchestrator.kind()), "Orchestrator");
    }
}
