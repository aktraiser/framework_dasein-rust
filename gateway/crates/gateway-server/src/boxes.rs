//! Single-process Docker boxes: durable ownership/policy, fresh files per execution.
use crate::api::chat::ApiError;
use dasein_agentic_sandbox::{DockerSandbox, Sandbox};
use gateway_core::{sandbox::*, storage::Storage};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{watch, Mutex, Semaphore};

pub struct Boxes {
    slots: Mutex<HashMap<String, Arc<Slot>>>,
    capacity: Semaphore,
}
struct Slot {
    cancel: watch::Sender<bool>,
    permit: Semaphore,
}
impl Default for Boxes {
    fn default() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
            capacity: Semaphore::new(16),
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
        let _slots = self.slots.lock().await;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM docker_boxes")
            .fetch_one(storage.pool())
            .await
            .map_err(internal)?;
        if count >= 1000 {
            return Err(ApiError::Busy(
                "Box capacity reached; delete unused boxes".into(),
            ));
        }
        let id = SandboxId::new();
        sqlx::query("INSERT INTO docker_boxes (id, owner, policy) VALUES (?, ?, ?)")
            .bind(id.as_str())
            .bind(owner)
            .bind(serde_json::to_string(&request).map_err(internal)?)
            .execute(storage.pool())
            .await
            .map_err(internal)?;
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
        let slots = self.slots.lock().await;
        let request = Self::read(storage, owner, id).await?;
        let running = slots
            .get(id)
            .is_some_and(|slot| slot.permit.available_permits() == 0);
        drop(slots);
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
        let mut slots = self.slots.lock().await;
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
        let slot = slots
            .entry(id.into())
            .or_insert_with(|| {
                Arc::new(Slot {
                    cancel: watch::channel(false).0,
                    permit: Semaphore::new(1),
                })
            })
            .clone();
        let mut cancel = slot.cancel.subscribe();
        let _box_permit = slot
            .permit
            .try_acquire()
            .map_err(|_| ApiError::Busy("Sandbox already executing".into()))?;
        let _global_permit = self
            .capacity
            .try_acquire()
            .map_err(|_| ApiError::Busy("Execution capacity reached".into()))?;
        drop(slots);
        let command = format!(
            "printf %s {} | python -c {}",
            quote(request.stdin.as_deref().unwrap_or("")),
            quote(&request.code)
        );
        let result = tokio::select! {
            result = sandbox.execute(&command) => result.map_err(internal)?,
            _ = cancel.changed() => {
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
        let mut slots = self.slots.lock().await;
        let result = sqlx::query("DELETE FROM docker_boxes WHERE id=? AND owner=?")
            .bind(id)
            .bind(owner)
            .execute(storage.pool())
            .await
            .map_err(internal)?;
        if result.rows_affected() == 0 {
            return Err(missing());
        }
        if let Some(slot) = slots.remove(id) {
            slot.cancel.send_replace(true);
        }
        Ok(())
    }
}
