//! Hermes Agent (`hermes`) sessions — pure parsers + thin IO over its store.
//!
//! Hermes keeps sessions in SQLite (`<hermes home>/state.db`), owned by its
//! background gateway; clash only ever opens it read-only. Hermes mints its own
//! ids, so clash names each session it spawns by **title** instead:
//! `hermes chat -c clash:<clash-id> --create-if-missing` creates it the first
//! time and resumes it (following compression to the live tip) every time
//! after. A compression rotates a conversation onto a new row whose parent
//! ended with `end_reason = 'compression'`; one conversation is one listed row.
//! Contract: `docs/hooks.md` → *Hermes sessions*.

use std::path::{Path, PathBuf};

/// Title prefix of every session clash spawns; the rest is the clash id.
pub const TITLE_PREFIX: &str = "clash:";

/// Name of the status plugin clash installs into `<hermes home>/plugins/`.
pub const PLUGIN_NAME: &str = "clash-status";

/// The Hermes home: `$HERMES_HOME`, else `~/.hermes`.
pub fn default_home() -> PathBuf {
    std::env::var_os("HERMES_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".hermes")))
        .unwrap_or_else(|| PathBuf::from(".hermes"))
}

pub fn state_db(home: &Path) -> PathBuf {
    home.join("state.db")
}

/// The title clash gives the Hermes session of clash id `id`.
pub fn session_title(id: &str) -> String {
    format!("{TITLE_PREFIX}{id}")
}

/// The clash id a session title names, when clash spawned it.
pub fn clash_id_of_title(title: &str) -> Option<&str> {
    title.strip_prefix(TITLE_PREFIX).filter(|id| !id.is_empty())
}

/// Argv starting — or resuming — the session of clash id `id`. One shape for
/// both, since Hermes resolves the title to the live tip or creates it.
pub fn launch_args(id: &str) -> Vec<String> {
    vec![
        "chat".into(),
        "-c".into(),
        session_title(id),
        "--create-if-missing".into(),
    ]
}

/// Argv resuming a session clash did not name (started outside clash).
pub fn resume_args(hermes_id: &str) -> Vec<String> {
    vec!["chat".into(), "--resume".into(), hermes_id.into()]
}

/// One `sessions` row, as much of it as clash reads.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub id: String,
    pub parent: Option<String>,
    pub end_reason: Option<String>,
    pub title: Option<String>,
    pub cwd: String,
    pub started_at: f64,
    pub last_activity_at: Option<f64>,
    pub ended_at: Option<f64>,
    pub message_count: i64,
}

/// One conversation: a root row and its compression continuations.
#[derive(Debug, Clone, PartialEq)]
pub struct Conversation {
    /// The id clash lists it under: the clash id for a session clash
    /// spawned, else the root's Hermes id.
    pub row_id: String,
    pub root: String,
    /// The live end of the compression chain.
    pub tip: String,
    /// Every Hermes id of the conversation, root first.
    pub chain: Vec<String>,
    /// The user's own title (never a `clash:` one).
    pub title: Option<String>,
    pub cwd: String,
    pub last_activity: f64,
    pub message_count: i64,
}

impl Conversation {
    pub fn spawned_by_clash(&self) -> bool {
        self.row_id != self.root
    }
}

fn compressed(r: &Row) -> bool {
    r.end_reason.as_deref() == Some("compression")
}

/// Pure: fold rows into conversations. A row continues its parent only when
/// the parent ended in a compression; anything else (a `/branch`, a missing
/// parent) starts a conversation of its own. At each step the chain prefers a
/// continuation that compressed again, then an open one, then the most
/// recently active — Hermes's own rule for the tip.
pub fn conversations(rows: &[Row]) -> Vec<Conversation> {
    use std::collections::HashMap;
    let by_id: HashMap<&str, &Row> = rows.iter().map(|r| (r.id.as_str(), r)).collect();
    let mut children: HashMap<&str, Vec<&Row>> = HashMap::new();
    for r in rows {
        if let Some(p) = r.parent.as_deref() {
            children.entry(p).or_default().push(r);
        }
    }
    let continues = |r: &Row| {
        r.parent
            .as_deref()
            .and_then(|p| by_id.get(p))
            .is_some_and(|p| compressed(p))
    };
    let activity = |r: &Row| r.last_activity_at.unwrap_or(r.started_at);
    let mut out = Vec::new();
    for root in rows.iter().filter(|r| !continues(r)) {
        let mut chain = vec![root];
        let mut cur = root;
        while compressed(cur) && chain.len() < 256 {
            let Some(next) = children.get(cur.id.as_str()).and_then(|kids| {
                kids.iter().copied().max_by(|a, b| {
                    (compressed(a), a.ended_at.is_none())
                        .cmp(&(compressed(b), b.ended_at.is_none()))
                        .then(activity(a).total_cmp(&activity(b)))
                })
            }) else {
                break;
            };
            chain.push(next);
            cur = next;
        }
        let root_title = root.title.as_deref().unwrap_or("");
        let row_id = clash_id_of_title(root_title)
            .unwrap_or(&root.id)
            .to_string();
        let title = chain
            .iter()
            .rev()
            .filter_map(|r| r.title.as_deref())
            .find(|t| !t.is_empty() && !t.starts_with(TITLE_PREFIX))
            .map(str::to_string);
        let cwd = chain
            .iter()
            .rev()
            .map(|r| r.cwd.as_str())
            .find(|c| !c.is_empty())
            .unwrap_or("")
            .to_string();
        out.push(Conversation {
            row_id,
            root: root.id.clone(),
            tip: cur.id.clone(),
            chain: chain.iter().map(|r| r.id.clone()).collect(),
            title,
            cwd,
            last_activity: chain.iter().map(|r| activity(r)).fold(0.0, f64::max),
            message_count: chain.iter().map(|r| r.message_count).sum(),
        });
    }
    out
}

