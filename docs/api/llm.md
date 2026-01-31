# agentic-llm

LLM adapters for multiple providers with a unified interface.

## Installation

```toml
[dependencies]
agentic-llm = { version = "0.1", features = ["all"] }
```

### Feature Flags

| Feature | Description |
|---------|-------------|
| `openai` | OpenAI GPT models (default) |
| `ollama` | Ollama local models (default) |
| `anthropic` | Anthropic Claude models |
| `gemini` | Google Gemini models |
| `all` | All providers |

## Adapters

### OpenAIAdapter

```rust
impl OpenAIAdapter {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self;
    pub fn with_temperature(self, temperature: f32) -> Self;
    pub fn with_max_tokens(self, max_tokens: u32) -> Self;
}
```

### AnthropicAdapter

```rust
impl AnthropicAdapter {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self;
    pub fn with_temperature(self, temperature: f32) -> Self;
    pub fn with_max_tokens(self, max_tokens: u32) -> Self;
}
```

### GeminiAdapter

```rust
impl GeminiAdapter {
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self;
    pub fn with_temperature(self, temperature: f32) -> Self;
    pub fn with_max_tokens(self, max_tokens: u32) -> Self;
}
```

### OllamaAdapter

```rust
impl OllamaAdapter {
    pub fn new(model: impl Into<String>) -> Self;
    pub fn with_base_url(self, base_url: impl Into<String>) -> Self;
    pub fn with_temperature(self, temperature: f32) -> Self;
}
```

## Traits

### LLMAdapter

```rust
#[async_trait]
pub trait LLMAdapter: Send + Sync {
    fn provider(&self) -> &str;
    fn model(&self) -> &str;

    async fn generate(&self, messages: &[LLMMessage]) -> Result<LLMResponse, LLMError>;

    fn generate_stream(
        &self,
        messages: &[LLMMessage],
    ) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, LLMError>> + Send + '_>>;

    async fn health_check(&self) -> Result<bool, LLMError>;
}
```

## Types

### LLMMessage

```rust
pub struct LLMMessage {
    pub role: Role,
    pub content: String,
}

impl LLMMessage {
    pub fn system(content: impl Into<String>) -> Self;
    pub fn user(content: impl Into<String>) -> Self;
    pub fn assistant(content: impl Into<String>) -> Self;
}

pub enum Role {
    System,
    User,
    Assistant,
}
```

### LLMResponse

```rust
pub struct LLMResponse {
    pub content: String,
    pub tokens_used: TokenUsage,
    pub finish_reason: FinishReason,
    pub model: String,
}

pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
    pub total: u32,
}

pub enum FinishReason {
    Stop,
    Length,
    Error,
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

### LLMError

```rust
pub enum LLMError {
    ApiError(String),
    ConnectionError(String),
    InvalidResponse(String),
    EmptyResponse,
    RateLimited,
    AuthenticationFailed,
}
```
