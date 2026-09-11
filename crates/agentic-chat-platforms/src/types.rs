//! Common types for chat platforms.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ============================================================================
// PLATFORM ENUM
// ============================================================================

/// Supported chat platforms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    /// Telegram Bot API
    Telegram,
    /// Discord Bot
    Discord,
    /// Slack App
    Slack,
    /// WhatsApp via Twilio
    WhatsApp,
    /// Web chat interface
    Web,
    /// Direct API call
    Api,
}

impl Platform {
    /// Get platform name as string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Discord => "discord",
            Self::Slack => "slack",
            Self::WhatsApp => "whatsapp",
            Self::Web => "web",
            Self::Api => "api",
        }
    }
}

impl std::fmt::Display for Platform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ============================================================================
// INCOMING MESSAGE
// ============================================================================

/// A message received from a chat platform.
///
/// This is a unified format that normalizes messages from different platforms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IncomingMessage {
    /// Unique message ID (platform-specific).
    pub id: String,

    /// Source platform.
    pub platform: Platform,

    /// Channel/chat/conversation ID.
    pub channel_id: String,

    /// User ID of the sender.
    pub user_id: String,

    /// Username or display name (if available).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,

    /// Message content (text).
    pub content: String,

    /// When the message was sent.
    pub timestamp: DateTime<Utc>,

    /// Platform-specific metadata.
    #[serde(default)]
    pub metadata: serde_json::Value,

    /// File attachments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,

    /// ID of message being replied to (if any).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_id: Option<String>,

    /// Whether this is a direct message (vs group/channel).
    #[serde(default)]
    pub is_direct: bool,
}

impl IncomingMessage {
    /// Create a new incoming message.
    #[must_use]
    pub fn new(
        platform: Platform,
        channel_id: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            platform,
            channel_id: channel_id.into(),
            user_id: user_id.into(),
            username: None,
            content: content.into(),
            timestamp: Utc::now(),
            metadata: serde_json::Value::Null,
            attachments: vec![],
            reply_to_id: None,
            is_direct: false,
        }
    }

    /// Set username.
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Set message ID.
    #[must_use]
    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    /// Set timestamp.
    #[must_use]
    pub fn with_timestamp(mut self, timestamp: DateTime<Utc>) -> Self {
        self.timestamp = timestamp;
        self
    }

    /// Add metadata.
    #[must_use]
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Add attachment.
    #[must_use]
    pub fn with_attachment(mut self, attachment: Attachment) -> Self {
        self.attachments.push(attachment);
        self
    }

    /// Set as direct message.
    #[must_use]
    pub fn as_direct(mut self) -> Self {
        self.is_direct = true;
        self
    }

    /// Set reply to.
    #[must_use]
    pub fn replying_to(mut self, message_id: impl Into<String>) -> Self {
        self.reply_to_id = Some(message_id.into());
        self
    }

    /// Get a unique key for routing (platform:channel).
    #[must_use]
    pub fn routing_key(&self) -> String {
        format!("{}:{}", self.platform, self.channel_id)
    }
}

// ============================================================================
// OUTGOING MESSAGE
// ============================================================================

/// A message to send to a chat platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutgoingMessage {
    /// Target channel/chat ID.
    pub channel_id: String,

    /// Message content (text).
    pub content: String,

    /// Reply to a specific message (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,

    /// File attachments to send.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,

    /// Rich embeds (for Discord, Slack, etc.).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub embeds: Vec<Embed>,

    /// Message formatting.
    #[serde(default)]
    pub format: MessageFormat,

    /// Platform-specific options.
    #[serde(default)]
    pub options: serde_json::Value,
}

impl OutgoingMessage {
    /// Create a new outgoing message.
    #[must_use]
    pub fn new(channel_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            channel_id: channel_id.into(),
            content: content.into(),
            reply_to: None,
            attachments: vec![],
            embeds: vec![],
            format: MessageFormat::default(),
            options: serde_json::Value::Null,
        }
    }

    /// Set as reply to another message.
    #[must_use]
    pub fn reply_to(mut self, message_id: impl Into<String>) -> Self {
        self.reply_to = Some(message_id.into());
        self
    }

    /// Add attachment.
    #[must_use]
    pub fn with_attachment(mut self, attachment: Attachment) -> Self {
        self.attachments.push(attachment);
        self
    }

    /// Add an embed (for Discord, Slack).
    #[must_use]
    pub fn with_embed(mut self, embed: Embed) -> Self {
        self.embeds.push(embed);
        self
    }

    /// Set format.
    #[must_use]
    pub fn with_format(mut self, format: MessageFormat) -> Self {
        self.format = format;
        self
    }

    /// Set platform-specific options.
    #[must_use]
    pub fn with_options(mut self, options: serde_json::Value) -> Self {
        self.options = options;
        self
    }

    /// Create a reply to an incoming message.
    #[must_use]
    pub fn reply(incoming: &IncomingMessage, content: impl Into<String>) -> Self {
        Self::new(&incoming.channel_id, content).reply_to(&incoming.id)
    }
}

// ============================================================================
// ATTACHMENT
// ============================================================================

/// A file attachment in a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    /// Type of attachment.
    pub attachment_type: AttachmentType,

    /// URL to the file (if remote).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Raw file data (if local).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<u8>>,

    /// Filename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,

    /// MIME type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,

    /// File size in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

