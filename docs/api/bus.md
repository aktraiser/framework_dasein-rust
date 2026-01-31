# agentic-bus

NATS message bus for inter-agent communication.

## Installation

```toml
[dependencies]
agentic-bus = "0.1"
```

## NatsBus

```rust
use dasein_agentic_bus::NatsBus;

// Connect to NATS server
let bus = NatsBus::connect("nats://localhost:4222").await?;

// Publish a message
bus.publish("agents.agent-1.inbox", &message).await?;

// Subscribe to messages
let mut subscriber = bus.subscribe("agents.*.outbox").await?;
while let Some(msg) = subscriber.next().await {
    println!("Received: {:?}", msg);
}

// Request/Reply pattern
let response = bus.request("orchestrator.inbox", &task, 5000).await?;
```

## Trait

### MessageBus

```rust
#[async_trait]
pub trait MessageBus: Send + Sync {
    async fn publish(&self, subject: &str, message: &Message) -> Result<(), BusError>;

    async fn subscribe(&self, subject: &str) -> Result<Subscriber, BusError>;

    async fn request(
        &self,
        subject: &str,
        message: &Message,
        timeout_ms: u64,
    ) -> Result<Message, BusError>;

    async fn queue_subscribe(
        &self,
        subject: &str,
        queue_group: &str,
    ) -> Result<Subscriber, BusError>;
}
```

## Subject Patterns

| Pattern | Description |
|---------|-------------|
| `agents.{id}.inbox` | Send tasks to specific agent |
| `agents.{id}.outbox` | Receive results from agent |
| `agents.{id}.events` | Agent lifecycle events |
| `orchestrator.inbox` | Central coordinator |
| `system.events` | System-wide notifications |

## Errors

```rust
pub enum BusError {
    ConnectionFailed(String),
    PublishFailed(String),
    SubscribeFailed(String),
    Timeout,
    SerializationError(String),
}
```
