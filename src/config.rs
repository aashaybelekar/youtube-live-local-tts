use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use regex::Regex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    pub video_id: String,
    pub voice_id: String,
    pub tts_prefix: String,
    pub speed: f32,
    pub volume: f32,
    pub cooldown_seconds: u64,
    pub max_message_length: usize,
    pub tts_template: String,
    pub device: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            video_id: String::new(),
            voice_id: "af_bella".to_string(),
            tts_prefix: String::new(),
            speed: 1.0,
            volume: 1.0,
            cooldown_seconds: 10,
            max_message_length: 150,
            tts_template: "{author} says: {message}".to_string(),
            device: "auto".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub author: String,
    pub message: String,
    pub timestamp: String,
    pub was_read: bool,
    pub is_owner: bool,
    pub is_moderator: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BotStatus {
    pub running: bool,
    pub connected: bool,
    pub messages_read: usize,
    pub queue_size: usize,
    pub backend: String,
}

/// Extracts an 11-character YouTube video ID from a URL or returns the input if already an ID.
pub fn extract_video_id(url_or_id: &str) -> String {
    let trimmed = url_or_id.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let patterns = [
        r"(?:youtube\.com/live/)([a-zA-Z0-9_-]{11})",
        r"(?:youtube\.com/watch\?v=)([a-zA-Z0-9_-]{11})",
        r"(?:youtu\.be/)([a-zA-Z0-9_-]{11})",
    ];

    for pattern in patterns {
        if let Ok(re) = Regex::new(pattern) {
            if let Some(caps) = re.captures(trimmed) {
                if let Some(matched) = caps.get(1) {
                    return matched.as_str().to_string();
                }
            }
        }
    }

    trimmed.to_string()
}

/// Checks if an author is currently in the cooldown period.
pub fn is_on_cooldown(
    author_id: &str,
    cooldowns: &HashMap<String, Instant>,
    cooldown_duration: Duration,
) -> bool {
    if let Some(&last_time) = cooldowns.get(author_id) {
        last_time.elapsed() < cooldown_duration
    } else {
        false
    }
}

/// Validates whether a chat message passes all filters (prefix, length, cooldown).
pub fn is_valid_message(
    text: &str,
    author_id: &str,
    prefix: &str,
    max_length: usize,
    cooldowns: &HashMap<String, Instant>,
    cooldown_duration: Duration,
) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Prefix check
    let content = if !prefix.is_empty() {
        if !trimmed.to_lowercase().starts_with(&prefix.to_lowercase()) {
            return false;
        }
        trimmed[prefix.len()..].trim()
    } else {
        trimmed
    };

    if content.is_empty() || content.len() > max_length {
        return false;
    }

    if is_on_cooldown(author_id, cooldowns, cooldown_duration) {
        return false;
    }

    true
}

/// Formats text using the template tokens `{author}` and `{message}`.
pub fn format_tts_message(template: &str, author: &str, message: &str) -> String {
    template
        .replace("{author}", author)
        .replace("{message}", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_config_deserialization_with_missing_fields() {
        let partial_json = r#"{
            "video_id": "test1234567"
        }"#;
        let config: Result<AppConfig, _> = serde_json::from_str(partial_json);
        assert!(config.is_ok(), "Config should deserialize with default values for missing fields");
        let cfg = config.unwrap();
        assert_eq!(cfg.video_id, "test1234567");
        assert_eq!(cfg.device, "auto");
        assert_eq!(cfg.voice_id, "af_bella");
        assert_eq!(cfg.speed, 1.0);
    }

    #[test]
    fn test_extract_video_id_formats() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            "dQw4w9WgXcQ"
        );
        assert_eq!(
            extract_video_id("https://youtube.com/live/dQw4w9WgXcQ?feature=share"),
            "dQw4w9WgXcQ"
        );
        assert_eq!(
            extract_video_id("https://youtu.be/dQw4w9WgXcQ"),
            "dQw4w9WgXcQ"
        );
        assert_eq!(extract_video_id("dQw4w9WgXcQ"), "dQw4w9WgXcQ");
        assert_eq!(extract_video_id(""), "");
    }

    #[test]
    fn test_cooldown_tracking() {
        let mut cooldowns = HashMap::new();
        let user = "user123";
        let duration = Duration::from_secs(5);

        assert!(!is_on_cooldown(user, &cooldowns, duration));

        cooldowns.insert(user.to_string(), Instant::now());
        assert!(is_on_cooldown(user, &cooldowns, duration));

        // Expired cooldown simulation
        let expired_time = Instant::now() - Duration::from_secs(10);
        cooldowns.insert(user.to_string(), expired_time);
        assert!(!is_on_cooldown(user, &cooldowns, duration));
    }

    #[test]
    fn test_is_valid_message_prefix() {
        let cooldowns = HashMap::new();
        let duration = Duration::from_secs(5);

        // Required prefix "!tts"
        assert!(is_valid_message(
            "!tts hello world",
            "u1",
            "!tts",
            100,
            &cooldowns,
            duration
        ));
        assert!(is_valid_message(
            "!TTS uppercase prefix",
            "u1",
            "!tts",
            100,
            &cooldowns,
            duration
        ));
        assert!(!is_valid_message(
            "hello without prefix",
            "u1",
            "!tts",
            100,
            &cooldowns,
            duration
        ));
        assert!(!is_valid_message(
            "!tts",
            "u1",
            "!tts",
            100,
            &cooldowns,
            duration
        )); // empty after prefix

        // No prefix required
        assert!(is_valid_message(
            "plain message",
            "u1",
            "",
            100,
            &cooldowns,
            duration
        ));
    }

    #[test]
    fn test_is_valid_message_length_limit() {
        let cooldowns = HashMap::new();
        let duration = Duration::from_secs(5);

        assert!(is_valid_message(
            "short message",
            "u1",
            "",
            20,
            &cooldowns,
            duration
        ));
        assert!(!is_valid_message(
            "this message is way too long for the limit",
            "u1",
            "",
            10,
            &cooldowns,
            duration
        ));
    }

    #[test]
    fn test_format_tts_message() {
        let template = "{author} says: {message}";
        let formatted = format_tts_message(template, "Alice", "Good morning!");
        assert_eq!(formatted, "Alice says: Good morning!");

        let custom_template = "[{author}]: {message}";
        assert_eq!(
            format_tts_message(custom_template, "Bob", "Testing 123"),
            "[Bob]: Testing 123"
        );
    }
}
