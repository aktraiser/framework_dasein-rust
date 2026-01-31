//! # agentic-sandbox
//!
//! Sandboxed code execution for the Agentic Framework.
//!
//! Provides isolated environments to safely execute LLM-generated code.
//!
//! ## Features
//!
//! - `process` (default) - Local process execution (for development)
//! - `docker` - Docker container isolation (for production)
//! - `remote` - Remote Firecracker server (for macOS/Windows development)
//!
//! ## Example
//!
//! ```rust,no_run
//! use agentic_sandbox::{Sandbox, ProcessSandbox};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let sandbox = ProcessSandbox::new();
//!
//!     let result = sandbox.execute("echo 'Hello, World!'").await?;
//!     println!("Output: {}", result.stdout);
//!
//!     Ok(())
//! }
//! ```
//!
//! ## Remote Sandbox (for macOS/Windows)
//!
//! ```rust,no_run
//! use agentic_sandbox::{Sandbox, RemoteSandbox};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let sandbox = RemoteSandbox::new("http://your-server:8080")
//!         .api_key("your-secret-key")
//!         .build();
//!
//!     let result = sandbox.execute("echo 'Hello from Firecracker!'").await?;
//!     println!("Output: {}", result.stdout);
//!
//!     Ok(())
//! }
//! ```

mod error;
mod traits;

#[cfg(feature = "process")]
mod process;

#[cfg(feature = "docker")]
mod docker;

#[cfg(feature = "remote")]
mod remote;

mod firecracker;

pub use error::SandboxError;
pub use traits::{ExecutionResult, Sandbox, SandboxArtifact};

#[cfg(feature = "process")]
pub use process::ProcessSandbox;

#[cfg(feature = "docker")]
pub use docker::DockerSandbox;

#[cfg(feature = "remote")]
pub use remote::{RemoteSandbox, RemoteSandboxConfig, RemoteSandboxBuilder, ExecuteRequest, ExecuteResponse};

pub use firecracker::{FirecrackerSandbox, FirecrackerConfig, FirecrackerSandboxBuilder};
