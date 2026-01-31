//! Gateway Sandbox - Firecracker microVM management
//!
//! This crate provides:
//! - Firecracker microVM lifecycle management
//! - Sandbox pooling for fast allocation
//! - Code execution with isolation
//! - Remote execution via HTTP API
//! - File upload/download
//! - Remote VM orchestration via SSH

pub mod firecracker;
pub mod pool;
pub mod executor;
pub mod remote;
pub mod orchestrator;

pub use firecracker::FirecrackerManager;
pub use pool::SandboxPool;
pub use executor::{CodeExecutor, ExecutionBackend};
pub use remote::{RemoteSandbox, RemoteSandboxConfig, RemoteSandboxBuilder, FileInfo};
pub use orchestrator::{FirecrackerOrchestrator, OrchestratorConfig, VmStatus};
