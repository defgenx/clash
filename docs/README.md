# clash documentation

Start with the project [README](../README.md). The guides below go into detail.

## User guides

| Document | What it covers |
|---|---|
| [gui.md](gui.md) | The desktop app: workspaces, panes and tabs, sessions, terminals, browser and links, Files panel, scratches, teams, attention inbox, queued follow-ups, settings, updates, keyboard shortcuts |
| [workflows-guide.md](workflows-guide.md) | Workflows from the user's side: the pipeline, action bar, reviews, explanations, check runs, PRs, multi-repo work, ticket trackers and sharing, models and skills |
| [tui.md](tui.md) | The terminal UI: every keybinding and `:` command, attached mode, scratches, opening sessions in external panes |
| [sessions.md](sessions.md) | The session model shared by both frontends: status detection, session sources, wild sessions, OMP sessions, session presets |
| [config-file.md](config-file.md) | The `config.toml` file: location, `clash config`, layers, every key with its default, live-reload behaviour |
| [data.md](data.md) | Every file and directory clash reads or writes |

## Reference

| Document | What it covers |
|---|---|
| [workflows.md](workflows.md) | The workflow file contract the GUI and the agents' skills follow, and the design behind each rule |
| [configuration.md](configuration.md) | Configuration internals: the schema table, provenance, the write path, live reload, migrations |
| [hooks.md](hooks.md) | How status hooks are registered (`claude --settings`), the events they record, and the OMP status extension |
| [development.md](development.md) | Building, tests, lint, releases and the architecture |
