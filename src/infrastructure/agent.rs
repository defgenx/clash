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
//!
//! A **workflow** session's models are applied here too
//! ([`Agents::with_workflow_models`]) — at its first launch *and* at every
//! relaunch, since a resume keeps none of the original command line or env.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::entities::AgentKind;
use crate::infrastructure::config::Config;
use crate::infrastructure::hooks::registry::{self, ClashSession};
use crate::infrastructure::omp;

/// Binary, argv and extra env for one daemon spawn.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Launch {
    pub bin: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// The `workflows.*` model settings, per harness — what a workflow session
/// is launched with.
#[derive(Debug, Clone, Default)]
pub struct WorkflowModels {
    pub delegation: String,
    pub claude_lead: String,
    pub claude_subagent: String,
    pub omp_lead: String,
    pub omp_subagent: String,
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
    pub workflow: WorkflowModels,
    /// Where the OMP subagent overlay is written (clash's data dir).
    pub omp_overlay_path: PathBuf,
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

/// Append an initial prompt. Both agents take it as the last positional
/// argument, so it goes after every flag (`--model` included).
///
/// `dead_code` is allowed because the only caller is the sibling `clash-gui`
/// crate (workflow launchers); the TUI launches no prompted sessions.
#[allow(dead_code)]
pub fn with_prompt(mut args: Vec<String>, prompt: &str) -> Vec<String> {
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
            workflow: WorkflowModels {
                delegation: cfg.workflows.delegation.clone(),
                claude_lead: cfg.workflows.lead_model.clone(),
                claude_subagent: cfg.workflows.subagent_model.clone(),
                omp_lead: cfg.workflows.omp_model.clone(),
                omp_subagent: cfg.workflows.omp_subagent_model.clone(),
            },
            omp_overlay_path: Config::clash_data_dir()
                .join("omp")
                .join("workflow-models.yml"),
        }
    }

    /// Apply the workflow models for `agent` to `launch`: the lead model as
    /// `--model`, and the subagent pin as env (Claude Code: model + force
    /// flag; OMP: a config overlay setting its `task` role). Called for a
    /// workflow session's first launch and for every relaunch of one.
    pub fn with_workflow_models(&self, agent: AgentKind, mut launch: Launch) -> Launch {
        use crate::application::workflow::{delegation_env, launch_model, Delegation};
        let w = &self.workflow;
        if let Some(m) = launch_model(agent, &w.claude_lead, &w.omp_lead) {
            launch.args.push("--model".into());
            launch.args.push(m.to_string());
        }
        let delegation = Delegation::from_settings(
            &w.delegation,
            match agent {
                AgentKind::Claude => &w.claude_subagent,
                AgentKind::Omp => &w.omp_subagent,
            },
        );
        let overlay = match agent {
            AgentKind::Omp if delegation.team && !delegation.subagent_model.is_empty() => {
                self.write_omp_overlay(delegation.subagent_model)
            }
            _ => None,
        };
        let inherited =
            std::env::var(crate::application::workflow::OMP_CONFIG_FILES_ENV).unwrap_or_default();
        launch.env.extend(delegation_env(
            agent,
            &delegation,
            overlay.as_deref(),
            &inherited,
        ));
        launch
    }

    /// Write the OMP subagent overlay when its content changed; `None` when it
    /// cannot be written, which costs the pin and never the session (omp
    /// refuses to start on a missing `PI_CONFIG_FILES` entry).
    fn write_omp_overlay(&self, model: &str) -> Option<String> {
        let path = &self.omp_overlay_path;
        let body = crate::application::workflow::omp_subagent_overlay(model);
        if std::fs::read_to_string(path).ok().as_deref() != Some(body.as_str()) {
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|_| {
                    crate::infrastructure::fs::atomic::write_atomic(path, body.as_bytes())
                });
            if let Err(e) = written {
                tracing::warn!("omp subagent overlay not written ({}): {e}", path.display());
                return None;
            }
        }
        Some(path.to_string_lossy().into_owned())
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
            ..Launch::default()
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
        let launch = match agent {
            AgentKind::Claude => {
                if claude_has_transcript(&conversation) {
                    Launch {
                        bin: self.bin(agent),
                        args: resume_args(agent, &conversation, Path::new("")),
                        ..Launch::default()
                    }
                } else {
                    self.fresh(agent, session_id, cwd)
                }
            }
            AgentKind::Omp => match omp::find_session_file(&self.omp_dir, &conversation) {
                Some(file) => Launch {
                    bin: self.bin(agent),
                    args: resume_args(agent, &conversation, &file),
                    ..Launch::default()
                },
                None => self.fresh(agent, session_id, cwd),
            },
        };
        // A resume keeps none of the first launch's flags or env, so a
        // workflow session would otherwise come back on the agent's default
        // model with its subagents unpinned.
        if registry::is_workflow_session(registry, session_id) {
            self.with_workflow_models(agent, launch)
        } else {
            launch
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
    fn the_prompt_is_the_last_argument() {
        let base = vec!["--model".to_string(), "opus".to_string()];
        assert_eq!(with_prompt(base, "do it"), ["--model", "opus", "do it"]);
    }

    fn agents_with(dir: &Path, w: WorkflowModels) -> Agents {
        Agents {
            claude_bin: "claude".into(),
            omp_bin: "omp".into(),
            claude_projects_dir: dir.join("projects"),
            omp_dir: dir.join("omp"),
            workflow: w,
            omp_overlay_path: dir.join("clash").join("workflow-models.yml"),
        }
    }

    fn models() -> WorkflowModels {
        WorkflowModels {
            delegation: "team".into(),
            claude_lead: "claude-opus-5-5".into(),
            claude_subagent: "claude-sonnet-5-5".into(),
            omp_lead: "glm-5".into(),
            omp_subagent: "glm-5-flash".into(),
        }
    }

    /// Each harness gets its own lead and its own subagent pin.
    #[test]
    fn workflow_models_are_applied_per_harness() {
        let dir = tempfile::TempDir::new().unwrap();
        let a = agents_with(dir.path(), models());
        let c = a.with_workflow_models(AgentKind::Claude, Launch::default());
        assert_eq!(c.args, ["--model", "claude-opus-5-5"]);
        assert_eq!(c.env["CLAUDE_CODE_SUBAGENT_MODEL"], "claude-sonnet-5-5");
        assert_eq!(c.env["CLAUDE_CODE_SUBAGENT_MODEL_FORCE"], "1");

        let o = a.with_workflow_models(AgentKind::Omp, Launch::default());
        assert_eq!(o.args, ["--model", "glm-5"]);
        let overlay = &o.env["PI_CONFIG_FILES"];
        assert!(overlay.ends_with("workflow-models.yml"), "{overlay}");
        let body = std::fs::read_to_string(overlay).unwrap();
        assert!(body.contains("task: \"glm-5-flash\""), "{body}");
        assert!(!o.env.contains_key("CLAUDE_CODE_SUBAGENT_MODEL"));

        // Solo pins nothing, empty leads leave the agent's own default.
        let bare = agents_with(
            dir.path(),
            WorkflowModels {
                delegation: "solo".into(),
                ..WorkflowModels::default()
            },
        );
        for agent in [AgentKind::Claude, AgentKind::Omp] {
            assert_eq!(
                bare.with_workflow_models(agent, Launch::default()),
                Launch::default()
            );
        }
    }

    /// The regression: a resumed workflow session got a bare `--resume`.
    #[test]
    fn a_relaunched_workflow_session_gets_its_models_back_and_others_do_not() {
        use crate::infrastructure::hooks::registry::ClashSession;
        let dir = tempfile::TempDir::new().unwrap();
        let a = agents_with(dir.path(), models());
        let entry = |id: &str, workflow: bool| ClashSession {
            session_id: id.into(),
            name: id.into(),
            cwd: "/p".into(),
            claude_session_id: id.into(),
            created_at: String::new(),
            source_branch: None,
            previous_ids: Vec::new(),
            agent: AgentKind::Claude,
            workflow,
        };
        let reg: HashMap<String, ClashSession> = [
            ("wf".to_string(), entry("wf", true)),
            ("plain".to_string(), entry("plain", false)),
        ]
        .into();
        let wf = a.relaunch_with(&reg, "wf", Some("/p"), |_| true);
        assert_eq!(wf.args, ["--resume", "wf", "--model", "claude-opus-5-5"]);
        assert_eq!(wf.env["CLAUDE_CODE_SUBAGENT_MODEL_FORCE"], "1");
        // Starting fresh because the transcript is gone still pins them.
        let fresh = a.relaunch_with(&reg, "wf", Some("/p"), |_| false);
        assert!(fresh
            .args
            .ends_with(&["--model".into(), "claude-opus-5-5".into()]));
        let plain = a.relaunch_with(&reg, "plain", Some("/p"), |_| true);
        assert_eq!(plain.args, ["--resume", "plain"]);
        assert!(plain.env.is_empty());
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
            workflow: WorkflowModels::default(),
            omp_overlay_path: PathBuf::from("/nope/overlay.yml"),
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
