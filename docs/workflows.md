# Workflows — file contract

The Workflows feature manages a plan → review → implement → PR pipeline per
item. clash's GUI renders and mutates the files below; Claude Code agents co-edit
them — the executor (`clash-workflow` skill) does the work, the reviewers
(`clash-plan-review` for plans, `clash-code-review` for diffs) judge it. This
document is the contract all sides follow.

## Layout

Rooted at the workflows directory — default `~/.claude/clash/workflows`,
overridable via `workflows_dir` in `config.toml` or the GUI Settings panel.
Deliberately independent of the scratch tree: scratches are free-form notes,
workflows are a structured store.

```
<root>/<project>/<slug>/
├── meta.json          # status, PR info, session, iteration, review round, timestamps
├── plan.md            # the plan (freely editable markdown)
├── review.md          # append-only iteration audit trail (clash writes, agent reads)
├── agent-review.md    # append-only agent review rounds (agent writes, clash renders)
├── structure.md       # the explain round's document (clash-explain overwrites, Structure tab renders)
├── annotations.json   # line-level diff comments
└── history/<NNN>/     # per-iteration snapshots (diff.patch + plan.md + annotations.json)
```

`review.md` and `agent-review.md` are deliberately two files: the first is
clash's record of **human** decisions, the second the reviewer's own findings.
One file would make ownership ambiguous exactly where concurrent writes happen.

`(project, slug)` — the directory path — is the identity; `meta.json` never
overrides it. All JSON is lenient: unknown fields must be preserved on
read-modify-write (clash uses `#[serde(flatten)]` extras; the agent must
merge, never rewrite from scratch). Three fields are clash-owned per-item settings
(the item ⚙ Settings tab; agents never read or write them):
`meta.bareSessionNames` switches that item's agent sessions from the
title-prefixed default (`Auth refactor · implement`) to bare job names;
`meta.prSkill` overrides the global PR skill for this item (`none`
disables); `meta.interactionDefault` (`interactive` | `autonomous`, absent =
ask) pre-answers the skills' opening question for rounds launched without an
explicit choice.

## Statuses

`draft → planning → plan-review → changes-requested → implementing →
diff-review → pr-draft → pr-ready → done`, plus `abandoned` (from anywhere) and
`reviewing` (a side-trip off any decision state, see below). Kebab-case strings
in `meta.json`. Decision states (`plan-review`, `diff-review`, `pr-draft`)
notify the user; `reviewing` does not — an agent is working, there is nothing to
decide yet.

The `pr-*` stages are **optional**. Approving at `diff-review` may go straight to
`done`, so a repo that merges to its default branch without a PR is never forced
through a draft-PR step it has no use for. `diff-review → pr-draft` remains for
items that do use a PR.

The `pr-*` stages are also **not a dead end**: `pr-draft → changes-requested`
and `pr-ready → changes-requested` are legal, because review feedback (agent
rounds, GitHub review comments) keeps arriving after the PR exists and must be
able to become the next fix round. When a fix round runs on an item that
already has a PR, the executor pushes after committing — the branch is
published, and a fix that only commits locally leaves the PR silently stale.

### Status ownership

- **Agent-owned transitions**: `planning → plan-review`,
  `changes-requested → plan-review` (plan revision round),
  `changes-requested → implementing → diff-review → pr-draft`,
  `implementing → plan-review` (a `revise` launch parks the item in
  `implementing`; a revision that only touched the plan hands back to
  `plan-review`), and `reviewing → <the status the round started in>`.
- **Everything else is clash-owned** (buttons in the GUI): approve, request
  changes, mark PR ready, done, abandon, reopen, and launching a review round.

### The change-request note is a prompt

`review.md`'s latest `## Iteration N` section is the first thing the executor
skill reads, so the human's note *is* that round's instructions. clash composes it
in a markdown editor (template, preview, the queued annotations shown alongside,
draft kept on dismiss) rather than a single-line input, and writes it verbatim —
the agent gets exactly what the human wrote, followed by the auto-generated
`### Open annotations` list.

A round must carry either a note or at least one open annotation; clash refuses
to send one with neither, since it would give the agent nothing to act on.

The composer is the round's whole launchpad, not just a text field:

