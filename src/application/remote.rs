//! The phone remote's pure half: who may call the API, and what a phone is
//! told about this instance. The server is `gui/src-tauri/src/remote.rs`; the
//! design and security model are `docs/remote.md`.

use std::net::{SocketAddr, ToSocketAddrs};

use serde::Serialize;

use crate::domain::entities::{Session, SessionStatus};
use crate::domain::workflow::WorkflowItem;

/// Random bytes in a pairing token (hex-encoded on disk and on the wire).
pub const TOKEN_BYTES: usize = 32;

/// Lowercase hex of `bytes` — the token's one spelling.
pub fn encode_token(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A token file's content is usable only when it is exactly the shape clash
/// writes; anything else (truncated, edited, empty) is regenerated rather than
/// trusted.
pub fn valid_token(token: &str) -> bool {
    token.len() == TOKEN_BYTES * 2 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Compare a presented token against the expected one in time that does not
/// depend on where they first differ. An empty expected token matches nothing.
pub fn token_matches(expected: &str, presented: &str) -> bool {
    let (e, p) = (expected.as_bytes(), presented.as_bytes());
    if e.is_empty() {
        return false;
    }
    let mut diff = u8::from(e.len() != p.len());
    for (i, &b) in e.iter().enumerate() {
        diff |= b ^ p.get(i).copied().unwrap_or(0);
    }
    diff == 0
}

/// The token an `Authorization: Bearer <token>` header carries.
pub fn bearer(header: Option<&str>) -> Option<&str> {
    let (scheme, token) = header?.trim().split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| token.trim())
        .filter(|t| !t.is_empty())
}

/// Parse `remote.bind` (`host:port`; `localhost` allowed).
pub fn parse_bind(bind: &str) -> Result<SocketAddr, String> {
    let bind = bind.trim();
    if let Ok(addr) = bind.parse::<SocketAddr>() {
        return Ok(addr);
    }
    match bind.rsplit_once(':') {
        Some(("localhost", port)) => port
            .parse::<u16>()
            .map(|p| SocketAddr::from(([127, 0, 0, 1], p)))
            .map_err(|_| format!("invalid port in `{bind}`")),
        _ => bind
            .to_socket_addrs()
            .ok()
            .and_then(|mut it| it.next())
            .ok_or_else(|| format!("`{bind}` is not a host:port address")),
    }
}

/// Only reachable from this machine.
pub fn is_loopback(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback()
}

/// The link that pairs a phone: the page, with the token in the fragment so it
/// never rides a request line (and so never lands in a server or proxy log).
/// An unspecified bind (`0.0.0.0`) is not an address a phone can open, so the
/// machine's host name stands in for it.
pub fn pairing_url(addr: &SocketAddr, hostname: &str, token: &str) -> String {
    let host = if addr.ip().is_unspecified() {
        hostname.to_string()
    } else if addr.is_ipv6() {
        format!("[{}]", addr.ip())
    } else {
        addr.ip().to_string()
    };
    format!("http://{host}:{}/remote/#token={token}", addr.port())
}

/// Another clash instance this GUI has been paired with: its origin and the
/// token its pairing link carried. Persisted 0600 by the GUI; the token never
/// goes back to the webview.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
pub struct RemotePeer {
    pub name: String,
    pub url: String,
    pub token: String,
}

/// A pairing link (`http://host:port/remote/#token=<64 hex>`) → the peer's
/// origin and token. Mirrors `parsePairingLink` in `gui/dist/remote-model.js`.
pub fn parse_pairing_link(text: &str) -> Option<(String, String)> {
    let text = text.trim();
    let (scheme, rest) = text.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..end];
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let fragment = rest.split_once('#')?.1;
    let token = fragment
        .split('&')
        .find_map(|kv| kv.strip_prefix("token="))?
        .to_ascii_lowercase();
    valid_token(&token).then(|| (format!("{scheme}://{authority}"), token))
}