/// The conversation a row id or any Hermes id of it names.
pub fn find<'a>(convs: &'a [Conversation], id: &str) -> Option<&'a Conversation> {
    convs
        .iter()
        .find(|c| c.row_id == id || c.chain.iter().any(|h| h == id))
}

/// Pure: whether `config.yaml` lists the clash plugin under
/// `plugins.enabled`, inline (`enabled: [a, b]`) or as a block list. A plain
/// line scan of the top-level `plugins:` mapping — enough for the two shapes
/// `hermes plugins enable` writes, and a miss only costs a Settings hint.
pub fn plugin_enabled_in(config_yaml: &str) -> bool {
    let unquote = |s: &str| s.trim().trim_matches(|c| c == '"' || c == '\'').to_string();
    let mut in_plugins = false;
    let mut in_enabled: Option<usize> = None;
    for line in config_yaml.lines() {
        let body = line.split(" #").next().unwrap_or(line);
        if body.trim().is_empty() || body.trim_start().starts_with('#') {
            continue;
        }
        let indent = body.len() - body.trim_start().len();
        let text = body.trim();
        if indent == 0 {
            in_plugins = text == "plugins:";
            in_enabled = None;
            continue;
        }
        if !in_plugins {
            continue;
        }
        if let Some(ind) = in_enabled {
            if let Some(item) = text.strip_prefix("- ") {
                if indent >= ind && unquote(item) == PLUGIN_NAME {
                    return true;
                }
                continue;
            }
            if indent <= ind {
                in_enabled = None;
            }
        }
        if let Some(rest) = text.strip_prefix("enabled:") {
            let rest = rest.trim();
            if let Some(list) = rest.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
                if list.split(',').any(|p| unquote(p) == PLUGIN_NAME) {
                    return true;
                }
            } else if rest.is_empty() {
                in_enabled = Some(indent);
            }
        }
    }
    false
}

/// Whether Hermes is set to load the clash status plugin.
pub fn plugin_enabled(home: &Path) -> bool {
    std::fs::read_to_string(home.join("config.yaml"))
        .map(|c| plugin_enabled_in(&c))
        .unwrap_or(false)
}

// ── IO ───────────────────────────────────────────────────────────────

/// Pure: the SQLite URI clash opens `db` with. A read-only connection cannot
/// create the `-shm` index a WAL database needs, and Hermes's gateway deletes
/// both `-wal` and `-shm` when it exits (it exits when idle) — so with no WAL
/// on disk the file is complete by itself and is opened `immutable`; with one,
/// the gateway is running and its `-shm` exists.
pub fn open_uri(db: &Path, wal_exists: bool) -> String {
    let path: String = db
        .to_string_lossy()
        .chars()
        .map(|c| match c {
            '%' => "%25".to_string(),
            '?' => "%3f".to_string(),
            '#' => "%23".to_string(),
            c => c.to_string(),
        })
        .collect();
    if wal_exists {
        format!("file:{path}?mode=ro")
    } else {
        format!("file:{path}?mode=ro&immutable=1")
    }
}

fn open(home: &Path) -> Option<rusqlite::Connection> {
    let db = state_db(home);
    if !db.is_file() {
        return None;
    }
    let wal = home.join("state.db-wal").exists();
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
        | rusqlite::OpenFlags::SQLITE_OPEN_URI;
    let conn = rusqlite::Connection::open_with_flags(open_uri(&db, wal), flags)
        .map_err(|e| tracing::debug!("hermes: {} not opened: {e}", db.display()))
        .ok()?;
    let _ = conn.busy_timeout(std::time::Duration::from_millis(250));
    Some(conn)
}

