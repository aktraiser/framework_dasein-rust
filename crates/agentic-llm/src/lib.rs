//! # agentic-llm
//!
//! LLM adapters for the Agentic Framework.
//!
//! Supports multiple providers:
//! - `OpenAI` (GPT-4, GPT-4o, o1)
//! - Ollama (local models)
//! - Anthropic (Claude Sonnet, Haiku, Opus)
//! - Google Gemini (Gemini 2.0 Flash, 1.5 Pro)
//!
//! ## Example
//!
//! ```rust,no_run
//! use dasein_agentic_llm::{LLMAdapter, OpenAIAdapter, LLMMessage};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let adapter = OpenAIAdapter::new("sk-...", "gpt-4o");
//!
//!     let messages = vec![
//!         LLMMessage::system("You are a helpful assistant."),
//!         LLMMessage::user("Hello!"),
//!     ];
//!
//!     let response = adapter.generate(&messages).await?;
//!     println!("{}", response.content);
//!
//!     Ok(())
//! }
//! ```

mod error;
mod traits;

#[cfg(feature = "openai")]
mod openai;

#[cfg(feature = "ollama")]
mod ollama;

#[cfg(feature = "anthropic")]
mod anthropic;

#[cfg(feature = "gemini")]
mod gemini;

pub use error::LLMError;
pub use traits::{
    FinishReason, LLMAdapter, LLMMessage, LLMResponse, Role, StreamChunk, TokenUsage,
};

#[cfg(feature = "openai")]
pub use openai::OpenAIAdapter;

#[cfg(feature = "ollama")]
pub use ollama::OllamaAdapter;

#[cfg(feature = "anthropic")]
pub use anthropic::AnthropicAdapter;

#[cfg(feature = "gemini")]
pub use gemini::GeminiAdapter;
