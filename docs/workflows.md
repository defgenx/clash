# Workflows — file contract

The Workflows feature manages a plan → review → implement → PR pipeline per
item. clash's GUI renders and mutates the files below; a Claude Code agent
(driven by the `clash-workflow` skill) co-edits them. This document is the
contract both sides follow.

## Layout

Rooted at the workflows directory — default `~/.claude/clash/workflows`,
overridable via `workflows_dir` in `config.toml` or the GUI Settings panel.
Deliberately independent of the scratch tree: scratches are free-form notes,
workflows are a structured store.

```
<root>/<project>/<slug>/
├── meta.json          # status, PR info, session, iteration, timestamps
├── plan.md            # the plan (freely editable markdown)
├── review.md          # append-only iteration audit trail
├── annotations.json   # line-level diff comments
└── history/<NNN>/     # per-iteration snapshots (diff.patch + annotations.json)
```

`(project, slug)` — the directory path — is the identity; `meta.json` never
overrides it. All JSON is lenient: unknown fields must be preserved on
read-modify-write (clash uses `#[serde(flatten)]` extras; the agent must
merge, never rewrite from scratch).

## Statuses

`draft → planning → plan-review → changes-requested → implementing →
diff-review → pr-draft → pr-ready → done`, plus `abandoned` (from anywhere).
Kebab-case strings in `meta.json`. Decision states (`plan-review`,
`diff-review`, `pr-draft`) notify the user.

### Status ownership

- **Agent-owned transitions**: `planning → plan-review`,
  `changes-requested → plan-review` (plan revision round),
  `changes-requested → implementing → diff-review → pr-draft`.
- **Everything else is clash-owned** (buttons in the GUI): approve, request
  changes, mark PR ready, done, abandon, reopen.

## The agent contract (clash-workflow skill)

The kickoff prompt is:
`Use the clash-workflow skill. Workflow item directory: <abs path>. Phase: <plan|revise|implement>.`

On every run, the agent first reads: `meta.json`, `plan.md`, `review.md`
(top-to-bottom — it is the accumulated decision history), the `open`
annotations in `annotations.json`, and the latest `history/<NNN>/diff.patch`
for context.

- **Phase `plan`**: write/overwrite `plan.md`; finish by setting
  `meta.json.status = "plan-review"`.
- **Phase `revise`**: if the review round was about the plan, revise
  `plan.md` and finish with `"plan-review"`. If it was about code, behave
  like `implement`.
- **Phase `implement`**: set `"implementing"` while working; address every
  `open` annotation — set it `"addressed"` with a short resolution appended
  to its `replies`, or `"wontfix"` with a justification; commit on the item
  branch (never push, never `--no-verify`); optionally create a draft PR
  (`gh pr create --draft`) and write its URL into `meta.json.pr.url`;
  finish with `"diff-review"` (or `"pr-draft"` when a PR was created).

### Hard rules

- Never touch `history/` and never change `iteration` — clash owns both
  (they are written atomically by the request-changes flow).
- Write `annotations.json` **only** while status is `changes-requested` or
  `implementing` — during review phases the GUI owns the file (this phase
  split is what makes concurrent writes safe).
- Read-modify-write `meta.json`; keep unknown fields.
- Never rewrite `review.md` history — it is append-only (clash appends the
  `## Iteration N` sections; the agent only reads it).

## PR integration

clash records the PR in `meta.json.pr` (`url`, `number`, `draft`, `state`,
`lastCheckedAt`) via the `gh` CLI. `state == "MERGED"` observed on refresh
moves the item to `done`. The agent may create the draft PR itself — writing
`pr.url` is enough; clash fills the rest on the next refresh.