/// Every terminal (`cli`) session row that is not archived or hidden. A
/// schema this clash does not know reads as no sessions, never as an error.
pub fn load_rows(home: &Path) -> Vec<Row> {
    let Some(conn) = open(home) else {
        return Vec::new();
    };
    let sql = "SELECT id, parent_session_id, end_reason, title, COALESCE(cwd, ''), \
               started_at, last_activity_at, ended_at, COALESCE(message_count, 0) \
               FROM sessions WHERE source = 'cli' AND archived = 0 AND hidden = 0";
    let rows = conn.prepare(sql).and_then(|mut stmt| {
        stmt.query_map([], |r| {
            Ok(Row {
                id: r.get(0)?,
                parent: r.get(1)?,
                end_reason: r.get(2)?,
                title: r.get(3)?,
                cwd: r.get(4)?,
                started_at: r.get(5)?,
                last_activity_at: r.get(6)?,
                ended_at: r.get(7)?,
                message_count: r.get(8)?,
            })
        })
        .map(|it| it.flatten().collect::<Vec<_>>())
    });
    rows.unwrap_or_else(|e| {
        tracing::debug!("hermes: sessions not read: {e}");
        Vec::new()
    })
}

pub fn load_conversations(home: &Path) -> Vec<Conversation> {
    conversations(&load_rows(home))
}

/// The first thing the user said in a conversation.
pub fn first_prompt(home: &Path, root: &str) -> String {
    open(home)
        .and_then(|c| {
            c.query_row(
                "SELECT content FROM messages WHERE session_id = ?1 AND role = 'user' \
                 AND content IS NOT NULL ORDER BY id LIMIT 1",
                [root],
                |r| r.get::<_, String>(0),
            )
            .ok()
        })
        .unwrap_or_default()
}

