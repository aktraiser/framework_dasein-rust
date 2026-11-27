//! Conversation history for calls made without tool definitions.
//!
//! Providers only accept their native tool-call and tool-result blocks in a
//! request that also declares the tools. A plain or streamed call on a history
//! that contains tool rounds therefore cannot send those blocks. Sending the
//! tool output as if the user had typed it, and dropping what the assistant
//! asked for, leaves the model guessing; this module writes both out as text
//! the model can follow.

use crate::traits::{LLMMessage, Role};

/// Rewrite tool calls and tool results as plain assistant and user text.
///
/// An assistant message keeps its text and gains one line per tool it called.
/// Tool results become user messages that name the call they answer;
/// consecutive results are merged into one message. Other messages are
/// returned unchanged.
pub(crate) fn tool_rounds_as_text(messages: &[LLMMessage]) -> Vec<LLMMessage> {
    let mut output: Vec<LLMMessage> = Vec::with_capacity(messages.len());
    let mut last_was_result = false;
    for message in messages {
        match message.role {
            Role::Assistant if !message.tool_calls.is_empty() => {
                let mut text = message.content.clone();
                for call in &message.tool_calls {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(&format!(
                        "[Called tool `{}` (call {}) with arguments {}]",
                        call.name, call.id, call.arguments
                    ));
                }
                output.push(LLMMessage::assistant(text));
                last_was_result = false;
            }
            Role::Tool => {
                let call = message.tool_call_id.as_deref().unwrap_or("unknown");
                let text = format!("[Result of tool call {call}]\n{}", message.content);
                match output.last_mut() {
                    Some(previous) if last_was_result => {
                        previous.content.push_str("\n\n");
                        previous.content.push_str(&text);
                    }
                    _ => output.push(LLMMessage::user(text)),
                }
                last_was_result = true;
            }
            _ => {
                output.push(message.clone());
                last_was_result = false;
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::ToolCall;

    #[test]
    fn tool_rounds_become_readable_text() {
        let history = [
            LLMMessage::system("Be brief."),
            LLMMessage::user("What is the weather in Paris and Rome?"),
            LLMMessage::assistant_with_tool_calls(
                "Let me check.",
                vec![
                    ToolCall::new("c1", "weather", serde_json::json!({ "city": "Paris" })),
                    ToolCall::new("c2", "weather", serde_json::json!({ "city": "Rome" })),
                ],
            ),
            LLMMessage::tool_result("c1", "12°C"),
            LLMMessage::tool_result("c2", "19°C"),
            LLMMessage::user("Thanks, and tomorrow?"),
        ];
        let text = tool_rounds_as_text(&history);

        // No message carries tool fields any more.
        assert!(text
            .iter()
            .all(|m| m.tool_calls.is_empty() && m.tool_call_id.is_none()));
        assert_eq!(
            text.iter().map(|m| m.role).collect::<Vec<_>>(),
            [
                Role::System,
                Role::User,
                Role::Assistant,
                Role::User,
                Role::User
            ]
        );
        // The assistant's requests are spelled out...
        assert!(text[2]
            .content
            .starts_with("Let me check.\n[Called tool `weather` (call c1)"));
        assert!(text[2].content.contains(r#"{"city":"Rome"}"#));
        // ...and both results sit in one message, each tied to its call.
        assert_eq!(
            text[3].content,
            "[Result of tool call c1]\n12°C\n\n[Result of tool call c2]\n19°C"
        );
        assert_eq!(text[4].content, "Thanks, and tomorrow?");
    }

    #[test]
    fn history_without_tools_is_unchanged() {
        let history = [LLMMessage::user("hi"), LLMMessage::assistant("hello")];
        let text = tool_rounds_as_text(&history);
        assert_eq!(text.len(), 2);
        assert_eq!(text[1].content, "hello");
    }
}
