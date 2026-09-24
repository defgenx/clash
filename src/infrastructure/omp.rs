//! OMP (oh-my-pi) adapter: session-file layout, transcript parsing and status
//! detection — the counterpart of the Claude Code transcript model in
//! `fs::backend`. Parsers are pure; the thin IO wrappers sit at the bottom.
//!
//! Identity: every OMP session clash spawns is created by
//! `omp --resume <path>` on a file that does not exist yet, which omp
//! materializes at exactly that path. clash names the file
//! `<timestamp>_<clash-uuid>.jsonl`, so the uuid suffix — not the header id —
//! is the session id everywhere in clash, the same role `claude --session-id`
//! plays. Sessions omp creates itself (`/new`) follow the same naming with
//! omp's own uuid. See docs/hooks.md ("OMP sessions").

use std::path::{Path, PathBuf};

use crate::adapters::format;
use crate::domain::entities::{ConversationMessage, SessionStatus};

/// Default OMP agent directory (`~/.omp/agent`).
pub fn default_agent_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".omp")
        .join("agent")
}

/// `<agent_dir>/sessions` — one bucket directory per project cwd.
pub fn sessions_root(agent_dir: &Path) -> PathBuf {
    agent_dir.join("sessions")
}

// ── Pure: naming ─────────────────────────────────────────────────────

/// The clash session id of a transcript file name (`<ts>_<id>.jsonl` → `<id>`).
/// `None` for anything that is not a top-level session file.
pub fn session_id_from_file_name(name: &str) -> Option<&str> {
    let stem = name.strip_suffix(".jsonl")?;
    let (_, id) = stem.split_once('_')?;
    (!id.is_empty()).then_some(id)
}

/// omp's session-file name for a session created at `now`
/// (`2026-09-24T09-23-29-158Z_<id>.jsonl`: ISO-8601 millis, `:`/`.` → `-`).
pub fn session_file_name(id: &str, now: chrono::DateTime<chrono::Utc>) -> String {
    let ts = now
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
        .replace([':', '.'], "-");
    format!("{ts}_{id}.jsonl")
}

/// omp's bucket name for a (canonical) cwd: `-<rel>` under home, `-tmp-<rel>`
/// under the temp root, `--<abs>--` elsewhere, separators mapped to `-`.
/// Mirrors `getDefaultSessionDirName` in omp's `session-paths.ts`.
pub fn encode_session_dir_name(cwd: &Path, home: &Path, tmp: &Path) -> String {
    fn rel(prefix: &str, relative: &Path) -> String {
        let encoded = relative.to_string_lossy().replace(['/', '\\', ':'], "-");
        if encoded.is_empty() {
            prefix.to_string()
        } else if prefix.ends_with('-') {
            format!("{prefix}{encoded}")
        } else {
            format!("{prefix}-{encoded}")
        }
    }
    if let Ok(r) = cwd.strip_prefix(home) {
        return rel("-", r);
    }
    if let Ok(r) = cwd.strip_prefix(tmp) {
        return rel("-tmp", r);
    }
    let abs = cwd.to_string_lossy();
    format!(
        "--{}--",
        abs.trim_start_matches(['/', '\\'])
            .replace(['/', '\\', ':'], "-")
    )
}

// ── Pure: transcript parsing ─────────────────────────────────────────

/// What the head of an OMP transcript says about the session.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct OmpMeta {
    pub cwd: String,
    /// `/rename` or auto-generated title (title slot or header).
    pub title: String,
    /// First user message, whitespace-collapsed.
    pub first_prompt: String,
}

/// Text of an omp message `content` (string, or `[{type:"text",text}]`).
fn content_text(content: &serde_json::Value, joiner: &str) -> String {
    if let Some(s) = content.as_str() {
        return s.to_string();
    }
    content
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()).unwrap_or("text") == "text")
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join(joiner)
        })
        .unwrap_or_default()
}

/// Parse the head lines of a transcript: title slot, header, first user turn.
pub fn parse_meta<'a>(lines: impl IntoIterator<Item = &'a str>) -> OmpMeta {
    let mut meta = OmpMeta::default();
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("title") | Some("session") => {
                if meta.title.is_empty() {
                    if let Some(t) = v.get("title").and_then(|t| t.as_str()) {
                        meta.title = t.trim().to_string();
                    }
                }
                if meta.cwd.is_empty() {
                    if let Some(c) = v.get("cwd").and_then(|c| c.as_str()) {
                        meta.cwd = c.to_string();
                    }
                }
            }
            Some("message") => {
                let msg = v.get("message");
                let role = msg.and_then(|m| m.get("role")).and_then(|r| r.as_str());
                if meta.first_prompt.is_empty() && role == Some("user") {
                    let text = msg
                        .and_then(|m| m.get("content"))
                        .map(|c| content_text(c, " "))
                        .unwrap_or_default();
                    meta.first_prompt = text.split_whitespace().collect::<Vec<_>>().join(" ");
                }
            }
            _ => {}
        }
        if !meta.cwd.is_empty() && !meta.first_prompt.is_empty() {
            break;
        }
    }
    meta
}

