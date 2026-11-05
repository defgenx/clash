//! Jira transport for workflow sharing: post a share document as one comment
//! on an issue.
//!
//! Shells out to `curl` — the `infrastructure::webhook` approach — with stdin
//! closed and a hard timeout. The payload shaping (markdown → Jira wiki
//! markup, truncation, ticket-key detection) is pure and unit-tested; the
//! send is a thin wrapper. Credentials travel through a 0600 curl config
//! file, never argv (argv is world-readable via `ps`, and an API token is a
//! real credential where a webhook URL is only a capability URL).
//!
//! Nothing here decides *when* to post: sharing is an explicit user action
//! with a preview.

/// Jira rejects comment bodies over 32,767 characters.
const JIRA_MAX_CHARS: usize = 32_000;

/// Ticket keys have one shape across trackers; the rule lives with them.
pub use crate::application::trackers::valid_ticket_key;

/// Pure: the first Jira issue key found in `text` (`PROJ-123`, any case —
/// branch names often carry `ps-1234`), uppercased to the canonical form.
/// Used to pre-fill the share dialog's ticket prompt from the item's
/// title/branch/slug.
pub fn detect_ticket_key(text: &str) -> Option<String> {
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        // Candidate start: a letter not preceded by a letter/digit.
        if bytes[i].is_ascii_alphabetic() && (i == 0 || !bytes[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
                j += 1;
            }
            // Project keys are at least two chars; a single letter before a
            // dash is a version fragment (`v-2`), not a ticket.
            if j - i >= 2 && j < bytes.len() && bytes[j] == '-' {
                let mut k = j + 1;
                while k < bytes.len() && bytes[k].is_ascii_digit() {
                    k += 1;
                }
                let digits = k - (j + 1);
                let end_ok = k == bytes.len() || !bytes[k].is_ascii_alphanumeric();
                if digits > 0 && end_ok {
                    return Some(bytes[i..k].iter().collect::<String>().to_ascii_uppercase());
                }
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// Pure: a modest markdown → Jira wiki-markup conversion, good enough for the
/// share document to read natively in a Jira comment (the REST v2 comment
/// body takes wiki markup, not markdown). Line-based: headings, fences,
/// bullets, rules; inline bold/code/links outside fences. Anything it doesn't
/// recognize passes through verbatim.
pub fn wiki_markup(md: &str) -> String {
    let mut out = Vec::new();
    let mut in_fence = false;
    for line in md.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("```") {
            if in_fence {
                out.push("{code}".to_string());
            } else {
                let lang = rest.trim();
                out.push(if lang.is_empty() {
                    "{code}".to_string()
                } else {
                    format!("{{code:{}}}", lang)
                });
            }
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            out.push(line.to_string());
            continue;
        }
        let mut l = line.to_string();
        // Headings: `## x` → `h2. x` (Jira knows h1..h6).
        if let Some(n) = trimmed.find(|c| c != '#') {
            if (1..=6).contains(&n) && trimmed[n..].starts_with(' ') {
                l = format!("h{}. {}", n, trimmed[n + 1..].trim_start());
                out.push(l);
                continue;
            }
        }
        if trimmed == "---" || trimmed == "***" {
            out.push("----".to_string());
            continue;
        }
        // Bullets: `- x` → `* x`, preserving one nesting level per 2 spaces.
        if let Some(rest) = trimmed.strip_prefix("- ") {
            let depth = (line.len() - trimmed.len()) / 2;
            l = format!("{} {}", "*".repeat(depth + 1), rest);
            out.push(inline_wiki(&l));
            continue;
        }
        out.push(inline_wiki(&l));
    }
    // An unclosed fence would swallow the rest of the comment in Jira.
    if in_fence {
        out.push("{code}".to_string());
    }
    out.join("\n")
}

/// Inline conversions: `**b**` → `*b*`, `` `c` `` → `{{c}}`,
/// `[t](u)` → `[t|u]`. Deliberately regex-free and single-pass per pattern.
fn inline_wiki(line: &str) -> String {
    let mut s = replace_pairs(line, "**", "*");
    s = replace_pairs(&s, "`", |inner: &str| format!("{{{{{}}}}}", inner));
    // Links: [text](url) → [text|url].
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(start) = rest.find('[') {
        let (before, after) = rest.split_at(start);
        out.push_str(before);
        if let Some(mid) = after.find("](") {
            if let Some(end) = after[mid + 2..].find(')') {
                let text = &after[1..mid];
                let url = &after[mid + 2..mid + 2 + end];
                out.push_str(&format!("[{}|{}]", text, url));
                rest = &after[mid + 2 + end + 1..];
                continue;
            }
        }
        out.push('[');
        rest = &after[1..];
    }
    out.push_str(rest);
    out
}

/// Replace balanced `delim…delim` pairs on one line. `wrap` is either the
/// replacement delimiter (&str) or a closure building the whole replacement.
fn replace_pairs<W>(line: &str, delim: &str, wrap: W) -> String
where
    W: PairWrap,
{
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    loop {
        let Some(a) = rest.find(delim) else {
            out.push_str(rest);
            return out;
        };
        let after = &rest[a + delim.len()..];
        let Some(b) = after.find(delim) else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..a]);
        out.push_str(&wrap.wrap(&after[..b]));
        rest = &after[b + delim.len()..];
    }
}

