//! Telegram Bot implementation.
//!
//! Uses the teloxide library to interact with the Telegram Bot API.
//!
//! # Example
//!
//! ```rust,ignore
//! use dasein_agentic_chat_platforms::{TelegramBot, ChatPlatform};
//!
//! let bot = TelegramBot::new("BOT_TOKEN");
//! bot.start().await?;
//!
//! let mut rx = bot.subscribe();
//! while let Ok(msg) = rx.recv().await {
//!     println!("Received from {}: {}", msg.user_id, msg.content);
//! }
//! ```

#[cfg(feature = "telegram")]
mod telegram_impl {
    use async_trait::async_trait;
    use std::sync::Arc;
    use tokio::sync::{broadcast, RwLock};
    use tracing::{debug, info, warn};

    use teloxide::{
        prelude::*,
        types::{
            InputFile, MediaKind, Message as TgMessage, MessageId, MessageKind, ParseMode,
            ReplyParameters,
        },
    };

    use crate::{
        error::{ChatError, ChatResult},
        traits::{ChatPlatform, PlatformConfig},
        types::{
            Attachment, AttachmentType, IncomingMessage, MessageFormat, OutgoingMessage, Platform,
        },
    };

    /// Telegram bot implementation.
    pub struct TelegramBot {
        bot: Bot,
        config: PlatformConfig,
        tx: broadcast::Sender<IncomingMessage>,
        running: Arc<RwLock<bool>>,
        bot_user_id: Arc<RwLock<Option<String>>>,
    }

    impl TelegramBot {
        /// Create a new Telegram bot with the given token.
        #[must_use]
        pub fn new(token: impl Into<String>) -> Self {
            let token = token.into();
            let bot = Bot::new(&token);
            let (tx, _) = broadcast::channel(1000);

            Self {
                bot,
                config: PlatformConfig::new(Platform::Telegram, token),
                tx,
                running: Arc::new(RwLock::new(false)),
                bot_user_id: Arc::new(RwLock::new(None)),
            }
        }

        /// Effective platform configuration.
        pub fn config(&self) -> &PlatformConfig {
            &self.config
        }

        /// Create from a platform config.
        #[must_use]
        pub fn from_config(config: PlatformConfig) -> Self {
            let bot = Bot::new(&config.token);
            let (tx, _) = broadcast::channel(config.channel_buffer_size);

            Self {
                bot,
                config,
                tx,
                running: Arc::new(RwLock::new(false)),
                bot_user_id: Arc::new(RwLock::new(None)),
            }
        }

        /// Convert a Telegram message to our unified format.
        fn convert_message(msg: &TgMessage) -> Option<IncomingMessage> {
            // Extract text content
            let content = match &msg.kind {
                MessageKind::Common(common) => match &common.media_kind {
                    MediaKind::Text(text) => text.text.clone(),
                    _ => return None, // Skip non-text messages for now
                },
                _ => return None,
            };

            let user = msg.from.as_ref()?;
            let chat = &msg.chat;

            let mut incoming = IncomingMessage::new(
                Platform::Telegram,
                chat.id.0.to_string(),
                user.id.0.to_string(),
                content,
            )
            .with_id(msg.id.0.to_string());

            // Set username if available
            if let Some(username) = &user.username {
                incoming = incoming.with_username(username.clone());
            } else if let Some(first_name) = Some(&user.first_name) {
                incoming = incoming.with_username(first_name.clone());
            }

            // Check if it's a private chat (DM)
            if chat.is_private() {
                incoming = incoming.as_direct();
            }

            // Set reply_to if this is a reply
            if let Some(reply_to) = &msg.reply_to_message() {
                incoming = incoming.replying_to(reply_to.id.0.to_string());
            }

            // Add metadata
            let metadata = serde_json::json!({
                "chat_type": if chat.is_private() { "private" } else if chat.is_group() { "group" } else { "channel" },
                "chat_title": chat.title(),
            });
            incoming = incoming.with_metadata(metadata);

            Some(incoming)
        }

        /// Get the underlying teloxide bot.
        pub fn inner(&self) -> &Bot {
            &self.bot
        }
    }

    #[async_trait]
    impl ChatPlatform for TelegramBot {
        fn platform(&self) -> Platform {
            Platform::Telegram
        }