/// Display summary: the title when there is one, else the first prompt.
pub fn display_summary(meta: &OmpMeta) -> String {
    if !meta.title.is_empty() {
        meta.title.clone()
    } else {
        format::truncate(&meta.first_prompt, 60, "...")
    }
}

/// Baseline status from the transcript tail — the omp analogue of
/// `FsBackend::detect_session_status`. Like it, never answers `Stashed`:
/// liveness is the refresh pipeline's call, not the transcript's.
pub fn detect_status<'a>(tail: impl IntoIterator<Item = &'a str>) -> SessionStatus {
    let mut status = SessionStatus::Waiting;
    for line in tail {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("message") => {
                let msg = v.get("message");
                let role = msg.and_then(|m| m.get("role")).and_then(|r| r.as_str());
                status = match role {
                    Some("assistant") => {
                        match msg
                            .and_then(|m| m.get("stopReason"))
                            .and_then(|s| s.as_str())
                        {
                            Some("toolUse") | None => SessionStatus::Thinking,
                            Some(_) => SessionStatus::Waiting,
                        }
                    }
                    Some("user") | Some("toolResult") | Some("developer") | Some("fileMention") => {
                        SessionStatus::Thinking
                    }
                    _ => status,
                };
            }
            Some("custom") => match v.get("customType").and_then(|t| t.as_str()) {
                Some("tool_execution_start") => status = SessionStatus::Running,
                Some("session_exit") => status = SessionStatus::Waiting,
                _ => {}
            },
            _ => {}
        }
    }
    status
}

/// User/assistant turns of a transcript, for the conversation view.
pub fn parse_conversation<'a>(
    lines: impl IntoIterator<Item = &'a str>,
) -> Vec<ConversationMessage> {
    let mut out = Vec::new();
    for line in lines {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("message") {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        let role = match msg.get("role").and_then(|r| r.as_str()) {
            Some(r @ ("user" | "assistant")) => r,
            _ => continue,
        };
        let text = msg
            .get("content")
            .map(|c| content_text(c, "\n"))
            .unwrap_or_default();
        if !text.is_empty() {
            out.push(ConversationMessage {
                role: role.to_string(),
                text,
            });
        }
    }
    out
}

// ── IO wrappers ──────────────────────────────────────────────────────

/// Bucket directory omp uses for `cwd` under `agent_dir`. The cwd, home and
/// temp root are canonicalized first, as omp does, so `/tmp` and
/// `/private/tmp` land in the same bucket.
pub fn session_dir_for(agent_dir: &Path, cwd: &str) -> PathBuf {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let name = encode_session_dir_name(
        &canon(Path::new(cwd)),
        &canon(&home),
        &canon(&std::env::temp_dir()),
    );
    sessions_root(agent_dir).join(name)
}

/// Path a new session with clash id `id` should be created at.
pub fn new_session_path(agent_dir: &Path, cwd: &str, id: &str) -> PathBuf {
    session_dir_for(agent_dir, cwd).join(session_file_name(id, chrono::Utc::now()))
}

/// Every top-level session file under `agent_dir`, as `(id, path)`.
pub fn list_session_files(agent_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let Ok(buckets) = std::fs::read_dir(sessions_root(agent_dir)) else {
        return out;
    };
    for bucket in buckets.flatten() {
        let bucket = bucket.path();
        if !bucket.is_dir() {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&bucket) else {
            continue;
        };
        for f in files.flatten() {
            let path = f.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(id) = session_id_from_file_name(name) {
                out.push((id.to_string(), path));
            }
        }
    }
    out
}

/// The transcript of session `id`, if omp has one on disk.
pub fn find_session_file(agent_dir: &Path, id: &str) -> Option<PathBuf> {
    if id.is_empty() {
        return None;
    }
    list_session_files(agent_dir)
        .into_iter()
        .find(|(sid, _)| sid == id)
        .map(|(_, p)| p)
}

/// First `n` lines of a file (metadata lives at the top).
pub fn read_head(path: &Path, n: usize) -> Vec<String> {
    use std::io::BufRead;
    let Ok(f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    std::io::BufReader::new(f)
        .lines()
        .take(n)
        .map_while(Result::ok)
        .collect()
}

/// Complete lines in the last `bytes` of a file (status lives at the bottom).
pub fn read_tail(path: &Path, bytes: u64) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(bytes);
    let _ = f.seek(SeekFrom::Start(start));
    let mut buf = String::new();
    let _ = f.read_to_string(&mut buf);
    let mut lines: Vec<String> = buf.lines().map(str::to_string).collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // partial first line
    }
    lines
}

