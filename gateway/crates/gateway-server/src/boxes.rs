//! Docker boxes: durable ownership/policy, fresh files per execution.
//!
//! What must hold across executions is kept in the database, not in this
//! process, so that several gateway processes sharing the database agree:
//! one execution at a time per box, a limit on boxes executing at once, and
//! a deleted box stops executing wherever it runs.
use crate::api::chat::ApiError;
use dasein_agentic_sandbox::{DockerSandbox, Sandbox};
use gateway_core::{
    sandbox::*,
    storage::{BoxClaim, Storage},
};
use std::time::Duration;

/// Boxes that exist at most.
const MAX_BOXES: i64 = 1000;
/// Boxes executing at once at most, over all gateway processes.
const MAX_EXECUTING: i64 = 16;
/// How long a claim outlives the execution's own timeout: what a process
/// that died leaves to expire.
const CLAIM_GRACE_SECONDS: i64 = 30;
/// How often an execution looks whether its box was deleted.
const DELETION_CHECK: Duration = Duration::from_millis(500);

#[derive(Default)]
pub struct Boxes;

/// The right to execute in a box, given back when dropped: also when the
/// request is abandoned by its client.
struct Claim {
    storage: Storage,
    box_id: String,
    run_id: String,
}
impl Drop for Claim {
    fn drop(&mut self) {
        let (storage, box_id, run_id) = (
            self.storage.clone(),
            std::mem::take(&mut self.box_id),
            std::mem::take(&mut self.run_id),
        );
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Err(error) = storage.release_box_run(&box_id, &run_id).await {
                    // It ends by itself at its expiry.
                    tracing::warn!(%error, %box_id, "Could not release a box");
                }
            });
        }
    }
}
fn internal(e: impl std::fmt::Display) -> ApiError {
    ApiError::Internal(e.to_string())
}
fn missing() -> ApiError {
    ApiError::NotFound("Sandbox not found".into())
}
fn policy(request: &CreateSandboxRequest, timeout: u32) -> Result<DockerSandbox, ApiError> {
    if request.runtime != Runtime::Python {
        return Err(ApiError::Unavailable(
            "Docker beta supports Python; other runtimes need qualification".into(),
        ));
    }
    if !request.packages.is_empty() {
        return Err(ApiError::BadRequest(
            "Install dependencies in an operator-managed image before use".into(),
        ));
    }
    let memory = request.memory_mb.unwrap_or(256);
    if !(32..=512).contains(&memory) || !(1..=120).contains(&timeout) {
        return Err(ApiError::BadRequest(
            "Memory must be 32..512 MiB and timeout 1..120 seconds".into(),
        ));
    }
    Ok(DockerSandbox::builder()
        .image(std::env::var("DASEIN_BOX_IMAGE").unwrap_or_else(|_| "python:3.12-alpine".into()))
        .memory_limit(format!("{memory}m"))
        .timeout_ms(u64::from(timeout) * 1000)
        .build())
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}
impl Boxes {
    pub async fn create(
        &self,
        storage: &Storage,
        owner: &str,
        request: CreateSandboxRequest,
    ) -> Result<CreateSandboxResponse, ApiError> {
        let sandbox = policy(&request, request.timeout_seconds.unwrap_or(30))?;
        if !sandbox.is_ready().await.map_err(internal)? {
            return Err(ApiError::ServiceUnavailable(
                "Docker image is not installed".into(),
            ));
        }
        let id = SandboxId::new();
        let policy = serde_json::to_string(&request).map_err(internal)?;
        if !storage
            .create_box(id.as_str(), owner, &policy, MAX_BOXES)
            .await
            .map_err(internal)?
        {
            return Err(ApiError::Busy(
                "Box capacity reached; delete unused boxes".into(),
            ));
        }
        Ok(CreateSandboxResponse {
            sandbox_id: id,
            status: SandboxStatus::Ready,
        })
    }
    async fn read(
        storage: &Storage,
        owner: &str,
        id: &str,
    ) -> Result<CreateSandboxRequest, ApiError> {
        let record: Option<String> =
            sqlx::query_scalar("SELECT policy FROM docker_boxes WHERE id=? AND owner=?")
                .bind(id)
                .bind(owner)
                .fetch_optional(storage.pool())
                .await
                .map_err(internal)?;
        serde_json::from_str(&record.ok_or_else(missing)?).map_err(internal)
    }
    pub async fn info(
        &self,
        storage: &Storage,
        owner: &str,
        id: &str,
    ) -> Result<(SandboxStatus, bool), ApiError> {
        let request = Self::read(storage, owner, id).await?;
        let running = storage.box_is_running(id).await.map_err(internal)?;
        let ready = policy(&request, request.timeout_seconds.unwrap_or(30))?
            .is_ready()
            .await
            .unwrap_or(false);
        Ok((
            if running {
                SandboxStatus::Running
            } else if ready {
                SandboxStatus::Ready
            } else {
                SandboxStatus::Stopped
            },
            ready,
        ))
    }
    pub async fn execute(
        &self,
        storage: &Storage,
        owner: &str,
        id: &str,
        request: ExecuteRequest,
    ) -> Result<ExecuteResponse, ApiError> {
        let settings = Self::read(storage, owner, id).await?;
        if request.runtime != settings.runtime {
            return Err(ApiError::BadRequest(
                "Runtime differs from sandbox policy".into(),
            ));
        }
        if request.code.len() > 256 * 1024
            || request.stdin.as_ref().is_some_and(|s| s.len() > 256 * 1024)
        {
            return Err(ApiError::BadRequest(
                "Code and stdin each have a 256 KiB limit".into(),
            ));
        }
        let maximum = settings.timeout_seconds.unwrap_or(30);
        let timeout = request.timeout_seconds.unwrap_or(maximum);
        if timeout > maximum {
            return Err(ApiError::BadRequest(
                "Execution timeout exceeds sandbox policy".into(),
            ));
        }
        let sandbox = policy(&settings, timeout)?;
        let run_id = uuid::Uuid::new_v4().simple().to_string();
        match storage
            .claim_box_run(
                id,
                owner,
                &run_id,
                i64::from(timeout) + CLAIM_GRACE_SECONDS,
                MAX_EXECUTING,
            )
            .await
            .map_err(internal)?
        {
            BoxClaim::Claimed => {}
            BoxClaim::Busy => return Err(ApiError::Busy("Sandbox already executing".into())),
            BoxClaim::Full => return Err(ApiError::Busy("Execution capacity reached".into())),
            BoxClaim::Missing => return Err(missing()),
        }
        let _claim = Claim {
            storage: storage.clone(),
            box_id: id.to_owned(),
            run_id,
        };
        let command = format!(
            "printf %s {} | python -c {}",
            quote(request.stdin.as_deref().unwrap_or("")),
            quote(&request.code)
        );
        // The box may be deleted through any gateway process.
        let deleted = async {
            loop {
                tokio::time::sleep(DELETION_CHECK).await;
                if matches!(storage.box_exists(id, owner).await, Ok(false)) {
                    return;
                }
            }
        };
        let result = tokio::select! {
            result = sandbox.execute(&command) => result.map_err(internal)?,
            () = deleted => {
                sandbox.stop().await.map_err(internal)?;
                return Err(ApiError::NotFound("Sandbox deleted during execution".into()));
            }
        };
        Ok(ExecuteResponse {
            stdout: result.stdout,
            stderr: result.stderr,
            exit_code: result.exit_code,
            duration_ms: result.execution_time_ms,
            error: None,
        })
    }
    pub async fn delete(&self, storage: &Storage, owner: &str, id: &str) -> Result<(), ApiError> {
        let result = sqlx::query("DELETE FROM docker_boxes WHERE id=? AND owner=?")
            .bind(id)
            .bind(owner)
            .execute(storage.pool())
            .await
            .map_err(internal)?;
        if result.rows_affected() == 0 {
            return Err(missing());
        }
        // An execution in progress, here or in another process, sees the
        // box gone and stops.
        Ok(())
    }
}
