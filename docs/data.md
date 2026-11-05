# Files clash reads and writes

## Claude Code's data

clash reads directly from Claude Code's filesystem:

```
~/.claude/
├── projects/{name}/
│   ├── sessions-index.json            # Session index with summaries
│   ├── {session-id}.jsonl             # Conversation log
│   └── {session-id}/subagents/        # Subagent transcripts
├── teams/{name}/config.json           # Team config + members
│                                       #   (Claude's auto session-* teams are hidden)
├── teams/{name}/inboxes/{agent}.json  # Per-agent inboxes
└── tasks/{team-name}/{id}.json        # Tasks
```

OMP sessions are read from `~/.omp/agent/sessions/` (see
[sessions.md → OMP sessions](sessions.md#omp-sessions)).

Outside its own `~/.claude/clash/` directory, clash writes only where you ask it
to — team configs, inboxes and tasks when you edit them — plus its embedded
workflow skills under `~/.claude/skills/<name>/` (with a `.clash-skills.json`
manifest; also under `~/.omp/agent/skills/` when that directory exists). It
registers nothing in your Claude settings files; older versions' entry in
`~/.claude/settings.local.json` is withdrawn at startup. See
[hooks.md](hooks.md) for how status hooks are registered instead.

## clash's own state

clash maintains its own state in `~/.claude/clash/`:

```
~/.claude/clash/
├── hooks/status-hook.sh               # Lifecycle hook script
├── hooks/settings.json                # Hook registration, passed to `claude --settings`
├── hooks/omp-status.js                # OMP status extension, loaded with `omp -e`
├── status/{session-id}                # Instant status from hooks
├── names/{session-id}                 # Session display names
├── project-names/{encoded-cwd}        # Project-to-name mapping
├── sessions.json                      # Session registry (+ sessions.json.bak)
├── ui_state.json                      # Persisted TUI state (nav, selection, filters) — saved
│                                       #   continuously so any exit resumes where you were
├── scratch/                           # Scratch notes — a nested tree of
│   ├── {name}.md                       #   free-form text files and
│   └── {folder}/{name}.md              #   user-created folders
├── workflows/<project>/<item>/        # Workflow items (see the Workflows guide → Storage)
├── drops/                             # Copies of files dropped on a GUI terminal (kept 7 days)
└── share/                             # Payloads handed to share sessions
```

Workflow item layout: [workflows-guide.md → Storage](workflows-guide.md#storage);
the full file contract: [workflows.md](workflows.md).

## Config and data directories

Outside `~/.claude`, in the platform's config and data directories
(`~/.config/clash` and `~/.local/share/clash` on Linux, `~/Library/Application
Support/clash` on macOS):

```
config.toml                # the shared configuration (see config-file.md)
presets.json               # global session presets
gui-state.json             # GUI workspaces, layout and GUI-only settings
clash.log                  # log, rotated after 24h (CLASH_LOG_RETENTION_HOURS)
daemon-<pid>.sock          # one per running instance; `clash attach` finds the owner
```
