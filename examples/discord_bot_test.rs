//! Simple Discord bot test
//!
//! Run with:
//! ```bash
//! export DISCORD_TOKEN="your_token"
//! cargo run --example discord_bot_test --features discord
//! ```

use std::sync::Arc;

use dasein_agentic_chat_platforms::{ChatPlatform, DiscordBot, OutgoingMessage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing for logs
    tracing_subscriber::fmt().with_env_filter("info").init();

    println!("=================================");
    println!("   Discord Bot Test");
    println!("=================================\n");

    // Get token from environment
    let token = std::env::var("DISCORD_TOKEN").expect("DISCORD_TOKEN environment variable not set");

    println!("Token loaded (length: {} chars)", token.len());

    // Create bot
    let bot = Arc::new(DiscordBot::new(token));

    println!("Starting bot...");
    bot.start().await?;

    println!("Bot started! Listening for messages...");
    println!("Send a message in a channel where the bot is present.");
    println!("Press Ctrl+C to stop.\n");

    // Subscribe to messages
    let mut rx = bot.subscribe();

    // Handle messages
    while let Ok(msg) = rx.recv().await {
        println!(
            "[{}] {}: {}",
            msg.channel_id,
            msg.username.as_deref().unwrap_or("unknown"),
            msg.content
        );

        // Reply to "ping" messages
        if msg.content.to_lowercase().contains("ping") {
            let reply = OutgoingMessage::reply(&msg, "Pong! 🏓");

            match bot.send(reply).await {
                Ok(id) => println!("  -> Replied with message ID: {}", id),
                Err(e) => println!("  -> Failed to reply: {}", e),
            }
        }

        // Reply to "hello" messages
        if msg.content.to_lowercase().contains("hello")
            || msg.content.to_lowercase().contains("salut")
        {
            let reply = OutgoingMessage::new(
                msg.channel_id.clone(),
                format!(
                    "Salut {} ! Je suis un bot Agentic-RS 🤖",
                    msg.username.as_deref().unwrap_or("")
                ),
            );

            match bot.send(reply).await {
                Ok(id) => println!("  -> Replied with message ID: {}", id),
                Err(e) => println!("  -> Failed to reply: {}", e),
            }
        }
    }

    Ok(())
}