/// Add or re-pair a peer: one entry per origin, the newest token wins, and a
/// re-pair keeps the name.
pub fn add_peer(
    peers: &[RemotePeer],
    url: &str,
    token: &str,
    name: Option<&str>,
) -> Vec<RemotePeer> {
    let old = peers.iter().find(|p| p.url == url);
    let host = url
        .split_once("://")
        .map(|(_, a)| a)
        .unwrap_or(url)
        .to_string();
    let name = name
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .or_else(|| old.map(|p| p.name.clone()))
        .unwrap_or(host);
    let mut out: Vec<RemotePeer> = peers.iter().filter(|p| p.url != url).cloned().collect();
    out.push(RemotePeer {
        name,
        url: url.to_string(),
        token: token.to_string(),
    });
    out
}

/// What a phone polls for each paired machine.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceStatus {
    /// Always `"clash"`, so the page can tell clash from whatever else
    /// answers on that port.
    pub app: &'static str,
    pub hostname: String,
    pub version: String,
    pub pid: u32,
    /// Epoch ms this instance started.
    pub started_at: i64,
    /// Epoch ms on this machine when the status was built — the phone measures
    /// uptime against it, never against its own clock.
    pub now: i64,
    pub sessions_running: usize,
    pub needs_you: usize,
}

/// Facts about this process the status is built from.
pub struct InstanceFacts<'a> {
    pub hostname: &'a str,
    pub version: &'a str,
    pub pid: u32,
    pub started_at: i64,
    pub now: i64,
}

pub fn instance_status(
    facts: InstanceFacts<'_>,
    sessions: &[Session],
    items: &[WorkflowItem],
) -> InstanceStatus {
    InstanceStatus {
        app: "clash",
        hostname: facts.hostname.to_string(),
        version: facts.version.to_string(),
        pid: facts.pid,
        started_at: facts.started_at,
        now: facts.now,
        sessions_running: sessions.iter().filter(|s| s.is_running).count(),
        needs_you: needs_you(sessions, items),
    }
}

/// How many inbox rows there are: the count of `attentionRows` in
/// `gui/dist/attention.js`, which is the canonical rule — keep the two in step.
pub fn needs_you(sessions: &[Session], items: &[WorkflowItem]) -> usize {
    let session_rows = sessions
        .iter()
        .filter(|s| match s.status {
            SessionStatus::Errored => true,
            SessionStatus::Prompting | SessionStatus::Waiting => s.is_running,
            _ => false,
        })
        .count();
    let item_rows = items
        .iter()
        .filter(|it| {
            let unanswered: u64 = it
                .meta
                .pr
                .iter()
                .chain(it.meta.linked_prs.iter())
                .filter(|p| !p.url.is_empty())
                .filter_map(|p| p.unanswered_comments)
                .sum();
            !it.agent_alive || it.meta.status.needs_attention() || unanswered > 0
        })
        .count();
    session_rows + item_rows
}

// ── Actions (`remote.actions`) ──────────────────────────────────────────

/// The keys a paired device may send, by name. A fixed list rather than raw
/// bytes: answering a prompt needs these, and a phone that could send any
/// byte sequence could type anything into a shell.
pub fn key_bytes(name: &str) -> Option<&'static [u8]> {
    Some(match name {
        "1" => b"1",
        "2" => b"2",
        "3" => b"3",
        "4" => b"4",
        "5" => b"5",
        "6" => b"6",
        "7" => b"7",
        "8" => b"8",
        "9" => b"9",
        "enter" => b"\r",
        "esc" => b"\x1b",
        "ctrl-c" => b"\x03",
        "tab" => b"\t",
        "shift-tab" => b"\x1b[Z",
        "up" => b"\x1b[A",
        "down" => b"\x1b[B",
        _ => return None,
    })
}

/// One option of a numbered prompt on screen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PromptChoice {
    /// The key that picks it (`"1"`…`"9"`).
    pub key: String,
    pub label: String,
}

