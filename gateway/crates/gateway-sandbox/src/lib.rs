//! Gateway Sandbox - Firecracker microVM management
//!
//! This crate provides:
//! - Firecracker microVM lifecycle management
//! - Sandbox pooling for fast allocation
//! - Code execution with isolation
//! - File upload/download

pub mod firecracker;
pub mod pool;
pub mod executor;

pub use firecracker::FirecrackerManager;
pub use pool::SandboxPool;
pub use executor::CodeExecutor;
