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
//! - `remote` - Remote Firecracker server (legacy API)
//! - `gateway` - Agentic Gateway integration (recommended for Firecracker)
//!
//! ## Example
//!
//! ```rust,no_run
//! use dasein_agentic_sandbox::{Sandbox, ProcessSandbox};
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
//! ## Gateway Sandbox (recommended for Firecracker)
//!
//! ```rust,ignore
//! use dasein_agentic_sandbox::{Sandbox, GatewaySandbox};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let sandbox = GatewaySandbox::builder("http://gateway:8080")
//!         .runtime("python")
//!         .build()
//!         .await?;
//!
//!     let result = sandbox.execute("print('Hello from Firecracker!')").await?;
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

#[cfg(feature = "gateway")]
mod gateway;

mod firecracker;

pub use error::SandboxError;
pub use traits::{ExecutionResult, Sandbox, SandboxArtifact};

#[cfg(feature = "process")]
pub use process::ProcessSandbox;

#[cfg(feature = "docker")]
pub use docker::DockerSandbox;

#[cfg(feature = "remote")]
pub use remote::{
    ExecuteRequest, ExecuteResponse, RemoteSandbox, RemoteSandboxBuilder, RemoteSandboxConfig,
};

#[cfg(feature = "gateway")]
pub use gateway::{
    GatewayHealthResponse, GatewaySandbox, GatewaySandboxBuilder, GatewaySandboxConfig, Runtime,
};

pub use firecracker::{FirecrackerConfig, FirecrackerSandbox, FirecrackerSandboxBuilder};
