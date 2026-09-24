//! What to exec for a session, per agent CLI. Every spawn site — both
//! frontends, the workflow launchers — builds its command line here, so the
//! Claude-vs-OMP differences live in one file:
//!
//! | | Claude Code | OMP |
//! |---|---|---|
//! | new session under clash id `X` | `--session-id X` | `--resume <bucket>/<ts>_X.jsonl` (omp creates it there) |
//! | resume | `--resume <conversation>` (forks; chased in the registry) | `--resume <file>` (appends in place) |
//! | initial prompt | positional | positional |
//!
//! Status hooks are injected by the daemon at spawn (`daemon::session`), not
//! here, so a bare `claude`/`omp` from any caller still gets them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::entities::AgentKind;
use crate::infrastructure::config::Config;
use crate::infrastructure::hooks::registry::{self, ClashSession};
use crate::infrastructure::omp;

/// Binary + argv for one daemon spawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Launch {
    pub bin: String,
    pub args: Vec<String>,
}

/// Where each agent lives on this machine, from config.
#[derive(Debug, Clone)]
pub struct Agents {
    pub claude_bin: String,
    pub omp_bin: String,
    /// `~/.claude/projects` — Claude transcripts.
    pub claude_projects_dir: PathBuf,
    /// `~/.omp/agent` — OMP sessions and skills.
    pub omp_dir: PathBuf,
}

// ── Pure argv ────────────────────────────────────────────────────────

/// Argv starting a brand-new session. `omp_file` is where omp must create it.
pub fn fresh_args(agent: AgentKind, id: &str, omp_file: &Path) -> Vec<String> {
    match agent {
        AgentKind::Claude => vec!["--session-id".into(), id.into()],
        AgentKind::Omp => vec!["--resume".into(), omp_file.to_string_lossy().into_owned()],
    }
}

/// Argv resuming `conversation` (Claude) / the transcript at `omp_file` (OMP).
pub fn resume_args(agent: AgentKind, conversation: &str, omp_file: &Path) -> Vec<String> {
    match agent {
        AgentKind::Claude => vec!["--resume".into(), conversation.into()],
        AgentKind::Omp => vec!["--resume".into(), omp_file.to_string_lossy().into_owned()],
    }
}

/// Append a model and an initial prompt. Both agents take the prompt as the
/// last positional argument; an empty model leaves the agent's own default.
///
/// `dead_code` is allowed because the only caller is the sibling `clash-gui`
/// crate (workflow launchers); the TUI launches no prompted sessions.
#[allow(dead_code)]
pub fn with_prompt(mut args: Vec<String>, model: Option<&str>, prompt: &str) -> Vec<String> {
    if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
        args.push("--model".into());
        args.push(m.into());
    }
    args.push(prompt.into());
    args
}

impl Agents {
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            claude_bin: cfg.general.claude_bin.clone(),
            omp_bin: cfg.general.omp_bin.clone(),
            claude_projects_dir: cfg.claude_dir().join("projects"),
            omp_dir: cfg.omp_dir(),
        }
    }

    pub fn bin(&self, agent: AgentKind) -> String {
        match agent {
            AgentKind::Claude => self.claude_bin.clone(),
            AgentKind::Omp => self.omp_bin.clone(),
        }
    }

    /// A brand-new session under clash id `id`, started in `cwd`.
    pub fn fresh(&self, agent: AgentKind, id: &str, cwd: &str) -> Launch {
        let omp_file = match agent {
            AgentKind::Omp => omp::new_session_path(&self.omp_dir, cwd, id),
            AgentKind::Claude => PathBuf::new(),
        };
        Launch {
            bin: self.bin(agent),
            args: fresh_args(agent, id, &omp_file),
        }
    }

    /// Relaunch `session_id` after its process died: resolve the id forward to
    /// the current conversation (lineage, plus Claude's resume forks), record
    /// it, and resume it when a transcript exists — otherwise start fresh
    /// under the same id, since `claude --resume` on a missing transcript
    /// exits into a dead terminal.
    pub fn relaunch(
        &self,
        registry: &HashMap<String, ClashSession>,
        session_id: &str,
        cwd: Option<&str>,
    ) -> Launch {
        let cwd_str = cwd.unwrap_or("");
        self.relaunch_with(registry, session_id, cwd, |conv| {
            claude_transcript_exists(&self.claude_projects_dir, cwd_str, conv)
        })
    }

    /// [`Self::relaunch`] with the caller's own "does this Claude
    /// conversation have a transcript" check — the GUI also looks under the
    /// project dirs its last session list recorded.
    pub fn relaunch_with(
        &self,
        registry: &HashMap<String, ClashSession>,
        session_id: &str,
        cwd: Option<&str>,
        claude_has_transcript: impl Fn(&str) -> bool,
    ) -> Launch {
        // An unregistered id (a wild takeover) is whichever agent holds its
        // transcript.
        let agent = registry::registered_agent(registry, session_id).unwrap_or_else(|| {
            if omp::find_session_file(&self.omp_dir, session_id).is_some() {
                AgentKind::Omp
            } else {
                AgentKind::Claude
            }
        });
        let cwd = cwd.unwrap_or("");
        let conversation = registry::resolve_latest_conversation(
            registry,
            &self.claude_projects_dir,
            cwd,
            session_id,
        );
        registry::record_resumed_conversation(session_id, &conversation);
        match agent {
            AgentKind::Claude => {
                if claude_has_transcript(&conversation) {
                    Launch {
                        bin: self.bin(agent),
                        args: resume_args(agent, &conversation, Path::new("")),
                    }
                } else {
                    self.fresh(agent, session_id, cwd)
                }
            }
            AgentKind::Omp => match omp::find_session_file(&self.omp_dir, &conversation) {
                Some(file) => Launch {
                    bin: self.bin(agent),
                    args: resume_args(agent, &conversation, &file),
                },
                None => self.fresh(agent, session_id, cwd),
            },
        }
    }
}