/// Subagent transcripts: `<session file minus .jsonl>/<agent-id>.jsonl`.
pub fn subagent_files(session_file: &Path) -> Vec<(String, PathBuf)> {
    let dir = session_file.with_extension("");
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return out;
    };
    for e in entries.flatten() {
        let path = e.path();
        if let Some(stem) = path
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.strip_suffix(".jsonl"))
        {
            out.push((stem.to_string(), path.clone()));
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_is_the_uuid_suffix_of_the_file_name() {
        assert_eq!(
            session_id_from_file_name("2026-09-24T09-23-29-158Z_01a0d2ba-2306-743d.jsonl"),
            Some("01a0d2ba-2306-743d")
        );
        assert_eq!(session_id_from_file_name("abc.jsonl"), None);
        assert_eq!(session_id_from_file_name("x_y.json"), None);
    }

    #[test]
    fn file_name_matches_omps_timestamp_format() {
        let t = chrono::DateTime::parse_from_rfc3339("2026-09-24T09:23:29.158Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(
            session_file_name("u-1", t),
            "2026-09-24T09-23-29-158Z_u-1.jsonl"
        );
        assert_eq!(
            session_id_from_file_name(&session_file_name("u-1", t)),
            Some("u-1")
        );
    }

    #[test]
    fn bucket_names_follow_omps_three_scopes() {
        let home = Path::new("/Users/me");
        let tmp = Path::new("/private/tmp");
        assert_eq!(
            encode_session_dir_name(Path::new("/Users/me/work/alumni_connect"), home, tmp),
            "-work-alumni_connect"
        );
        assert_eq!(encode_session_dir_name(home, home, tmp), "-");
        assert_eq!(
            encode_session_dir_name(Path::new("/private/tmp/a/b"), home, tmp),
            "-tmp-a-b"
        );
        assert_eq!(
            encode_session_dir_name(Path::new("/opt/x"), home, tmp),
            "--opt-x--"
        );
    }

    const TRANSCRIPT: &str = r#"{"type":"title","v":1,"title":"Fix the build","updatedAt":"x"}
{"type":"session","version":3,"id":"hdr","timestamp":"t","cwd":"/work/p"}
{"type":"model_change","id":"1","parentId":null,"model":"a/b"}
{"type":"message","id":"2","parentId":"1","message":{"role":"user","content":[{"type":"text","text":"Make   it\ngreen"}]}}
{"type":"message","id":"3","parentId":"2","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hm"},{"type":"text","text":"On it."},{"type":"toolCall","id":"c"}],"stopReason":"toolUse"}}
{"type":"message","id":"4","parentId":"3","message":{"role":"toolResult","toolCallId":"c","content":[{"type":"text","text":"ok"}]}}
{"type":"message","id":"5","parentId":"4","message":{"role":"assistant","content":[{"type":"text","text":"Done."}],"stopReason":"stop"}}"#;

    #[test]
    fn meta_reads_title_cwd_and_first_prompt() {
        let m = parse_meta(TRANSCRIPT.lines());
        assert_eq!(m.cwd, "/work/p");
        assert_eq!(m.title, "Fix the build");
        assert_eq!(m.first_prompt, "Make it green");
        assert_eq!(display_summary(&m), "Fix the build");
        let untitled = OmpMeta {
            title: String::new(),
            ..m
        };
        assert_eq!(display_summary(&untitled), "Make it green");
    }

    #[test]
    fn status_follows_the_last_turn() {
        let lines: Vec<&str> = TRANSCRIPT.lines().collect();
        assert_eq!(detect_status(lines.iter().copied()), SessionStatus::Waiting);
        assert_eq!(
            detect_status(lines[..5].iter().copied()),
            SessionStatus::Thinking
        );
        assert_eq!(
            detect_status(lines[..6].iter().copied()),
            SessionStatus::Thinking
        );
        assert_eq!(
            detect_status(lines[..4].iter().copied()),
            SessionStatus::Thinking
        );
        let running = r#"{"type":"custom","customType":"tool_execution_start","data":{}}"#;
        assert_eq!(detect_status([lines[4], running]), SessionStatus::Running);
        let exit = r#"{"type":"custom","customType":"session_exit","data":{}}"#;
        assert_eq!(detect_status([lines[4], exit]), SessionStatus::Waiting);
        let errored = r#"{"type":"message","message":{"role":"assistant","content":[],"stopReason":"error"}}"#;
        assert_eq!(detect_status([errored]), SessionStatus::Waiting);
    }

    #[test]
    fn conversation_keeps_text_of_user_and_assistant_turns_only() {
        let c = parse_conversation(TRANSCRIPT.lines());
        let pairs: Vec<(&str, &str)> = c
            .iter()
            .map(|m| (m.role.as_str(), m.text.as_str()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                ("user", "Make   it\ngreen"),
                ("assistant", "On it."),
                ("assistant", "Done.")
            ]
        );
    }
}