/// A numbered prompt found on screen (a permission dialog, a question).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ScreenPrompt {
    /// The line just above the options, when it reads like the question.
    pub question: Option<String>,
    pub choices: Vec<PromptChoice>,
}

/// A row's text without the box it may be drawn in (`│ … │`) or the cursor
/// marker (`❯`, `>`).
fn unboxed(line: &str) -> &str {
    line.trim()
        .trim_start_matches(['│', '|', '┃'])
        .trim_end_matches(['│', '|', '┃'])
        .trim()
}

/// A row that only continues the option above it: indented, inside the box.
fn is_continuation(line: &str) -> bool {
    let inner = line.trim_start().trim_start_matches(['│', '|', '┃']);
    inner.starts_with("     ") && !unboxed(line).is_empty()
}

/// `"❯ 2. Yes, and don't ask again"` → `(2, "Yes, and don't ask again")`.
fn numbered(line: &str) -> Option<(u32, String)> {
    let t = unboxed(line)
        .trim_start_matches(['❯', '>', '›'])
        .trim_start();
    let (num, rest) = t.split_once(". ")?;
    if num.len() != 1 {
        return None;
    }
    let n = num.parse::<u32>().ok().filter(|n| (1..=9).contains(n))?;
    let label = rest.trim();
    (!label.is_empty()).then(|| (n, label.to_string()))
}

