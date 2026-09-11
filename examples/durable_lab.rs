//! No LLM account required. Publish in one process, execute in another.
use dasein_agentic_core::distributed::bus::{
    DurableQueue, DurableQueueConfig, NatsClient, NatsConfig, Task, TaskOutcome,
};
use dasein_agentic_sandbox::{DockerSandbox, Sandbox};
use serde_json::json;
use std::{sync::Arc, time::Duration};
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let command = args.get(1).map_or("status", String::as_str);
    let namespace = args.get(2).map_or("lab", String::as_str);
    let task_id = args.get(3).map_or("answer-42", String::as_str);
    let nats = Arc::new(
        NatsClient::connect(NatsConfig::new(
            std::env::var("DASEIN_TEST_NATS_URL")
                .unwrap_or_else(|_| "nats://127.0.0.1:4222".into()),
        ))
        .await?,
    );
    let queue = DurableQueue::new(nats, namespace, DurableQueueConfig::default()).await?;
    match command {
        "publish" => {
            queue
                .publish(Task::new(
                    task_id,
                    "python",
                    json!({"code":"python -c 'print(6*7)'"}),
                ))
                .await?;
            println!("Persisted {task_id}");
        }
        "work" => {
            let worker = queue.worker("python").await?;
            let delivery = worker
                .next(Duration::from_secs(5))
                .await?
                .ok_or_else(|| anyhow::anyhow!("No pending task"))?;
            let code = delivery.task.payload["code"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Missing code"))?
                .to_owned();
            // Keep the lease alive through Docker execution and cleanup.
            let sandbox = DockerSandbox::default();
            let execution = sandbox.execute(&code);
            tokio::pin!(execution);
            let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
            let result = loop {
                tokio::select! { result=&mut execution => break result?, _=heartbeat.tick() => delivery.heartbeat().await? }
            };
            anyhow::ensure!(result.exit_code == 0, "Execution failed: {}", result.stderr);
            delivery.complete(json!({"stdout":result.stdout,"stderr":result.stderr,"exit_code":result.exit_code})).await?;
            println!("Completed: {}", result.stdout.trim());
        }
        "status" => {
            let outcome = queue.outcome(task_id).await?;
            println!("{}", serde_json::to_string_pretty(&outcome)?);
            anyhow::ensure!(
                matches!(outcome, Some(TaskOutcome::Completed(_))),
                "Task is not completed"
            );
        }
        _ => anyhow::bail!("Usage: durable_lab publish|work|status [namespace] [task-id]"),
    }
    Ok(())
}
