//! Gateway Sandbox - Firecracker microVM management
//!
//! This crate provides:
//! - Firecracker microVM lifecycle management
//! - Sandbox pooling for fast allocation
//! - Code execution with isolation
//! - Remote execution via HTTP API
//! - File upload/download
//! - Remote VM orchestration via SSH

pub mod executor;
pub mod firecracker;
pub mod orchestrator;
pub mod pool;
pub mod remote;

pub use executor::{CodeExecutor, ExecutionBackend};
pub use firecracker::FirecrackerManager;
pub use orchestrator::{FirecrackerOrchestrator, OrchestratorConfig, VmStatus};
pub use pool::SandboxPool;
pub use remote::{FileInfo, RemoteSandbox, RemoteSandboxBuilder, RemoteSandboxConfig};
