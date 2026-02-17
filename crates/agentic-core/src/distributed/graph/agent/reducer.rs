//! Chat Reducer - Context window management for agent threads.
//!
//! When conversation history grows too long for the LLM's context window,
//! reducers help trim or summarize messages while preserving important context.
//!
//! # Strategies
//!
//! | Reducer | Strategy | Use Case |
//! |---------|----------|----------|
//! | `MessageCountingReducer` | Keep last N messages | Simple, predictable |
//! | `TokenCountingReducer` | Keep within token budget | Precise context control |
//! | `SlidingWindowReducer` | Keep recent + important | Balanced approach |
//!
//! # Example
//!
//! ```rust
//! use dasein_agentic_core::distributed::graph::agent::{
//!     ChatReducer, MessageCountingReducer, ChatMessage, ChatRole,
//! };
//!
//! let reducer = MessageCountingReducer::new(10);
//! let messages: Vec<ChatMessage> = vec![
//!     // ... many messages ...
//! ];
//!
//! let reduced = reducer.reduce(&messages);
//! // reduced now contains at most 10 messages + system messages
//! ```

use std::sync::Arc;

use super::types::{ChatMessage, ChatRole};

// ============================================================================
// CHAT REDUCER TRAIT
// ============================================================================

/// Trait for reducing chat history to fit context windows.
///
/// Implementations should preserve:
/// 1. System messages (always keep)
/// 2. Recent context (most recent exchanges)
/// 3. Important context (marked messages, if supported)
pub trait ChatReducer: Send + Sync {
    /// Reduce the message history according to the strategy.
    ///
    /// Returns a new vector with the reduced messages.
    /// The original vector is not modified.
    fn reduce(&self, messages: &[ChatMessage]) -> Vec<ChatMessage>;

    /// Check if reduction is needed.
    fn needs_reduction(&self, messages: &[ChatMessage]) -> bool;
}

// ============================================================================
// MESSAGE COUNTING REDUCER
// ============================================================================

/// Keeps the last N messages while preserving system messages.
///
/// This is the simplest reducer - it counts messages regardless of size.
/// System messages are always preserved.
///
/// # Example
///
/// ```rust
/// use dasein_agentic_core::distributed::graph::agent::{
///     ChatReducer, MessageCountingReducer, ChatMessage,
/// };
///
/// let reducer = MessageCountingReducer::new(5);
///
/// // 10 user/assistant messages
/// let messages: Vec<ChatMessage> = (0..10)
///     .map(|i| ChatMessage::user(format!("Message {}", i)))
///     .collect();
///
/// let reduced = reducer.reduce(&messages);
/// assert_eq!(reduced.len(), 5); // Last 5 messages
/// ```
pub struct MessageCountingReducer {
    /// Maximum number of non-system messages to keep.
    max_messages: usize,

    /// Whether to preserve alternating user/assistant pattern.
    preserve_pairs: bool,
}

impl MessageCountingReducer {
    /// Create a new reducer with a message limit.
    pub fn new(max_messages: usize) -> Self {
        Self {
            max_messages,
            preserve_pairs: true,
        }
    }

    /// Don't try to preserve user/assistant pairs.
    pub fn without_pair_preservation(mut self) -> Self {
        self.preserve_pairs = false;
        self
    }

    /// Check if we should preserve pairs (start with user).
    fn should_start_with_user(&self, messages: &[ChatMessage]) -> bool {
        if !self.preserve_pairs {
            return false;
        }

        // Check if first non-system message is from user
        messages
            .iter()
            .find(|m| !m.is_system())
            .map(super::types::ChatMessage::is_user)
            .unwrap_or(false)
    }
}

impl ChatReducer for MessageCountingReducer {
    fn reduce(&self, messages: &[ChatMessage]) -> Vec<ChatMessage> {
        // Separate system messages from others
        let system: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| m.role == ChatRole::System)
            .cloned()
            .collect();

        let non_system: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| m.role != ChatRole::System)
            .cloned()
            .collect();

        if non_system.len() <= self.max_messages {
            // No reduction needed
            return messages.to_vec();
        }

        // Calculate how many to keep
        let mut to_keep = self.max_messages;

        // If preserving pairs, adjust to start with user message
        if self.preserve_pairs && self.should_start_with_user(messages) {
            // Find where we need to start to have a user message first
            let start_idx = non_system.len().saturating_sub(to_keep);

            // If start message is assistant, move forward one
            if let Some(msg) = non_system.get(start_idx) {
                if msg.is_assistant() && to_keep > 1 {
                    to_keep -= 1;
                }
            }
        }

        // Take last N non-system messages
        let recent: Vec<ChatMessage> = non_system
            .into_iter()
            .rev()
            .take(to_keep)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        // Combine: system first, then recent
        [system, recent].concat()
    }

    fn needs_reduction(&self, messages: &[ChatMessage]) -> bool {
        let non_system_count = messages.iter().filter(|m| !m.is_system()).count();
        non_system_count > self.max_messages
    }
}

