//! Webhook transport for workflow sharing and notifications.
//!
//! Shells out to `curl` — the same zero-dependency HTTP approach as
//! `infrastructure::update` — with stdin closed and a hard timeout, so a
//! webhook call can never park the app on a prompt or a dead connection.
//! The payload shaping is pure and unit-tested; the send is a thin wrapper.
//!
//! Nothing here decides *when* to post: sharing is an explicit user action
//! with a preview, notifications are the `workflows.notify_webhook` opt-in.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

/// One POST, small payload — anything past this is a dead connection.
const SEND_TIMEOUT: Duration = Duration::from_secs(15);

/// Discord rejects `content` over 2000 characters outright.
const DISCORD_MAX_CHARS: usize = 2000;

/// Slack truncates around 40k; stay under it so the tail is ours to write.
const SLACK_MAX_CHARS: usize = 39_000;

/// Where a payload goes. The two services take the same shape with a
/// different key (`text` vs `content`), so one enum covers both.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebhookKind {
    Slack,
    Discord,
}

impl WebhookKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "slack" => Some(Self::Slack),
            "discord" => Some(Self::Discord),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Slack => "slack",
            Self::Discord => "discord",
        }
    }
}

/// Pure: the JSON body for one webhook message, truncated to the service's
/// hard limit (on a char boundary, with an explicit marker — a silently cut
/// message reads as complete). Returns the body and whether it was truncated.
pub fn payload(kind: WebhookKind, text: &str) -> (String, bool) {
    let (max, key) = match kind {
        WebhookKind::Slack => (SLACK_MAX_CHARS, "text"),
        WebhookKind::Discord => (DISCORD_MAX_CHARS, "content"),
    };
    let marker = "\n… (truncated)";
    let truncated = text.chars().count() > max;
    let body = if truncated {
        let keep: String = text.chars().take(max - marker.chars().count()).collect();
        format!("{}{}", keep, marker)
    } else {
        text.to_string()
    };
    let json = serde_json::json!({ key: body });
    (json.to_string(), truncated)
}

/// Pure: is this a URL a webhook may be sent to? Only absolute http(s) —
/// anything else (a file path, a pasted hostname) fails before curl runs.
pub fn valid_url(url: &str) -> bool {
    let u = url.trim();
    u.starts_with("https://") || u.starts_with("http://")
}

/// POST `text` to the webhook. Blocking (callers wrap in `spawn_blocking`).
/// Returns whether the message was truncated to fit the service's limit.
pub fn send(kind: WebhookKind, url: &str, text: &str) -> Result<bool, String> {
    let url = url.trim();
    if !valid_url(url) {
        return Err(format!("not an http(s) webhook URL: {:?}", url));
    }
    let (body, truncated) = payload(kind, text);

    // The body travels through a temp file, not argv: a full share document
    // can exceed platform argv limits (the `gh pr comment` precedent).
    let tmp = std::env::temp_dir().join(format!(
        "clash-webhook-{}-{}.json",
        std::process::id(),
        kind.as_str()
    ));
    std::fs::write(&tmp, &body).map_err(|e| e.to_string())?;

    let result = run_curl(url, &tmp);
    let _ = std::fs::remove_file(&tmp);
    result.map(|_| truncated)
}

fn run_curl(url: &str, body_file: &std::path::Path) -> Result<(), String> {
    let mut child = Command::new("curl")
        .arg("-sS")
        .arg("--max-time")
        .arg(SEND_TIMEOUT.as_secs().to_string())
        .arg("--fail")
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("--data")
        .arg(format!("@{}", body_file.display()))
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run curl: {}", e))?;

    let mut stderr = String::new();
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    // curl's own --max-time bounds the call; wait() cannot hang past it.
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("webhook POST failed: {}", stderr.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_parse_leniently() {
        assert_eq!(WebhookKind::parse("slack"), Some(WebhookKind::Slack));
        assert_eq!(WebhookKind::parse(" Discord "), Some(WebhookKind::Discord));
        assert_eq!(WebhookKind::parse("teams"), None);
        assert_eq!(WebhookKind::parse(""), None);
        assert_eq!(WebhookKind::Slack.as_str(), "slack");
        assert_eq!(WebhookKind::Discord.as_str(), "discord");
    }

    #[test]
    fn payload_uses_the_service_key() {
        let (slack, t) = payload(WebhookKind::Slack, "hello");
        assert_eq!(slack, r#"{"text":"hello"}"#);
        assert!(!t);
        let (discord, t) = payload(WebhookKind::Discord, "hello");
        assert_eq!(discord, r#"{"content":"hello"}"#);
        assert!(!t);
    }

    #[test]
    fn payload_escapes_json_meta_characters() {
        let (json, _) = payload(WebhookKind::Slack, "a \"quote\"\nand a line");
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["text"], "a \"quote\"\nand a line");
    }

    #[test]
    fn payload_truncates_at_the_service_limit_with_a_marker() {
        let long = "x".repeat(3000);
        let (json, truncated) = payload(WebhookKind::Discord, &long);
        assert!(truncated);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let content = parsed["content"].as_str().unwrap();
        assert!(content.chars().count() <= DISCORD_MAX_CHARS);
        assert!(content.ends_with("… (truncated)"));
        // Slack's limit is far higher — the same text passes untouched.
        let (_, truncated) = payload(WebhookKind::Slack, &long);
        assert!(!truncated);
    }

    #[test]
    fn payload_truncation_respects_multibyte_boundaries() {
        // char-based counting: a multibyte-heavy message must not be split
        // inside a code point (String::truncate on bytes would panic or
        // corrupt).
        let long = "é🚀".repeat(2000);
        let (json, truncated) = payload(WebhookKind::Discord, &long);
        assert!(truncated);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["content"].as_str().unwrap().chars().count() <= DISCORD_MAX_CHARS);
    }

    #[test]
    fn url_validation_requires_http() {
        assert!(valid_url("https://hooks.slack.com/services/T/B/x"));
        assert!(valid_url("  http://internal.example/hook  "));
        assert!(!valid_url("hooks.slack.com/services/T/B/x"));
        assert!(!valid_url("file:///etc/passwd"));
        assert!(!valid_url(""));
        // send() refuses before curl ever runs.
        assert!(send(WebhookKind::Slack, "not-a-url", "x").is_err());
    }
}
