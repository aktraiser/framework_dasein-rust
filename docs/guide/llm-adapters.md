# LLM Adapters

Agentic-RS provides a unified interface for multiple LLM providers through the `LLMAdapter` trait.

## Available Adapters

### OpenAI

```rust
use agentic_llm::OpenAIAdapter;

let llm = OpenAIAdapter::new("sk-...", "gpt-4o")
    .with_temperature(0.7)
    .with_max_tokens(4096);
```

**Supported models:** `gpt-4o`, `gpt-4o-mini`, `gpt-4-turbo`, `o1-preview`, `o1-mini`

### Anthropic (Claude)

```rust
use agentic_llm::AnthropicAdapter;

let llm = AnthropicAdapter::new("sk-ant-...", "claude-sonnet-4-20250514")
    .with_temperature(0.7)
    .with_max_tokens(4096);
```

**Supported models:** `claude-sonnet-4-20250514`, `claude-3-5-haiku-20241022`, `claude-3-opus-20240229`

### Google Gemini

```rust
use agentic_llm::GeminiAdapter;

let llm = GeminiAdapter::new("AIza...", "gemini-2.0-flash")
    .with_temperature(0.7)
    .with_max_tokens(8192);
```

**Supported models:** `gemini-2.0-flash`, `gemini-1.5-pro`, `gemini-1.5-flash`

### Ollama (Local)

```rust
use agentic_llm::OllamaAdapter;

let llm = OllamaAdapter::new("llama3.2")
    .with_base_url("http://localhost:11434")
    .with_temperature(0.7);
```

**Supported models:** Any model available in your Ollama installation

## The LLMAdapter Trait

All adapters implement the `LLMAdapter` trait:

```rust
#[async_trait]
pub trait LLMAdapter: Send + Sync {
    /// Get the provider name
    fn provider(&self) -> &str;

    /// Get the model name
    fn model(&self) -> &str;

    /// Generate a completion
    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError>;

    /// Generate a streaming completion
    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>>;

    /// Health check
    async fn health_check(&self) -> Result<bool, LLMError>;
}
```

## Messages

```rust
use agentic_llm::LLMMessage;

let messages = vec![
    LLMMessage::system("You are a helpful assistant."),
    LLMMessage::user("Hello!"),
    LLMMessage::assistant("Hi there! How can I help?"),
    LLMMessage::user("What's 2+2?"),
];
```

## Response Types

### LLMResponse

```rust
pub struct LLMResponse {
    pub content: String,           // Generated text
    pub tokens_used: TokenUsage,   // Token counts
    pub finish_reason: FinishReason,
    pub model: String,             // Actual model used
}

pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
    pub total: u32,
}
```

### StreamChunk

```rust
pub struct StreamChunk {
    pub content: String,
    pub done: bool,
    pub tokens_used: Option<TokenUsage>,
    pub finish_reason: Option<FinishReason>,
}
```

## Feature Flags

Enable only the providers you need:

```toml
[dependencies]
agentic-llm = { version = "0.1", features = ["openai"] }  # Only OpenAI
agentic-llm = { version = "0.1", features = ["anthropic", "gemini"] }  # Claude + Gemini
agentic-llm = { version = "0.1", features = ["all"] }  # All providers
```

## Implementing Custom Adapters

You can implement `LLMAdapter` for any provider:

```rust
use agentic_llm::{LLMAdapter, LLMMessage, LLMResponse, LLMError};

pub struct MyCustomAdapter {
    // your fields
}

#[async_trait]
impl LLMAdapter for MyCustomAdapter {
    fn provider(&self) -> &str {
        "my-provider"
    }

    fn model(&self) -> &str {
        "my-model"
    }

    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError> {
        // Your implementation
    }

    fn generate_stream(&self, messages: &[LLMMessage]) -> Pin<Box<dyn Stream<...> + Send + '_>> {
        // Your streaming implementation
    }

    async fn health_check(&self) -> Result<bool, LLMError> {
        // Your health check
    }
}
```