trait PairWrap {
    fn wrap(&self, inner: &str) -> String;
}
impl PairWrap for &str {
    fn wrap(&self, inner: &str) -> String {
        format!("{}{}{}", self, inner, self)
    }
}
impl<F: Fn(&str) -> String> PairWrap for F {
    fn wrap(&self, inner: &str) -> String {
        self(inner)
    }
}

/// Pure: the JSON body for the comment POST, truncated to Jira's hard limit
/// (on a char boundary, with an explicit marker — a silently cut comment
/// reads as complete). Returns the body and whether it was truncated.
pub fn comment_payload(text: &str) -> (String, bool) {
    let marker = "\n… (truncated)";
    let truncated = text.chars().count() > JIRA_MAX_CHARS;
    let body = if truncated {
        let keep: String = text
            .chars()
            .take(JIRA_MAX_CHARS - marker.chars().count())
            .collect();
        format!("{}{}", keep, marker)
    } else {
        text.to_string()
    };
    let json = serde_json::json!({ "body": body });
    (json.to_string(), truncated)
}

/// POST `markdown` (converted to wiki markup) as a comment on `ticket`.
/// Blocking (callers wrap in `spawn_blocking`). Returns whether the comment
/// was truncated to fit Jira's limit.
pub fn post_comment(
    base_url: &str,
    email: &str,
    token: &str,
    ticket: &str,
    markdown: &str,
) -> Result<bool, String> {
    let base = base_url.trim().trim_end_matches('/');
    if !crate::infrastructure::webhook::valid_url(base) {
        return Err(format!("not an http(s) Jira URL: {:?}", base));
    }
    let ticket = ticket.trim().to_ascii_uppercase();
    if !valid_ticket_key(&ticket) {
        return Err(format!("not a Jira ticket key: {:?}", ticket));
    }
    let (email, token) = (email.trim(), token.trim());
    // These land in a curl config file; quotes/newlines would break out of it.
    if email.is_empty() || token.is_empty() {
        return Err("Jira email/API token not configured".to_string());
    }
    if [email, token]
        .iter()
        .any(|s| s.contains('"') || s.chars().any(|c| c.is_control()))
    {
        return Err("Jira credentials contain unsupported characters".to_string());
    }
    let (body, truncated) = comment_payload(&wiki_markup(markdown));
    let url = format!("{}/rest/api/2/issue/{}/comment", base, ticket);

    // Body and credentials travel through temp files, not argv: the body can
    // exceed argv limits, and argv is visible to every local process.
    let pid = std::process::id();
    let tmp_body = std::env::temp_dir().join(format!("clash-jira-{}.json", pid));
    let tmp_auth = std::env::temp_dir().join(format!("clash-jira-{}.curl", pid));
    std::fs::write(&tmp_body, &body).map_err(|e| e.to_string())?;
    let auth = format!("user = \"{}:{}\"\n", email, token);
    let write_auth = crate::infrastructure::secure_post::write_private(&tmp_auth, &auth);
    let result = write_auth.and_then(|_| {
        crate::infrastructure::secure_post::post_json(&url, &tmp_body, &tmp_auth, "Jira")
    });
    let _ = std::fs::remove_file(&tmp_body);
    let _ = std::fs::remove_file(&tmp_auth);
    result.map(|_| truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticket_keys_validate() {
        assert!(valid_ticket_key("PS-1234"));
        assert!(valid_ticket_key(" ab2-9 ")); // lenient case; POST uppercases
        assert!(!valid_ticket_key("PS1234"));
        assert!(!valid_ticket_key("PS-"));
        assert!(!valid_ticket_key("-123"));
        assert!(!valid_ticket_key("2FA-12"));
        assert!(!valid_ticket_key("PS-12a"));
        assert!(!valid_ticket_key(""));
    }

    #[test]
    fn ticket_detection_finds_keys_in_titles_and_branches() {
        assert_eq!(
            detect_ticket_key("Fix login PS-1234"),
            Some("PS-1234".into())
        );
        assert_eq!(detect_ticket_key("fix/ps-987-login"), Some("PS-987".into()));
        assert_eq!(detect_ticket_key("feature/DOP-1"), Some("DOP-1".into()));
        assert_eq!(detect_ticket_key("no ticket here"), None);
        // Single-letter fragments are not tickets; "utf-8"-style false
        // positives are accepted — this only pre-fills an editable prompt.
        assert_eq!(detect_ticket_key("v-2 of the api"), None);
        assert_eq!(detect_ticket_key("utf-8 handling"), Some("UTF-8".into()));
    }

    #[test]
    fn wiki_markup_converts_the_common_shapes() {
        let md = "# Title\n\n## Part\n- one\n  - nested\n\n**bold** and `code` and [x](https://e.x)\n\n```rust\nfn a() {}\n```\n---";
        let w = wiki_markup(md);
        assert!(w.contains("h1. Title"));
        assert!(w.contains("h2. Part"));
        assert!(w.contains("* one"));
        assert!(w.contains("** nested"));
        assert!(w.contains("*bold* and {{code}} and [x|https://e.x]"));
        assert!(w.contains("{code:rust}\nfn a() {}\n{code}"));
        assert!(w.contains("----"));
    }

    #[test]
    fn wiki_markup_leaves_fence_contents_alone_and_closes_open_fences() {
        let md = "```\n# not a heading\n**not bold**";
        let w = wiki_markup(md);
        assert!(w.contains("# not a heading"));
        assert!(w.contains("**not bold**"));
        assert!(w.ends_with("{code}"));
    }

    #[test]
    fn payload_truncates_at_the_limit_with_a_marker() {
        let (json, t) = comment_payload("hello");
        assert!(!t);
        assert_eq!(json, r#"{"body":"hello"}"#);
        let long = "é🚀x".repeat(20_000);
        let (json, t) = comment_payload(&long);
        assert!(t);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let body = parsed["body"].as_str().unwrap();
        assert!(body.chars().count() <= JIRA_MAX_CHARS);
        assert!(body.ends_with("… (truncated)"));
    }

    #[test]
    fn post_refuses_bad_inputs_before_curl_runs() {
        assert!(post_comment("atlassian.net", "e", "t", "PS-1", "x").is_err());
        assert!(post_comment("https://x.atlassian.net", "e", "t", "nope", "x").is_err());
        assert!(post_comment("https://x.atlassian.net", "", "t", "PS-1", "x").is_err());
        assert!(post_comment("https://x.atlassian.net", "e", "a\"b", "PS-1", "x").is_err());
    }
}
