//! Multi-platform chat bot demo
//!
//! This example demonstrates how to use the chat-platforms crate to:
//! - Create bots for Telegram, Discord, and Slack
//! - Route messages to different agents based on channel/content
//! - Send replies back through the MessageRouter
//!
//! Run with:
//! ```bash
//! TELEGRAM_TOKEN=xxx DISCORD_TOKEN=xxx cargo run --example chat_platforms_demo --features chat-platforms
//! ```

fn main() {
    println!("=================================");
    println!("   Chat Platforms Demo");
    println!("=================================\n");

    // Show available platform implementations
    println!("Available platform implementations:");
    println!("  - Telegram (via teloxide)");
    println!("  - Discord (via serenity)");
    println!("  - Slack (via slack-morphism)");

    println!("\n---------------------------------");
    println!("Example code structure:");
    println!("---------------------------------\n");

    print_example_code();

    println!("\n---------------------------------");
    println!("To run a real bot, set tokens:");
    println!("  TELEGRAM_TOKEN=your_token");
    println!("  DISCORD_TOKEN=your_token");
    println!("  SLACK_BOT_TOKEN=xoxb-xxx");
    println!("  SLACK_APP_TOKEN=xapp-xxx");
    println!("---------------------------------");
}

fn print_example_code() {
    let code = r#"
use dasein_agentic_chat_platforms::{
    MessageRouter, RouteConfig, Platform,
    TelegramBot, DiscordBot, SlackBot,
    ChatPlatform, IncomingMessage, OutgoingMessage,
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create the message router
    let router = MessageRouter::new();

    // Create platform bots (feature-gated)
    #[cfg(feature = "telegram")]
    {
        let telegram = Arc::new(TelegramBot::new(
            std::env::var("TELEGRAM_TOKEN")?
        ));
        router.register_platform(telegram).await;
    }

    #[cfg(feature = "discord")]
    {
        let discord = Arc::new(DiscordBot::new(
            std::env::var("DISCORD_TOKEN")?
        ));
        router.register_platform(discord).await;
    }

    // Configure routing rules
    // Route VIP channel to premium agent
    router.set_route(
        "telegram:VIP_CHAT_ID",
        RouteConfig::new("premium-agent", "tenant-vip")
    ).await;

    // Route support channels to support agent
    router.set_route(
        "discord:SUPPORT_CHANNEL",
        RouteConfig::new("support-agent", "tenant-support")
    ).await;

    // Default route for everything else
    router.set_route(
        "*:*",
        RouteConfig::new("default-agent", "default")
    ).await;

    // Start all registered platforms
    router.start_all().await?;

    // Subscribe to incoming messages
    let mut rx = router.subscribe();

    println!("Bot is running! Press Ctrl+C to stop.");

    while let Ok(msg) = rx.recv().await {
        println!(
            "[{}] From {}: {}",
            msg.platform,
            msg.username.as_deref().unwrap_or("unknown"),
            msg.content
        );

        // Route message to appropriate agent
        if let Some(config) = router.route(&msg).await {
            println!(
                "  -> Routing to agent: {} (tenant: {})",
                config.agent_id,
                config.tenant_id
            );

            // Here you would send to your agent system
            // let response = agent_system.process(&msg, &config).await?;

            // For demo, just echo back
            let reply = format!(
                "Received your message! Routed to agent: {}",
                config.agent_id
            );

            router.reply(&msg, &reply).await?;
        }
    }

    Ok(())
}
"#;

    println!("{}", code);
}