- **Findings from any review round** are insertable (a round picker over
  `agent-review.md`'s `## Review <n>` sections) — round 2's findings stay
  reachable after round 5 lands.
- **Every queued comment is individually held back**: unchecking one *parks*
  it (`annotations.json` status `parked` — kept, findable, reopenable, but no
  longer `open`, so the agent contract "address every open annotation" skips
  it without any skill change). Parking happens before the snapshot, so
  `history/<NNN>/annotations.json` records the round exactly as sent. Each
  row also jumps to the comment in the diff (the draft is kept) or deletes
  it — open comments used to be swept along wholesale and were hard to find
  again.
- **"Record and launch the fix round now"** folds the second click into the
  same screen, carrying the interaction mode (ask in session / interactive /
  autonomous → the kickoff's `Interactive:` field) and an optional **executor
  skill override** — the kickoff then says `Use the <skill> skill.` instead of
  `clash-workflow`, for a custom skill that honors this same file contract.

**Every Request-changes goes through the same snapshotting flow** — the plan
path and the diff path alike freeze the current iteration into
`history/{NNN}/` (`diff.patch`, `plan.md` when the item has one,
`annotations.json`), append the note to `review.md`, and bump `iteration` in
one meta write. This is enforced, not conventional: the bare status-transition
command accepts no note (a note-append that skipped the snapshot was how plan
revisions once vanished), the `review.md` append is idempotent per iteration
number (a crash between the append and the meta write retries with the same
number — the stale section is replaced, never duplicated), and a failed
`git diff` aborts the round instead of freezing an empty `diff.patch` (except
at `plan-review`, where the plan — snapshotted anyway — is the artifact).

The plan copy is what makes a revision visible after the fact: the GUI's
**Timeline** renders one card per change round — the note that caused it, the
plan diff of snapshot N against N+1 (via the pure
`application::diff::unified_diff`), the full frozen plan text, and the code
diff — interleaved with the agent review rounds parsed from
`agent-review.md`. Before this, a plan revision left no trace.

## Agent review rounds

An **agent review** is a bounded side-trip, not a pipeline stage: the item leaves
a decision state, an agent reviews it, and the item comes back to *the same*
state. That round-trip is the whole design — it makes rounds **unbounded**. Run a
deep review, read it, run another, publish the third one to the PR. Nothing about
the item advances until the human clicks Approve.

```
plan-review  ──launch──>  reviewing  ──agent returns──>  plan-review   (repeat)
diff-review  ──launch──>  reviewing  ──agent returns──>  diff-review   (repeat)
pr-draft / pr-ready likewise
```

- Launchable from `plan-review`, `diff-review`, `pr-draft`, `pr-ready`
  (`WorkflowStatus::can_request_review`) — the states holding a reviewable
  artifact while parked on a human decision. Never from a state where an agent
  is already working.
- While `reviewing`, clash **gates approval** and locks the annotation editor
  (the reviewer writes annotations). The GUI always offers "End round", and
  abandon stays available, so a dead reviewer can never wedge an item.
- `meta.json.review` records the round: `target`, `depth`, `publish`,
  `returnStatus`, `round`, `startedAt`. `meta.json.reviewRound` is the
  clash-owned counter (like `iteration` — the agent never writes it).
- `meta.review` is **never cleared** — it describes the round in flight while
  `status == reviewing`, and the *most recent* round afterwards (the reviewer's
  hand-back changes `status` and nothing else; clearing would need a second
  writer). Anything that wants "is a round running?" must gate on the status,
  not on the block's presence.
- Round *outcomes* are read from `agent-review.md`, not from meta: the pure
  `application::workflow::latest_agent_review` parses the last `## Review <n>`
  section (verdict + `### Published` lines) into
  `WorkflowItem.lastAgentReview`, which the GUI shows as a strip on the item
  and in the hand-back toast — "nothing was posted" must be visible without
  opening the report. The Timeline uses the all-rounds sibling
  (`all_agent_reviews`) plus `parse_review_iterations` over `review.md`.
- **Interactivity is the human's call, made either at launch or in-session.**
  Every clash skill (reviewers *and* executor phases) opens with one
  `AskUserQuestion` — interactive or autonomous? — unless the kickoff prompt
  pre-answers it with `Interactive: yes` or `Interactive: no`. The GUI's
  review composer offers the three-way choice (ask in session / interactive /
  autonomous), recorded as `meta.review.interactive` (absent = ask). In
  interactive rounds the reviewer triages drafted findings with the human
  *before* anything is written, asks before making a trivial fix and before
  anything is posted to the PR. Findings the human drops never become
  annotations; they are recorded under `### Dismissed in triage` in the round
  report, which is what stops later rounds from re-raising them.

| field | values | meaning |
|---|---|---|
| `target` | `plan` \| `diff` \| `structure` | plan/diff derived from the launch status, never chosen; `structure` only via the explicit **Explain changes** action |
| `depth` | `standard` \| `deep` | `deep` reads the surrounding implementation and checks the artifact against it |
| `publish` | `local` \| `pr-comments` \| `respond-pr-comments` | what the round does beyond the item |
| `interactive` | absent \| `true` \| `false` | absent = the skill asks in-session; the composer's launch-time answer otherwise |
| `returnStatus` | any status | where the round puts the item back — the repeatability contract |

Publish rules that earned their place:

- The GUI launches `local` / `pr-comments` rounds from one composer (depth +
  findings destination on a single screen) and `respond-pr-comments` from its
  own **Answer PR comments** action (depth `standard`) — answering reviewers is
  a different job from producing a fresh review, and as the third option of a
  second dialog it was invisible. The meta and kickoff-prompt contract is
  identical for all three.
- Every round's report ends with a **mandatory `### Published` section**, even
  `local` rounds — clash parses it, and a missing section reads as "silently
  did nothing".
- `respond-pr-comments` fetches the PR's comments **twice — at the start and
  right before finishing**. A round takes long enough that comments routinely
  arrive mid-round; a single early check reported "zero comments" on a PR that
  had two by the time the round published.
- Publishing is **recoverable after the fact**: `publish_workflow_review`
  (GUI: "Post round N to PR") posts the latest `agent-review.md` round as one
  PR comment via `gh pr comment`, so sharing an already-written round never
  costs a new review.

Findings land on the surface that fits them: **code** findings become
`annotations.json` entries with `"author": "agent"` (so the human triages them in
the diff view and one click turns them into the next change round), **plan**
findings and the round's verdict go into `agent-review.md`. A reviewer may fix
only trivial mechanical issues (typos, unused imports, formatting), asks the
human before making them, and must declare them; anything behavioral is a
finding, not a fix.

**How a round becomes applied work** — a review never applies itself; the
executor does, and the hand-off is *Request changes*: code findings kept in
triage are already open annotations, so the next change round carries them
automatically; plan findings ride the note — the change-request composer's
**Insert round N findings** button pastes the latest round (minus its
record-keeping tails: `### Published`, `### Fixed in this round`,
`### Dismissed in triage`, extracted by the pure
`latestAgentRoundFindings` in `gui/dist/wf-compose.js`) into the note, which
is the next round's prompt. Approving never applies findings — approval is
"ship it as it stands".

### The review agent contract (clash-plan-review / clash-code-review skills)

`Use the <clash-plan-review|clash-code-review> skill. Workflow item directory: <abs path>. Target: <plan|diff>. Depth: <standard|deep>. Publish: <local|pr-comments|respond-pr-comments>. Round: <n>. Return to: <status>. Mode: <mode>.`

Every value is also in `meta.json.review`; repeating it in the prompt lets the
reviewer refuse impossible work before reading anything (a `plan` target with no
plan, a publish mode needing a PR that does not exist) and makes `Return to:`
impossible to miss. The reviewer's last act is always to restore that status.
One optional trailing field: `Interactive: yes|no` — **absent means the skill
asks in-session** before starting (the GUI composer's "ask me when it starts"
default omits the field).

Plan review and code review are **two separate, self-contained skills** — each
owns the whole job for its target: the judgement, the file contract
(`annotations.json` + `agent-review.md`, `### Published` mandatory) and the
status hand-back. They replaced a three-piece design (a `clash-review` harness
delegating diff judgement to the `/code-review` / `/review` built-ins), which
entangled the two review jobs through the shared harness and made neither
describable on its own.

| Target | Engine |
|--------|--------|
| `plan` | `clash-plan-review` (embedded skill) |
| `diff` | `clash-code-review` (embedded skill) |
| `structure` | `clash-explain` (embedded skill — explains, never judges) |

Every engine is a skill clash itself installs, so a review round needs no
third-party plugin present. A unit test asserts any skill named by
`review_engine_for` is in `SKILLS`; otherwise the round would die on an
unresolvable skill *after* a full session spawn. The retired `clash-review` is
removed from `<claude_dir>/skills/` at startup (`RETIRED_SKILLS`) so it can
never shadow its replacements.

`clash-plan-review` is derived from the public `plan-review` skill and carries its
four sections (architecture, code quality, tests, performance), its engineering
preferences, its per-issue options-with-a-recommendation shape — and its
**interactivity**: it pauses to `AskUserQuestion` per issue, recommended option
first, options labeled `3b` so a change request can approve one by name. The
round runs in a clash session pane with the human who launched it watching, so
blocking on a question is safe: the item is parked in `reviewing` and "End
round" is always available. (An earlier version stripped the questions on the
assumption nobody was in the session; it produced rounds that made every call
alone — which is exactly the human's job in this pipeline. The human's per-issue
decisions are recorded in the round report, which is what lets round N+1 skip
what round N already settled.)

Depth does not select the engine: the mapping is the pure
`application::workflow::review_engine_for(target)`. For a code round the
`Depth:` field tunes how hard the diff is read inside `clash-code-review`; a
plan has no hunks to read harder, so the GUI does not ask for depth on a plan
round — a choice with one real answer is not a choice.

### Explain rounds (clash-explain skill)

An **explain round** is review-shaped (parks in `reviewing`, restores
`Return to:`) but judges nothing: it reads the diff and enough surrounding
code, then writes **`structure.md`** — what the change does, organized by
functional part (behavior first, files second), with mermaid diagrams of how
the pieces fit, risks/review-focus observations, and a suggested reading
order. The GUI renders it as the **Structure** tab (mermaid fences are drawn
as real diagrams) and offers the round as the **◫ Explain changes** action
wherever a diff is parked on a decision (`diff-review`, `pr-draft`,
`pr-ready`). `structure.md` is a living document — each round overwrites it —
while the round still appends a `## Review <n> — structure · …` entry to
`agent-review.md` (verdict = a one-line summary, `### Published` = "wrote
structure.md"), so hand-back toasts, the outcome strip and the Timeline stay
truthful. Rounds are unbounded like every side-trip: regenerate after each
change round.

A code review is available at `diff-review`, `pr-draft` **and** `pr-ready`
(`WF_REVIEWABLE` / `can_request_review`), so an item that already has a PR can
still be reviewed without moving it backwards.

## Entry modes

`meta.json.mode` records how the item entered the pipeline. It is fixed at
creation, kebab-case, and **absent means `full`** (items written before modes
existed need no migration). The mode decides the initial status, which phases
exist, and how approval ends the item.

| `mode` | starts at | plan phase | approval at `diff-review` |
|---|---|---|---|
| `full` | `draft` | agent writes `plan.md` | `pr-draft` → `pr-ready` → `done` |
| `from-plan` | `plan-review` | `plan.md` supplied by the human | same as `full` |
| `review-only` | `diff-review` | **none** | straight to `done` |

- **`from-plan`** — the human's plan (a file, a scratch note, pasted text) is
  written into `plan.md` at creation, so no planning agent runs. From the
  agent's side this is identical to `full` from `plan-review` onwards.
- **`review-only`** — the code already exists (a PR or a branch) and clash
  owns neither it nor the PR. `meta.branch` is the branch under review,
  `meta.base` its diff base, `meta.worktree` a checkout clash materialized
  (reusing an existing worktree of that branch when there is one), and
  `meta.pr` the PR when the source was one. The only loop is
  `diff-review ⇄ changes-requested → implementing → diff-review`; there is no
  plan and there never will be one.

`meta.base` (any mode) is the ref the diff is taken against — empty means the
repo's origin default branch. A PR targeting `develop` records `develop`.

## The agent contract (clash-workflow skill)

The kickoff prompt is:
`Use the clash-workflow skill. Workflow item directory: <abs path>. Phase: <plan|revise|implement|pr>. Mode: <full|from-plan|review-only>.`
with two optional trailing fields: `PR skill: <name>.` (from the
`workflows.pr_skill` config — see PR integration) and `Interactive: yes|no.`
(absent means the skill's opening question asks in-session).

The mode is repeated in the prompt (it is also in `meta.json`) so a
`review-only` run knows before reading anything that it must not write a plan.
Like the reviewers, every executor phase opens by settling interactive vs
autonomous — what "interactive" means per phase (approach options before
writing a plan, confirmation before a `wontfix` or a plan deviation, title/body
preview before a PR) is defined in the skill.

On every run, the agent first reads: `meta.json`, `plan.md`, `review.md`
(top-to-bottom — it is the accumulated decision history), the `open`
annotations in `annotations.json`, and the latest `history/<NNN>/diff.patch`
for context.

- **Phase `plan`**: write/overwrite `plan.md`; finish by setting
  `meta.json.status = "plan-review"`.
- **Phase `revise`**: if the review round was about the plan, revise
  `plan.md` and finish with `"plan-review"`. If it was about code, behave
  like `implement`. In `review-only` mode there is no plan, so `revise` is
  always `implement`.
- **Phase `implement`**: set `"implementing"` while working; address every
  `open` annotation — set it `"addressed"` with a short resolution appended
  to its `replies`, or `"wontfix"` with a justification; commit on the item
  branch (never push, never `--no-verify`); optionally create a draft PR
  (`gh pr create --draft`) and write its URL into `meta.json.pr.url`;
  finish with `"diff-review"` (or `"pr-draft"` when a PR was created).

### Hard rules

- In `review-only` mode: never write `plan.md`, never transition to
  `plan-review`, and always finish at `diff-review`. The branch is pre-existing
  and shared, so after committing, **push** it (plain `git push`, never
  force) so the PR reflects the fixes; a rejected push means stop and report,
  never force.
- In any mode, once the item **has a PR** (`meta.pr.url` set), pushing after a
  commit follows the same rule — the branch is published, and a fix round that
  only commits locally leaves the PR silently stale. An unpublished branch
  (`full`/`from-plan`, no PR) is still never pushed.
- Never touch `history/` and never change `iteration` or `reviewRound` — clash
  owns all three (the first two are written atomically by the request-changes
  flow, the third by the review launcher).
- Write `annotations.json` **only** while status is `changes-requested` or
  `implementing` — during review phases the GUI owns the file (this phase
  split is what makes concurrent writes safe). Only `open` annotations are
  work; `parked` ones are human-owned (kept back from the round) and must
  never be touched, like `addressed`/`wontfix` ones.
- Read-modify-write `meta.json`; keep unknown fields.
- Never rewrite `review.md` history — it is append-only (clash appends the
  `## Iteration N` sections; the agent only reads it).

## PR integration

Entirely optional — an item can go `diff-review → done` without one.

Every PR operation goes through the **forge port** (`domain::forge::Forge` —
view, create-draft, mark-ready, comment, unanswered-count, URL parse,
capabilities), so the code host is an implementation, not an assumption.
`GithubForge` (the `gh` CLI, today's only implementation) and the explicit
`NoForge` live in `infrastructure::forge`; which one an item gets is decided
by the `workflows.forge` setting (`auto` | `github` | `none`) — `auto`
detects from the host of `git remote get-url origin`, cached per repo.
Detection is deliberately conservative: unknown hosts count as GitHub
(a GitHub Enterprise remote with a configured `gh` worked before detection
existed and must keep working); only hosts recognizably another forge
(gitlab, bitbucket) map to `none` until they have an implementation.

clash records the PR in `meta.json.pr` (`url`, `number`, `draft`, `state`,
`lastCheckedAt`, `unansweredComments`) via the forge.

**Identity-shaped PR errors are recoverable in place, never dead ends.**
Commands that need a PR identity return machine prefixes — `no-pr:` (nothing
recorded) and `pr-number-unknown:` (a URL clash cannot parse) — and the GUI
answers them by asking for the missing piece: paste the PR URL, it attaches
(`attach_workflow_pr`, which deliberately never demotes a `pr-ready` item)
and the original action retries once. A review round needing a PR
additionally offers "run the round locally instead". The principle
generalizes: an error caused by a data gap must offer the human a way to
supply the datum and continue, so the pipeline is never blocked on a bug
that can be fixed in parallel. `state == "MERGED"`
observed on refresh moves the item to `done`. The agent may create the draft PR
itself — writing `pr.url` is enough: clash fills the rest on the next refresh,
and every command that needs the number derives it from the URL in the
meantime (a URL-only record must never make a button fail with "refresh
first"; opening a PR-bearing item also triggers a throttled refresh).

`unansweredComments` is the count of review-comment threads nobody has replied
to (clash-only, refreshed with the rest of the PR state, absent until first
fetched). It exists so the GUI's "Answer PR comments" action — a
`respond-pr-comments` round launched as its own button — can show whether such a
round has work waiting. It is advisory: the count is up to a poll stale and
capped at one API page, and the reviewer re-fetches the comments itself.

Creating the PR publishes the branch when it is not on the remote yet: `gh pr
create` run non-interactively aborts with *"you must first push the current
branch to a remote"* rather than offering to push, so clash pushes
(`git push --set-upstream <remote> <branch>`, preferring `origin`) and retries
the create once. The retry is gated on that one message — any other `gh`
failure surfaces unchanged — and a detached HEAD or a remote-less repo fails
with a real error instead of a guess.

Every `gh`/`git` subprocess runs with stdin closed, prompting disabled
(`GIT_TERMINAL_PROMPT=0`, `ssh -oBatchMode=yes`, `GH_PROMPT_DISABLED=1`) and a
hard timeout that kills the child. All three are needed: `Command::output()`
inherits stdin, so a credential prompt used to park the call indefinitely — a
GUI launched from Finder has no terminal to answer on. `ssh` reads `/dev/tty`
directly and ignores the closed stdin, which is why the env matters; and a hung
connection ignores both, which is why the timeout exists.

Opening a draft PR has two paths, because the description has two honest price
points and which one is worth it is the human's call:

- **From the plan** (default, free, instant) — `workflow_create_pr` transcribes
  the item's `plan.md` plus its implementation/review round counts. No model runs,
  so the body cannot describe a change that isn't in the plan. An explicit body
  passed to the command always wins; an item with an empty plan gets an empty body
  rather than a bare heading.
- **Written by an agent** (spends tokens) — phase `pr` of `clash-workflow` spawns
  a session that reads the real diff against the base, follows the repo's PR
  conventions (`.github/pull_request_template.md`, recent merged titles, a repo
  skill for opening PRs), opens the draft PR and sets `pr.url`.

When the effective PR skill names a skill, the agent-written path carries it
in the kickoff as `PR skill: <name>` and the executor **must** open the PR
through that skill instead of a raw `gh pr create` — org house style
(templates, ticket references, review requests) rides along instead of being
re-discovered per repo (when the named skill isn't available in the session,
the executor says so and falls back to convention discovery rather than
stopping). Resolution is the pure `effective_pr_skill(item, global)`: the
item's ⚙ Settings-tab override wins (its `none` disables for that item),
else the global `workflows.pr_skill` setting — one `PROPS` row, **default
`hivebrite-engineering:github-pr`**, `none` to disable globally.

Phase `pr` is the one phase that does **not** move the item to a working status on
launch (`phase_keeps_status`): it runs on an item parked at a human decision, and
flipping it to `implementing` would both advertise work that isn't happening and
let it re-enter the implement loop. It is also forbidden from changing code — the
description is the whole deliverable.

## Model per phase

Phases are pinned to a model rather than inheriting whatever the user last
selected, so a round is reproducible — two review rounds on one item are
comparable because the reviewer was the same model both times.

| Phase | Model |
|-------|-------|
| `plan`, `revise` (both rewrite the *plan*) | `claude-fable-5` |
| agent review rounds (`clash-review`) | `claude-fable-5` |
| `pr` (writes prose about a finished diff) | `claude-fable-5` |
| `implement` | `claude-opus-5` |

The mapping is the pure `application::workflow::model_for_phase`, passed to the
session as `--model`. An unrecognized phase falls back to the implementation
model: a phase name that isn't listed is assumed to do work, and
under-powering real work is the worse failure.

## Embedded skills — install, versioning, retirement

The three skills (`clash-workflow`, `clash-plan-review`, `clash-code-review`)
are compiled into both binaries and installed under `<claude_dir>/skills/` at
every startup, compare-then-write. The embedded copy is the source of truth:
local edits are overwritten (copy under a different name for a custom
variant). Two mechanisms make that visible instead of silent:

- A manifest, `<claude_dir>/skills/.clash-skills.json`, records the clash
  version and a per-skill content hash of what was last installed. It gates
  nothing — it is how the installer can tell "this file changed because clash
  upgraded" from "the user edited it since we last wrote it".
- `install_skills` returns a report (updated / retired-and-removed /
  locally-edited-and-overwritten). The GUI toasts a non-noop report at boot
  and shows the installing version in the Skills tab; the TUI logs it.

Skills clash used to ship live in `RETIRED_SKILLS` and are deleted from the
install dir at startup — a retired `clash-review` left behind would keep
matching kickoff prompts its replacements now own.