/// Does a resumable Claude transcript exist? `claude --resume <id>` needs a
/// non-empty `<id>.jsonl` (otherwise it exits 1 into a dead terminal), and
/// Claude encodes the project dir from the *canonical* cwd (`/tmp` →
/// `/private/tmp`), so both spellings are checked.
pub fn claude_transcript_exists(projects: &Path, cwd: &str, session_id: &str) -> bool {
    use crate::infrastructure::fs::backend::encode_project_dir;
    let mut dirs = Vec::new();
    if !cwd.is_empty() {
        dirs.push(encode_project_dir(cwd));
        if let Ok(canon) = std::fs::canonicalize(cwd) {
            let enc = encode_project_dir(&canon.to_string_lossy());
            if !dirs.contains(&enc) {
                dirs.push(enc);
            }
        }
    }
    dirs.iter().any(|d| {
        let f = projects.join(d).join(format!("{session_id}.jsonl"));
        std::fs::metadata(&f).map(|m| m.len() > 0).unwrap_or(false)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_new_sessions_pin_the_id_and_omp_ones_pin_the_file() {
        let f = Path::new("/s/-p/2026_X.jsonl");
        assert_eq!(fresh_args(AgentKind::Claude, "X", f), ["--session-id", "X"]);
        assert_eq!(
            fresh_args(AgentKind::Omp, "X", f),
            ["--resume", "/s/-p/2026_X.jsonl"]
        );
    }

    #[test]
    fn resume_uses_the_conversation_for_claude_and_the_file_for_omp() {
        let f = Path::new("/s/-p/2026_Y.jsonl");
        assert_eq!(resume_args(AgentKind::Claude, "Y", f), ["--resume", "Y"]);
        assert_eq!(
            resume_args(AgentKind::Omp, "Y", f),
            ["--resume", "/s/-p/2026_Y.jsonl"]
        );
    }

    #[test]
    fn prompt_is_last_and_an_empty_model_is_omitted() {
        let base = vec!["--session-id".to_string(), "X".to_string()];
        assert_eq!(
            with_prompt(base.clone(), Some("opus"), "do it"),
            ["--session-id", "X", "--model", "opus", "do it"]
        );
        assert_eq!(
            with_prompt(base, Some("  "), "do it"),
            ["--session-id", "X", "do it"]
        );
    }

    #[test]
    fn a_claude_transcript_is_resumable_only_when_non_empty() {
        use crate::infrastructure::fs::backend::encode_project_dir;
        let dir = tempfile::TempDir::new().unwrap();
        let cwd = "/Users/me/proj";
        let sid = "019ed532-ade6-73a1-acfb-6a58581065c7";
        let proj_dir = dir.path().join(encode_project_dir(cwd));
        std::fs::create_dir_all(&proj_dir).unwrap();
        assert!(!claude_transcript_exists(dir.path(), cwd, sid));
        let jsonl = proj_dir.join(format!("{sid}.jsonl"));
        std::fs::write(&jsonl, b"").unwrap();
        assert!(!claude_transcript_exists(dir.path(), cwd, sid));
        std::fs::write(&jsonl, b"{\"type\":\"user\"}\n").unwrap();
        assert!(claude_transcript_exists(dir.path(), cwd, sid));
        assert!(!claude_transcript_exists(dir.path(), "/other/dir", sid));
    }

    #[test]
    fn omp_fresh_launch_creates_the_file_in_omps_bucket_for_the_cwd() {
        let agents = Agents {
            claude_bin: "claude".into(),
            omp_bin: "/opt/omp".into(),
            claude_projects_dir: PathBuf::from("/nope"),
            omp_dir: PathBuf::from("/omp-agent"),
        };
        let home = dirs::home_dir().unwrap();
        let cwd = home.join("some-project-that-does-not-exist");
        let l = agents.fresh(AgentKind::Omp, "abc", &cwd.to_string_lossy());
        assert_eq!(l.bin, "/opt/omp");
        assert_eq!(l.args[0], "--resume");
        let p = Path::new(&l.args[1]);
        assert!(p.starts_with("/omp-agent/sessions"));
        assert_eq!(
            omp::session_id_from_file_name(p.file_name().unwrap().to_str().unwrap()),
            Some("abc")
        );
    }
}