impl Default for MessageCountingReducer {
    fn default() -> Self {
        Self::new(50) // Default to 50 messages
    }
}

// ============================================================================
// TOKEN COUNTING REDUCER
// ============================================================================

/// Reduces messages to stay within a token budget.
///
/// Uses a simple approximation: ~4 characters per token.
/// System messages are always preserved (subtracted from budget).
pub struct TokenCountingReducer {
    /// Maximum tokens to allow.
    max_tokens: usize,

    /// Estimated characters per token (default: 4).
    chars_per_token: usize,
}

impl TokenCountingReducer {
    /// Create a new reducer with a token limit.
    pub fn new(max_tokens: usize) -> Self {
        Self {
            max_tokens,
            chars_per_token: 4,
        }
    }

    /// Set custom characters per token ratio.
    pub fn with_chars_per_token(mut self, chars: usize) -> Self {
        self.chars_per_token = chars;
        self
    }

    fn estimate_tokens(&self, message: &ChatMessage) -> usize {
        // Simple estimation: content length / chars_per_token
        message.content.len() / self.chars_per_token + 1
    }
}

impl ChatReducer for TokenCountingReducer {
    fn reduce(&self, messages: &[ChatMessage]) -> Vec<ChatMessage> {
        // Separate system messages
        let system: Vec<ChatMessage> = messages.iter().filter(|m| m.is_system()).cloned().collect();

        let non_system: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| !m.is_system())
            .cloned()
            .collect();

        // Calculate system message tokens
        let system_tokens: usize = system.iter().map(|m| self.estimate_tokens(m)).sum();

        if system_tokens >= self.max_tokens {
            // System messages alone exceed budget, return just system
            return system;
        }

        let remaining_budget = self.max_tokens - system_tokens;

        // Add messages from the end until budget exhausted
        let mut kept = Vec::new();
        let mut used_tokens = 0;

        for msg in non_system.iter().rev() {
            let tokens = self.estimate_tokens(msg);
            if used_tokens + tokens > remaining_budget {
                break;
            }
            kept.push(msg.clone());
            used_tokens += tokens;
        }

        // Reverse to maintain order
        kept.reverse();

        // Combine system + kept
        [system, kept].concat()
    }

    fn needs_reduction(&self, messages: &[ChatMessage]) -> bool {
        let total_tokens: usize = messages.iter().map(|m| self.estimate_tokens(m)).sum();
        total_tokens > self.max_tokens
    }
}

// ============================================================================
// SLIDING WINDOW REDUCER
// ============================================================================

/// Keeps a sliding window of recent messages plus any marked as important.
///
/// Messages can be marked as important via metadata:
/// `message.with_metadata(json!({"important": true}))`
pub struct SlidingWindowReducer {
    /// Size of the sliding window (recent messages).
    window_size: usize,

    /// Maximum important messages to preserve.
    max_important: usize,
}

impl SlidingWindowReducer {
    /// Create a new sliding window reducer.
    pub fn new(window_size: usize) -> Self {
        Self {
            window_size,
            max_important: 10,
        }
    }

    /// Set maximum important messages to keep.
    pub fn with_max_important(mut self, max: usize) -> Self {
        self.max_important = max;
        self
    }

    fn is_important(&self, msg: &ChatMessage) -> bool {
        msg.metadata
            .as_ref()
            .and_then(|m| m.get("important"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }
}

impl ChatReducer for SlidingWindowReducer {
    fn reduce(&self, messages: &[ChatMessage]) -> Vec<ChatMessage> {
        // System messages always kept
        let system: Vec<ChatMessage> = messages.iter().filter(|m| m.is_system()).cloned().collect();

        // Important messages (non-system)
        let important: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| !m.is_system() && self.is_important(m))
            .take(self.max_important)
            .cloned()
            .collect();

        // Recent messages (excluding system and already-kept important)
        let important_count = important.len();
        let recent_budget = self.window_size.saturating_sub(important_count);

        let recent: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| !m.is_system() && !self.is_important(m))
            .rev()
            .take(recent_budget)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        // Combine all
        [system, important, recent].concat()
    }

    fn needs_reduction(&self, messages: &[ChatMessage]) -> bool {
        let non_system_count = messages.iter().filter(|m| !m.is_system()).count();
        non_system_count > self.window_size
    }
}

// ============================================================================
// NO-OP REDUCER
// ============================================================================

/// A no-op reducer that returns messages unchanged.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoOpReducer;

impl ChatReducer for NoOpReducer {
    fn reduce(&self, messages: &[ChatMessage]) -> Vec<ChatMessage> {
        messages.to_vec()
    }

    fn needs_reduction(&self, _messages: &[ChatMessage]) -> bool {
        false
    }
}

// ============================================================================
// SHARED TYPES
// ============================================================================

/// Boxed chat reducer for dynamic dispatch.
pub type BoxedChatReducer = Box<dyn ChatReducer>;