/// The numbered prompt at the bottom of the screen, if there is one: the last
/// run of options numbered 1, 2, 3… with at least two of them, within the
/// last 20 rows (a list further up is scrollback the agent printed, not a
/// question). Numbering must start at 1 and be consecutive, so ordinary
/// numbered prose ("3. Run the tests") doesn't read as a prompt.
pub fn screen_prompt(lines: &[String]) -> Option<ScreenPrompt> {
    let tail = &lines[lines.len().saturating_sub(20)..];
    let mut best: Option<(usize, Vec<PromptChoice>)> = None;
    let mut run: Vec<PromptChoice> = Vec::new();
    let mut start = 0;
    for (i, line) in tail.iter().enumerate() {
        match numbered(line) {
            Some((n, label)) if n as usize == run.len() + 1 => {
                if run.is_empty() {
                    start = i;
                }
                run.push(PromptChoice {
                    key: n.to_string(),
                    label,
                });
            }
            Some((1, label)) => {
                run = vec![PromptChoice {
                    key: "1".into(),
                    label,
                }];
                start = i;
            }
            // A wrapped option label continues on the next row, indented
            // past the number.
            None if !run.is_empty() && is_continuation(line) => {}
            _ => {
                if run.len() >= 2 {
                    best = Some((start, std::mem::take(&mut run)));
                }
                run.clear();
            }
        }
    }
    if run.len() >= 2 {
        best = Some((start, run));
    }
    let (start, choices) = best?;
    let question = tail[..start]
        .iter()
        .rev()
        .map(|l| unboxed(l))
        .find(|l| !l.is_empty() && !l.chars().all(|c| "─━═╭╮╰╯┌┐└┘".contains(c)))
        .filter(|l| l.ends_with('?'))
        .map(str::to_string);
    Some(ScreenPrompt { question, choices })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{WorkflowPr, WorkflowStatus};

    #[test]
    fn tokens_are_64_hex_chars_and_compare_exactly() {
        let t = encode_token(&[0xab; TOKEN_BYTES]);
        assert_eq!(t.len(), 64);
        assert!(valid_token(&t));
        assert!(!valid_token(&t[..63]));
        assert!(!valid_token(&format!("{}g", &t[..63])));
        assert!(!valid_token(""));
        assert!(token_matches(&t, &t));
        assert!(!token_matches(&t, &t[..63]));
        assert!(!token_matches(&t, &format!("{t}0")));
        assert!(!token_matches(&t, &encode_token(&[0xac; TOKEN_BYTES])));
        assert!(!token_matches(&t, ""));
        // No token configured lets nobody in, not everybody.
        assert!(!token_matches("", ""));
    }

    #[test]
    fn the_bearer_scheme_is_case_insensitive_and_needs_a_token() {
        assert_eq!(bearer(Some("Bearer abc")), Some("abc"));
        assert_eq!(bearer(Some("bearer  abc ")), Some("abc"));
        assert_eq!(bearer(Some("Basic abc")), None);
        assert_eq!(bearer(Some("Bearer ")), None);
        assert_eq!(bearer(Some("abc")), None);
        assert_eq!(bearer(None), None);
    }

    #[test]
    fn binds_parse_and_say_whether_a_phone_can_reach_them() {
        let lo = parse_bind("127.0.0.1:7878").unwrap();
        assert!(is_loopback(&lo));
        assert!(is_loopback(&parse_bind("localhost:7878").unwrap()));
        let ts = parse_bind("100.64.1.2:7878").unwrap();
        assert!(!is_loopback(&ts));
        assert!(parse_bind("7878").is_err());
        assert!(parse_bind("localhost:http").is_err());
        assert_eq!(
            pairing_url(&ts, "mac", "t"),
            "http://100.64.1.2:7878/remote/#token=t"
        );
        assert_eq!(
            pairing_url(&parse_bind("0.0.0.0:9").unwrap(), "mac.local", "t"),
            "http://mac.local:9/remote/#token=t"
        );
        assert_eq!(
            pairing_url(&parse_bind("[::1]:9").unwrap(), "m", "t"),
            "http://[::1]:9/remote/#token=t"
        );
    }

    #[test]
    fn pairing_links_yield_the_origin_and_token_only() {
        let t = "a".repeat(64);
        assert_eq!(
            parse_pairing_link(&format!("http://100.64.1.2:7878/remote/#token={t}")),
            Some(("http://100.64.1.2:7878".into(), t.clone()))
        );
        assert_eq!(
            parse_pairing_link(&format!(
                " https://mac.tail.ts.net/remote/#x=1&token={} ",
                t.to_uppercase()
            )),
            Some(("https://mac.tail.ts.net".into(), t.clone()))
        );
        assert_eq!(parse_pairing_link(&format!("ftp://h/#token={t}")), None);
        assert_eq!(parse_pairing_link("http://h/remote/#token=abc"), None);
        assert_eq!(parse_pairing_link(&format!("http://u@h/#token={t}")), None);
        assert_eq!(parse_pairing_link("http://h/remote/"), None);
    }

    #[test]
    fn a_peer_is_one_per_origin_and_a_repair_keeps_its_name() {
        let a = add_peer(&[], "http://h:1", "t1", None);
        assert_eq!(a[0].name, "h:1");
        let b = add_peer(&a, "http://h:1", "t2", Some("Studio Mac"));
        let c = add_peer(&b, "http://h:1", "t3", None);
        assert_eq!(c.len(), 1);
        assert_eq!(
            (c[0].name.as_str(), c[0].token.as_str()),
            ("Studio Mac", "t3")
        );
        assert_eq!(add_peer(&c, "http://g:1", "t", None).len(), 2);
    }

    fn session(status: SessionStatus, running: bool) -> Session {
        Session {
            status,
            is_running: running,
            ..Default::default()
        }
    }

    fn item(status: WorkflowStatus, alive: bool, unanswered: Option<u64>) -> WorkflowItem {
        let mut it = WorkflowItem {
            agent_alive: alive,
            ..Default::default()
        };
        it.meta.status = status;
        it.meta.pr = unanswered.map(|n| WorkflowPr {
            url: "https://github.com/o/r/pull/1".into(),
            unanswered_comments: Some(n),
            ..Default::default()
        });
        it
    }

    /// Mirrors attention.js's rows: one per session that wants you, one per
    /// item however many reasons it has.
    #[test]
    fn needs_you_counts_the_inbox_rows() {
        let sessions = vec![
            session(SessionStatus::Prompting, true),
            session(SessionStatus::Waiting, true),
            session(SessionStatus::Waiting, false), // over, not waiting
            session(SessionStatus::Errored, false),
            session(SessionStatus::Thinking, true),
        ];
        let items = vec![
            item(WorkflowStatus::PlanReview, true, None),
            item(WorkflowStatus::DiffReview, true, Some(2)), // one row, two reasons
            item(WorkflowStatus::Implementing, false, None), // agent gone
            item(WorkflowStatus::Implementing, true, Some(0)),
            item(WorkflowStatus::PrReady, true, Some(1)),
        ];
        assert_eq!(needs_you(&sessions, &items), 3 + 4);
        let st = instance_status(
            InstanceFacts {
                hostname: "mac",
                version: "1.0.0",
                pid: 7,
                started_at: 1,
                now: 2,
            },
            &sessions,
            &items,
        );
        assert_eq!(st.app, "clash");
        assert_eq!(st.sessions_running, 3);
        assert_eq!(st.needs_you, 7);
    }

    #[test]
    fn only_named_keys_can_be_sent() {
        assert_eq!(key_bytes("1"), Some(&b"1"[..]));
        assert_eq!(key_bytes("enter"), Some(&b"\r"[..]));
        assert_eq!(key_bytes("esc"), Some(&b"\x1b"[..]));
        assert_eq!(key_bytes("ctrl-c"), Some(&b"\x03"[..]));
        for nope in ["0", "10", "a", "rm -rf /", "\x1b", "", "ctrl-d"] {
            assert_eq!(key_bytes(nope), None, "{nope:?}");
        }
    }

    fn lines(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    #[test]
    fn a_permission_dialog_reads_as_its_options_and_question() {
        let screen = lines(
            "some earlier output\n\
             ╭──────────────────────────────────────────╮\n\
             │ Bash command                             │\n\
             │   cargo test --workspace                 │\n\
             │ Do you want to proceed?                  │\n\
             │ ❯ 1. Yes                                 │\n\
             │   2. Yes, and don't ask again for cargo  │\n\
             │   3. No, and tell Claude what to do (esc)│\n\
             ╰──────────────────────────────────────────╯",
        );
        let p = screen_prompt(&screen).unwrap();
        assert_eq!(p.question.as_deref(), Some("Do you want to proceed?"));
        let keys: Vec<_> = p.choices.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, ["1", "2", "3"]);
        assert_eq!(p.choices[0].label, "Yes");
        assert_eq!(p.choices[2].label, "No, and tell Claude what to do (esc)");
    }

    #[test]
    fn numbered_prose_is_not_a_prompt() {
        // Not starting at 1, a single item, or a gap: none of these is a question.
        assert_eq!(screen_prompt(&lines("3. Run the tests\n4. Ship it")), None);
        assert_eq!(screen_prompt(&lines("1. Only one")), None);
        assert_eq!(screen_prompt(&lines("1. a\n3. c")), None);
        assert_eq!(screen_prompt(&[]), None);
        // A list far up the screen is scrollback, not the prompt.
        let mut old = lines("1. first\n2. second");
        old.extend(std::iter::repeat_n(String::from("output"), 25));
        assert_eq!(screen_prompt(&old), None);
    }

    #[test]
    fn the_last_numbered_run_wins_and_no_question_is_invented() {
        let p = screen_prompt(&lines("1. old\n2. older\n\nPick one\n1. Red\n2. Blue")).unwrap();
        assert_eq!(p.choices[0].label, "Red");
        assert_eq!(p.question, None, "\"Pick one\" is no question");
    }

    #[test]
    fn a_wrapped_option_label_does_not_end_the_run() {
        let p = screen_prompt(&lines(
            "│ ❯ 1. Yes                 │\n│   2. Yes, and don't ask    │\n│      again for this tool   │\n│   3. No                    │",
        ))
        .unwrap();
        assert_eq!(p.choices.len(), 3);
    }
}