        async fn start(&self) -> ChatResult<()> {
            // Check if already running
            {
                let running = self.running.read().await;
                if *running {
                    return Ok(());
                }
            }

            // Get bot info
            let me = self
                .bot
                .get_me()
                .await
                .map_err(|e| ChatError::AuthenticationFailed(e.to_string()))?;

            let bot_id = me.id.0.to_string();
            let bot_username = me.username.clone();

            info!(
                bot_id = %bot_id,
                bot_username = ?bot_username,
                "Telegram bot authenticated"
            );

            // Store bot user ID
            {
                let mut id = self.bot_user_id.write().await;
                *id = Some(bot_id);
            }

            // Mark as running
            {
                let mut running = self.running.write().await;
                *running = true;
            }

            // Start the message handler
            let tx = self.tx.clone();
            let bot = self.bot.clone();
            let running = self.running.clone();

            tokio::spawn(async move {
                info!("Starting Telegram message listener");

                let handler =
                    Update::filter_message().endpoint(move |_bot: Bot, msg: TgMessage| {
                        let tx = tx.clone();
                        async move {
                            if let Some(incoming) = TelegramBot::convert_message(&msg) {
                                debug!(
                                    message_id = %incoming.id,
                                    channel_id = %incoming.channel_id,
                                    "Received Telegram message"
                                );

                                if tx.send(incoming).is_err() {
                                    warn!("No subscribers for Telegram messages");
                                }
                            }
                            ResponseResult::<()>::Ok(())
                        }
                    });

                let mut dispatcher = Dispatcher::builder(bot, handler)
                    .enable_ctrlc_handler()
                    .build();

                // Run until stopped
                loop {
                    {
                        let is_running = running.read().await;
                        if !*is_running {
                            info!("Telegram bot stopping");
                            break;
                        }
                    }

                    // Process updates
                    Box::pin(dispatcher.dispatch()).await;
                }
            });

            Ok(())
        }

        async fn stop(&self) -> ChatResult<()> {
            let mut running = self.running.write().await;
            *running = false;
            info!("Telegram bot stopped");
            Ok(())
        }

        async fn send(&self, message: OutgoingMessage) -> ChatResult<String> {
            let chat_id: i64 = message
                .channel_id
                .parse()
                .map_err(|_| ChatError::InvalidChannelId(message.channel_id.clone()))?;

            let chat_id = ChatId(chat_id);

            // Determine parse mode
            let parse_mode = match message.format {
                MessageFormat::Markdown => Some(ParseMode::MarkdownV2),
                MessageFormat::Html => Some(ParseMode::Html),
                MessageFormat::Plain => None,
            };

            // Send the message
            let mut request = self.bot.send_message(chat_id, &message.content);

            if let Some(mode) = parse_mode {
                request = request.parse_mode(mode);
            }

            if let Some(reply_to) = &message.reply_to {
                if let Ok(msg_id) = reply_to.parse::<i32>() {
                    request = request.reply_parameters(ReplyParameters::new(MessageId(msg_id)));
                }
            }

            let sent = request
                .await
                .map_err(|e| ChatError::SendFailed(e.to_string()))?;

            let message_id = sent.id.0.to_string();

            debug!(
                message_id = %message_id,
                channel_id = %message.channel_id,
                "Sent Telegram message"
            );

            // Send attachments if any
            for attachment in &message.attachments {
                self.send_attachment(chat_id, attachment).await?;
            }

            Ok(message_id)
        }

        fn subscribe(&self) -> broadcast::Receiver<IncomingMessage> {
            self.tx.subscribe()
        }

        async fn is_healthy(&self) -> bool {
            self.bot.get_me().await.is_ok()
        }

        fn bot_user_id(&self) -> Option<&str> {
            // Note: This requires a different approach since we store in RwLock
            // For now, return None - could be improved with interior mutability
            None
        }
    }

    impl TelegramBot {
        /// Send an attachment.
        async fn send_attachment(
            &self,
            chat_id: ChatId,
            attachment: &Attachment,
        ) -> ChatResult<()> {
            let input_file = if let Some(url) = &attachment.url {
                InputFile::url(
                    url.parse()
                        .map_err(|_| ChatError::SendFailed(format!("Invalid URL: {}", url)))?,
                )
            } else if let Some(data) = &attachment.data {
                let filename = attachment
                    .filename
                    .clone()
                    .unwrap_or_else(|| "file".to_string());
                InputFile::memory(data.clone()).file_name(filename)
            } else {
                return Err(ChatError::SendFailed(
                    "Attachment has no URL or data".into(),
                ));
            };

            match attachment.attachment_type {
                AttachmentType::Image => {
                    self.bot
                        .send_photo(chat_id, input_file)
                        .await
                        .map_err(|e| ChatError::SendFailed(e.to_string()))?;
                }
                AttachmentType::Document | AttachmentType::File => {
                    self.bot
                        .send_document(chat_id, input_file)
                        .await
                        .map_err(|e| ChatError::SendFailed(e.to_string()))?;
                }
                AttachmentType::Audio => {
                    self.bot
                        .send_audio(chat_id, input_file)
                        .await
                        .map_err(|e| ChatError::SendFailed(e.to_string()))?;
                }
                AttachmentType::Video => {
                    self.bot
                        .send_video(chat_id, input_file)
                        .await
                        .map_err(|e| ChatError::SendFailed(e.to_string()))?;
                }
            }

            Ok(())
        }
    }
}

#[cfg(feature = "telegram")]
pub use telegram_impl::TelegramBot;
