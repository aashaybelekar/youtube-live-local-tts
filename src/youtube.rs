use std::collections::HashMap;
use std::time::Instant;
use chrono::{DateTime, Local, Utc};
use youtube_chat::item::{ChatItem, MessageItem};
use crate::config::{
    is_valid_message, format_tts_message, AppConfig, ChatMessage
};

/// Extracts only text parts from a vector of MessageItems (ignoring pure emojis/stickers).
pub fn extract_text_from_items(messages: &[MessageItem]) -> String {
    let mut parts = Vec::new();
    for msg in messages {
        if let MessageItem::Text(ref text) = msg {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed);
            }
        }
    }
    parts.join(" ")
}

/// Evaluates a ChatItem against config rules (prefix, cooldown, max length, and bot start time).
/// Returns a tuple of:
///   1. ChatMessage representation for the UI history / websocket broadcast.
///   2. Option<String> containing the formatted speech string if it should be spoken.
pub fn process_chat_item(
    item: &ChatItem,
    config: &AppConfig,
    cooldowns: &mut HashMap<String, Instant>,
    bot_start_time: Option<DateTime<Utc>>,
) -> (ChatMessage, Option<String>) {
    let author_name = item
        .author
        .name
        .clone()
        .unwrap_or_else(|| "Anonymous".to_string());
    let author_id = &item.author.channel_id;
    let full_text = extract_text_from_items(&item.message);

    let time_str = item
        .timestamp
        .map(|dt| dt.with_timezone(&Local).format("%H:%M:%S").to_string())
        .unwrap_or_else(|| Local::now().format("%H:%M:%S").to_string());

    // Ignore old messages sent before the bot was started
    let is_old_message = match (item.timestamp, bot_start_time) {
        (Some(item_time), Some(start_time)) => item_time < start_time,
        _ => false,
    };

    if is_old_message {
        let chat_msg = ChatMessage {
            author: author_name,
            message: full_text,
            timestamp: time_str,
            was_read: false,
            is_owner: item.is_owner,
            is_moderator: item.is_moderator,
        };
        return (chat_msg, None);
    }

    let cooldown_duration = std::time::Duration::from_secs(config.cooldown_seconds);

    let passes_filters = is_valid_message(
        &full_text,
        author_id,
        &config.tts_prefix,
        config.max_message_length,
        cooldowns,
        cooldown_duration,
    );

    let (was_read, speech_text) = if passes_filters {
        // Strip prefix if configured
        let clean_text = if !config.tts_prefix.is_empty()
            && full_text.to_lowercase().starts_with(&config.tts_prefix.to_lowercase())
        {
            full_text[config.tts_prefix.len()..].trim().to_string()
        } else {
            full_text.clone()
        };

        if !clean_text.is_empty() {
            // Update user cooldown
            cooldowns.insert(author_id.clone(), Instant::now());
            let formatted = format_tts_message(&config.tts_template, &author_name, &clean_text);
            (true, Some(formatted))
        } else {
            (false, None)
        }
    } else {
        (false, None)
    };

    let chat_msg = ChatMessage {
        author: author_name,
        message: full_text,
        timestamp: time_str,
        was_read,
        is_owner: item.is_owner,
        is_moderator: item.is_moderator,
    };

    (chat_msg, speech_text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use youtube_chat::item::{Author, EmojiItem};

    fn make_test_chat_item_with_time(
        channel_id: &str,
        author_name: &str,
        text: &str,
        time: Option<DateTime<Utc>>,
    ) -> ChatItem {
        ChatItem {
            id: "msg1".to_string(),
            author: Author {
                name: Some(author_name.to_string()),
                thumbnail: None,
                channel_id: channel_id.to_string(),
                badge: None,
            },
            message: vec![MessageItem::Text(text.to_string())],
            superchat: None,
            is_membership: false,
            is_verified: false,
            is_owner: false,
            is_moderator: false,
            timestamp: time,
        }
    }

    fn make_test_chat_item(channel_id: &str, author_name: &str, text: &str) -> ChatItem {
        make_test_chat_item_with_time(channel_id, author_name, text, None)
    }

    #[test]
    fn test_extract_text_from_mixed_items() {
        let items = vec![
            MessageItem::Text("Hello".to_string()),
            MessageItem::Emoji(EmojiItem {
                image_item: None,
                emoji_text: Some(":wave:".to_string()),
                is_custome_emoji: Some(false),
            }),
            MessageItem::Text("streamer!".to_string()),
        ];

        let extracted = extract_text_from_items(&items);
        assert_eq!(extracted, "Hello streamer!");
    }

    #[test]
    fn test_process_chat_item_passes_and_formats() {
        let mut cooldowns = HashMap::new();
        let config = AppConfig {
            video_id: "test".to_string(),
            voice_id: "af_bella".to_string(),
            tts_prefix: "!say".to_string(),
            speed: 1.0,
            volume: 1.0,
            cooldown_seconds: 10,
            max_message_length: 100,
            tts_template: "{author}: {message}".to_string(),
            device: "auto".to_string(),
        };

        let item = make_test_chat_item("c1", "Dave", "!say Loving the stream!");
        let (chat_msg, speech) = process_chat_item(&item, &config, &mut cooldowns, None);

        assert!(chat_msg.was_read);
        assert_eq!(chat_msg.author, "Dave");
        assert_eq!(speech, Some("Dave: Loving the stream!".to_string()));
        assert!(cooldowns.contains_key("c1"));
    }

    #[test]
    fn test_process_chat_item_blocked_by_cooldown() {
        let mut cooldowns = HashMap::new();
        let config = AppConfig::default();

        let item = make_test_chat_item("c2", "Bob", "First message");
        let (_, speech1) = process_chat_item(&item, &config, &mut cooldowns, None);
        assert!(speech1.is_some());

        // Immediately try second message
        let item2 = make_test_chat_item("c2", "Bob", "Second message immediately");
        let (chat_msg2, speech2) = process_chat_item(&item2, &config, &mut cooldowns, None);
        assert!(!chat_msg2.was_read);
        assert!(speech2.is_none());
    }

    #[test]
    fn test_process_chat_item_ignores_old_messages() {
        let mut cooldowns = HashMap::new();
        let config = AppConfig::default();

        let bot_start = Utc::now();
        let old_time = bot_start - ChronoDuration::seconds(60); // 1 minute ago

        let item = make_test_chat_item_with_time("c3", "Eve", "Old historical chat message", Some(old_time));
        let (chat_msg, speech) = process_chat_item(&item, &config, &mut cooldowns, Some(bot_start));

        assert!(!chat_msg.was_read, "Old message was_read must be false");
        assert!(speech.is_none(), "Old message must NOT produce speech output");
        assert!(!cooldowns.contains_key("c3"), "Old message should not trigger cooldowns");
    }

    #[test]
    fn test_process_chat_item_accepts_new_messages_after_start() {
        let mut cooldowns = HashMap::new();
        let config = AppConfig::default();

        let bot_start = Utc::now() - ChronoDuration::seconds(5);
        let new_time = Utc::now(); // After bot start

        let item = make_test_chat_item_with_time("c4", "Eve", "New live chat message", Some(new_time));
        let (chat_msg, speech) = process_chat_item(&item, &config, &mut cooldowns, Some(bot_start));

        assert!(chat_msg.was_read, "New message was_read should be true");
        assert!(speech.is_some(), "New message should produce speech output");
    }
}