/// The user and assistant turns of a conversation, oldest first, as
/// `(role, text, unix seconds)`.
pub fn load_messages(home: &Path, chain: &[String]) -> Vec<(String, String, f64)> {
    let Some(conn) = open(home) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for id in chain {
        let Ok(mut stmt) = conn.prepare(
            "SELECT role, content, timestamp FROM messages WHERE session_id = ?1 \
             AND role IN ('user', 'assistant') AND content IS NOT NULL AND content != '' \
             AND active = 1 ORDER BY id",
        ) else {
            return out;
        };
        let rows: Vec<(String, String, f64)> = match stmt.query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get(2)?))
        }) {
            Ok(it) => it.flatten().collect(),
            Err(_) => Vec::new(),
        };
        out.extend(rows);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, parent: Option<&str>, end: Option<&str>, title: Option<&str>) -> Row {
        Row {
            id: id.into(),
            parent: parent.map(str::to_string),
            end_reason: end.map(str::to_string),
            title: title.map(str::to_string),
            ended_at: end.map(|_| 2.0),
            started_at: 1.0,
            ..Row::default()
        }
    }

    #[test]
    fn a_clash_title_round_trips_and_nothing_else_names_an_id() {
        assert_eq!(session_title("abc"), "clash:abc");
        assert_eq!(clash_id_of_title("clash:abc"), Some("abc"));
        assert_eq!(clash_id_of_title("clash:"), None);
        assert_eq!(clash_id_of_title("Getting started"), None);
        assert_eq!(
            launch_args("abc"),
            ["chat", "-c", "clash:abc", "--create-if-missing"]
        );
    }

    /// A compression rotates the conversation onto a new id; the list must
    /// still show one row, under the clash id, resuming at the tip.
    #[test]
    fn a_compressed_conversation_is_one_row_under_its_clash_id() {
        let mut tip = row("local-c", Some("local-b"), None, Some("clash:X #3"));
        tip.cwd = "/w/repo".into();
        tip.last_activity_at = Some(9.0);
        let rows = vec![
            row("local-a", None, Some("compression"), Some("clash:X")),
            row("local-b", Some("local-a"), Some("compression"), None),
            tip,
        ];
        let c = conversations(&rows);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].row_id, "X");
        assert!(c[0].spawned_by_clash());
        assert_eq!(c[0].tip, "local-c");
        assert_eq!(c[0].chain, ["local-a", "local-b", "local-c"]);
        assert_eq!(c[0].cwd, "/w/repo");
        assert_eq!(c[0].last_activity, 9.0);
        assert_eq!(c[0].title, None, "a clash: title is not the user's");
        assert_eq!(find(&c, "local-b").map(|c| c.row_id.as_str()), Some("X"));
    }

    /// A `/branch` child is a conversation of its own, and a session started
    /// outside clash is listed under its Hermes id.
    #[test]
    fn a_branch_is_its_own_conversation() {
        let rows = vec![
            row("local-a", None, None, Some("Refactor")),
            row("local-b", Some("local-a"), None, None),
        ];
        let c = conversations(&rows);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].row_id, "local-a");
        assert!(!c[0].spawned_by_clash());
        assert_eq!(c[0].title.as_deref(), Some("Refactor"));
        assert_eq!(c[1].row_id, "local-b");
    }

    #[test]
    fn the_tip_prefers_a_continuation_that_compressed_again() {
        let mut open_late = row("late", Some("r"), None, None);
        open_late.last_activity_at = Some(50.0);
        let rows = vec![
            row("r", None, Some("compression"), Some("clash:Y")),
            open_late,
            row("again", Some("r"), Some("compression"), None),
            row("end", Some("again"), None, None),
        ];
        assert_eq!(conversations(&rows)[0].tip, "end");
    }

    #[test]
    fn the_plugin_counts_as_enabled_in_either_list_shape_only() {
        assert!(plugin_enabled_in(
            "plugins:\n  enabled: [a, clash-status]\n"
        ));
        assert!(plugin_enabled_in(
            "model:\n  default: x\nplugins:\n  enabled:\n    - other\n    - \"clash-status\"\n"
        ));
        assert!(plugin_enabled_in(
            "plugins:\n  enabled:\n  - clash-status\n"
        ));
        assert!(!plugin_enabled_in(
            "plugins:\n  disabled: [clash-status]\n  enabled: []\n"
        ));
        assert!(!plugin_enabled_in(
            "plugins:\n  enabled:\n    - a\n  disabled:\n    - clash-status\n"
        ));
        assert!(!plugin_enabled_in("other:\n  enabled: [clash-status]\n"));
        assert!(!plugin_enabled_in(""));
    }

    /// The regression: with Hermes's gateway gone (it exits when idle) its
    /// WAL files are deleted, and a plain read-only open then fails — every
    /// Hermes session vanished from the list until the gateway came back.
    #[test]
    fn a_wal_store_is_readable_with_and_without_its_gateway() {
        let dir = tempfile::TempDir::new().unwrap();
        let writer = rusqlite::Connection::open(dir.path().join("state.db")).unwrap();
        writer
            .execute_batch(
                "PRAGMA journal_mode = WAL;
                 CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT, parent_session_id TEXT, \
                 end_reason TEXT, title TEXT, cwd TEXT, started_at REAL, last_activity_at REAL, \
                 ended_at REAL, message_count INTEGER, archived INTEGER DEFAULT 0, \
                 hidden INTEGER DEFAULT 0);
                 INSERT INTO sessions (id, source, title, started_at) VALUES ('local-1', 'cli', 'clash:W', 1.0);",
            )
            .unwrap();
        assert!(dir.path().join("state.db-wal").exists());
        assert_eq!(
            load_conversations(dir.path())[0].row_id,
            "W",
            "gateway running"
        );
        drop(writer);
        assert!(!dir.path().join("state.db-wal").exists());
        assert_eq!(
            load_conversations(dir.path())[0].row_id,
            "W",
            "gateway gone"
        );
        assert_eq!(
            open_uri(Path::new("/a b/50%?#/state.db"), false),
            "file:/a b/50%25%3f%23/state.db?mode=ro&immutable=1"
        );
    }

    /// The real store: a read-only open of a WAL database Hermes writes.
    #[test]
    fn rows_are_read_from_a_state_db() {
        let dir = tempfile::TempDir::new().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("state.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, \
             parent_session_id TEXT, end_reason TEXT, title TEXT, cwd TEXT, \
             started_at REAL NOT NULL, last_activity_at REAL, ended_at REAL, \
             message_count INTEGER DEFAULT 0, archived INTEGER NOT NULL DEFAULT 0, \
             hidden INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT, role TEXT, \
             content TEXT, timestamp REAL, active INTEGER DEFAULT 1);
             INSERT INTO sessions (id, source, title, cwd, started_at) VALUES \
               ('local-1', 'cli', 'clash:K', '/p', 1.0), \
               ('local-2', 'gui', 'Desktop chat', '', 1.0), \
               ('local-3', 'cli', 'gone', '', 1.0);
             UPDATE sessions SET archived = 1 WHERE id = 'local-3';
             INSERT INTO messages (session_id, role, content, timestamp) VALUES \
               ('local-1', 'user', 'hello', 1.0), ('local-1', 'assistant', 'hi', 2.0);",
        )
        .unwrap();
        drop(conn);
        let convs = load_conversations(dir.path());
        assert_eq!(
            convs.len(),
            1,
            "terminal sessions only, archived ones hidden"
        );
        assert_eq!(convs[0].row_id, "K");
        assert_eq!(first_prompt(dir.path(), "local-1"), "hello");
        let msgs = load_messages(dir.path(), &convs[0].chain);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].0, "assistant");
        assert!(load_conversations(&dir.path().join("missing")).is_empty());
    }
}
