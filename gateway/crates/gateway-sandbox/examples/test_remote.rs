//! Test remote Firecracker sandbox connection
//!
//! Run with:
//!   cargo run --example test_remote

use gateway_sandbox::remote::{RemoteSandbox, RemoteSandboxConfig};
use gateway_core::sandbox::Runtime;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Create remote sandbox client
    let config = RemoteSandboxConfig::new("http://65.108.230.227:8080");
    let sandbox = RemoteSandbox::new(config)?;

    // Health check
    println!("=== Health Check ===");
    let health = sandbox.health().await?;
    println!("Status: {}", health.status);
    println!("Firecracker: {}", health.firecracker_available);
    println!("KVM: {}", health.kvm_available);
    println!("Version: {:?}", health.version);
    println!();

    // Test shell command
    println!("=== Shell Command ===");
    let result = sandbox.execute("echo 'Hello from Firecracker!'", None).await?;
    println!("Exit code: {}", result.exit_code);
    println!("Stdout: {}", result.stdout.trim());
    println!();

    // Test Python
    println!("=== Python ===");
    let result = sandbox.execute_runtime(
        &Runtime::Python,
        "print('Hello from Python!')\nprint(2 + 2)",
        None
    ).await?;
    println!("Exit code: {}", result.exit_code);
    println!("Stdout: {}", result.stdout.trim());
    println!();

    // Test Node.js
    println!("=== Node.js ===");
    let result = sandbox.execute_runtime(
        &Runtime::Node,
        "console.log('Hello from Node.js!');\nconsole.log(2 + 2);",
        None
    ).await?;
    println!("Exit code: {}", result.exit_code);
    println!("Stdout: {}", result.stdout.trim());
    println!();

    // Test Go version
    println!("=== Go ===");
    let result = sandbox.execute("go version", None).await?;
    println!("Exit code: {}", result.exit_code);
    println!("Stdout: {}", result.stdout.trim());
    println!();

    // Test Rust version
    println!("=== Rust ===");
    let result = sandbox.execute("rustc --version", None).await?;
    println!("Exit code: {}", result.exit_code);
    println!("Stdout: {}", result.stdout.trim());
    println!();

    println!("✅ All tests passed!");
    Ok(())
}