impl Attachment {
    /// Create an attachment from a URL.
    #[must_use]
    pub fn from_url(url: impl Into<String>, attachment_type: AttachmentType) -> Self {
        Self {
            attachment_type,
            url: Some(url.into()),
            data: None,
            filename: None,
            mime_type: None,
            size: None,
        }
    }

    /// Create an attachment from raw data.
    #[must_use]
    pub fn from_data(data: Vec<u8>, attachment_type: AttachmentType) -> Self {
        let size = data.len() as u64;
        Self {
            attachment_type,
            url: None,
            data: Some(data),
            filename: None,
            mime_type: None,
            size: Some(size),
        }
    }

    /// Set filename.
    #[must_use]
    pub fn with_filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    /// Set MIME type.
    #[must_use]
    pub fn with_mime_type(mut self, mime_type: impl Into<String>) -> Self {
        self.mime_type = Some(mime_type.into());
        self
    }
}

/// Type of attachment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentType {
    /// Image (PNG, JPG, etc.)
    Image,
    /// Document (PDF, DOC, etc.)
    Document,
    /// Audio file
    Audio,
    /// Video file
    Video,
    /// Other file type
    File,
}

// ============================================================================
// EMBED
// ============================================================================

/// A rich embed for platforms that support them (Discord, Slack).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Embed {
    /// Embed title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Embed description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// URL linked to the title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// Color (hex value like 0x00FF00).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,

    /// Thumbnail URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,

    /// Image URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,

    /// Footer text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<String>,

    /// Fields (name, value, inline).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<EmbedField>,
}

/// A field in an embed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedField {
    /// Field name.
    pub name: String,
    /// Field value.
    pub value: String,
    /// Display inline with other fields.
    #[serde(default)]
    pub inline: bool,
}

impl Embed {
    /// Create a new embed with a title.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: Some(title.into()),
            ..Default::default()
        }
    }

    /// Set description.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Set URL (makes title clickable).
    #[must_use]
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Set color.
    #[must_use]
    pub fn with_color(mut self, color: u32) -> Self {
        self.color = Some(color);
        self
    }

    /// Set thumbnail.
    #[must_use]
    pub fn with_thumbnail(mut self, url: impl Into<String>) -> Self {
        self.thumbnail = Some(url.into());
        self
    }

    /// Set image.
    #[must_use]
    pub fn with_image(mut self, url: impl Into<String>) -> Self {
        self.image = Some(url.into());
        self
    }

    /// Set footer.
    #[must_use]
    pub fn with_footer(mut self, footer: impl Into<String>) -> Self {
        self.footer = Some(footer.into());
        self
    }

    /// Add a field.
    #[must_use]
    pub fn with_field(
        mut self,
        name: impl Into<String>,
        value: impl Into<String>,
        inline: bool,
    ) -> Self {
        self.fields.push(EmbedField {
            name: name.into(),
            value: value.into(),
            inline,
        });
        self
    }
}

// ============================================================================
// MESSAGE FORMAT
// ============================================================================

/// How to format message content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageFormat {
    /// Plain text, no formatting.
    #[default]
    Plain,
    /// Markdown formatting.
    Markdown,
    /// HTML formatting (platform-dependent).
    Html,
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_incoming_message_builder() {
        let msg = IncomingMessage::new(Platform::Telegram, "chat123", "user456", "Hello!")
            .with_username("testuser")
            .as_direct();

        assert_eq!(msg.platform, Platform::Telegram);
        assert_eq!(msg.channel_id, "chat123");
        assert_eq!(msg.user_id, "user456");
        assert_eq!(msg.content, "Hello!");
        assert_eq!(msg.username, Some("testuser".to_string()));
        assert!(msg.is_direct);
    }

    #[test]
    fn test_outgoing_message_reply() {
        let incoming = IncomingMessage::new(Platform::Discord, "channel1", "user1", "Question?")
            .with_id("msg123");

        let reply = OutgoingMessage::reply(&incoming, "Answer!");

        assert_eq!(reply.channel_id, "channel1");
        assert_eq!(reply.content, "Answer!");
        assert_eq!(reply.reply_to, Some("msg123".to_string()));
    }

    #[test]
    fn test_routing_key() {
        let msg = IncomingMessage::new(Platform::Slack, "C12345", "U67890", "Test");
        assert_eq!(msg.routing_key(), "slack:C12345");
    }

    #[test]
    fn test_attachment_from_url() {
        let attachment =
            Attachment::from_url("https://example.com/image.png", AttachmentType::Image)
                .with_filename("image.png")
                .with_mime_type("image/png");

        assert_eq!(
            attachment.url,
            Some("https://example.com/image.png".to_string())
        );
        assert_eq!(attachment.filename, Some("image.png".to_string()));
        assert_eq!(attachment.mime_type, Some("image/png".to_string()));
    }

    #[test]
    fn test_platform_display() {
        assert_eq!(Platform::Telegram.as_str(), "telegram");
        assert_eq!(Platform::Discord.to_string(), "discord");
    }

    #[test]
    fn test_message_serialization() {
        let msg = IncomingMessage::new(Platform::Telegram, "chat1", "user1", "Test message");
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: IncomingMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.platform, Platform::Telegram);
        assert_eq!(parsed.content, "Test message");
    }
}