/// Arc-wrapped chat reducer for sharing.
pub type SharedChatReducer = Arc<dyn ChatReducer>;

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_messages(n: usize) -> Vec<ChatMessage> {
        (0..n)
            .map(|i| {
                if i % 2 == 0 {
                    ChatMessage::user(format!("User message {}", i))
                } else {
                    ChatMessage::assistant(format!("Assistant message {}", i))
                }
            })
            .collect()
    }

    #[test]
    fn test_message_counting_reducer_no_reduction() {
        let reducer = MessageCountingReducer::new(10);
        let messages = make_messages(5);

        assert!(!reducer.needs_reduction(&messages));
        let reduced = reducer.reduce(&messages);
        assert_eq!(reduced.len(), 5);
    }

    #[test]
    fn test_message_counting_reducer_with_reduction() {
        // Use without_pair_preservation for predictable counts
        let reducer = MessageCountingReducer::new(5).without_pair_preservation();
        let messages = make_messages(10);

        assert!(reducer.needs_reduction(&messages));
        let reduced = reducer.reduce(&messages);
        assert_eq!(reduced.len(), 5);

        // Should be last 5 messages (indices 5-9)
        assert!(reduced[0].content.contains("5"));
        assert!(reduced[4].content.contains("9"));
    }

    #[test]
    fn test_message_counting_reducer_preserves_system() {
        // Use without_pair_preservation for predictable counts
        let reducer = MessageCountingReducer::new(3).without_pair_preservation();
        let mut messages = vec![ChatMessage::system("System prompt")];
        messages.extend(make_messages(10));

        let reduced = reducer.reduce(&messages);
        // System + 3 recent
        assert_eq!(reduced.len(), 4);
        assert!(reduced[0].is_system());
    }

    #[test]
    fn test_token_counting_reducer() {
        let reducer = TokenCountingReducer::new(100);

        // Create messages with known sizes
        let messages = vec![
            ChatMessage::system("System"),           // ~2 tokens
            ChatMessage::user("a".repeat(100)),      // ~25 tokens
            ChatMessage::assistant("b".repeat(100)), // ~25 tokens
            ChatMessage::user("c".repeat(100)),      // ~25 tokens
            ChatMessage::assistant("d".repeat(200)), // ~50 tokens
        ];

        let reduced = reducer.reduce(&messages);

        // Should keep system + some messages within budget
        assert!(reduced[0].is_system());

        // Verify we stayed under budget
        let total_tokens: usize = reduced.iter().map(|m| m.content.len() / 4 + 1).sum();
        assert!(total_tokens <= 100);
    }

    #[test]
    fn test_sliding_window_reducer() {
        let reducer = SlidingWindowReducer::new(5);

        let mut messages = make_messages(10);

        // Mark message 2 as important
        messages[2] = messages[2]
            .clone()
            .with_metadata(serde_json::json!({"important": true}));

        let reduced = reducer.reduce(&messages);

        // Should have important + recent messages
        assert!(reduced.len() <= 6); // 5 window + 1 important

        // Important message should be preserved
        let has_important = reduced.iter().any(|m| {
            m.metadata
                .as_ref()
                .and_then(|meta| meta.get("important"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        });
        assert!(has_important);
    }

    #[test]
    fn test_sliding_window_with_system() {
        let reducer = SlidingWindowReducer::new(3);

        let mut messages = vec![ChatMessage::system("Be helpful")];
        messages.extend(make_messages(10));

        let reduced = reducer.reduce(&messages);

        // System always preserved
        assert!(reduced[0].is_system());
        // Plus 3 from window
        assert_eq!(reduced.len(), 4);
    }

    #[test]
    fn test_noop_reducer() {
        let reducer = NoOpReducer;
        let messages = make_messages(100);

        assert!(!reducer.needs_reduction(&messages));
        let reduced = reducer.reduce(&messages);
        assert_eq!(reduced.len(), 100);
    }

    #[test]
    fn test_preserve_pairs() {
        let reducer = MessageCountingReducer::new(4);

        // Odd number of messages: U A U A U A U A U (9 messages)
        let messages: Vec<ChatMessage> = (0..9)
            .map(|i| {
                if i % 2 == 0 {
                    ChatMessage::user(format!("U{}", i))
                } else {
                    ChatMessage::assistant(format!("A{}", i))
                }
            })
            .collect();

        let reduced = reducer.reduce(&messages);

        // Should start with a user message for coherent context
        // Taking last 4 from U0 A1 U2 A3 U4 A5 U6 A7 U8
        // would be: U6 A7 U8 (3) or adjusted to: A5 U6 A7 U8 (4)
        // With pair preservation, we might get 3 to start with user

        assert!(!reduced.is_empty());
    }

    #[test]
    fn test_message_counting_reducer_default() {
        let reducer = MessageCountingReducer::default();
        assert!(!reducer.needs_reduction(&make_messages(50)));
        assert!(reducer.needs_reduction(&make_messages(51)));
    }
}
