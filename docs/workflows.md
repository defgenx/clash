# Workflows — file contract

The Workflows feature manages a plan → review → implement → PR pipeline per
item. clash's GUI renders and mutates the files below; Claude Code agents co-edit
them — the executor (`clash-workflow` skill) does the work, the reviewers
(`clash-plan-review` for plans, `clash-pr-review` for code — including the
code review that ends in a **verdict** posted on the PR) judge it, and
`clash-drift-review` judges the two against each other. This document is the
contract all sides follow.

What the GUI shows and how to use it is in the user guide,
[workflows-guide.md](workflows-guide.md).

## Layout

Rooted at the workflows directory — default `~/.claude/clash/workflows`,
overridable via `workflows_dir` in `config.toml` or the GUI Settings panel.
Deliberately independent of the scratch tree: scratches are free-form notes,
workflows are a structured store.

```
<root>/<project>/<slug>/
├── meta.json          # status, PR info, session, iteration, review round(s), timestamps
├── plan.md            # the plan (freely editable markdown)
├── waves.json         # a project's waves: ids, dependencies, areas (planner writes; absent on one task)
├── review.md          # append-only iteration audit trail (clash writes, agent reads)
├── agent-review.md    # append-only agent review rounds (agent writes, clash renders)
├── explain-plan.md    # what the plan WILL DO — prose + mermaid (clash-explain overwrites)
├── explain-plan.html  # …and as one hand-drawn page: boxes, arrows, repos, features
├── explain-diff.md    # what the change DID — prose + mermaid (clash-explain overwrites)
├── explain-diff.html  # …and as one hand-drawn page
│                      #   (structure.md / blueprint.md: the pre-pair names, still read)
├── explain-*.json     # the overview's parts — the index the drill-down is built from
├── explain-diff/      # one level per zoomed-in part: <id>.md / .html / .json
├── explain-plan/      #   (ids 3, 3.2, 3.2.1) — see Drill-down explanations
├── drift.md           # the plan AGAINST what was built — graded (clash-drift-review)
├── drift.html         # …and as one hand-drawn map, every part badged
├── drift.json         # its areas as a drill-down index; drift/<id>.* are dives
├── annotations.json   # line-level diff comments: {"annotations": [ … ]}
├── handoff.md         # the executor's note to the human (overwritten every phase)
├── explainers.json    # explainer rounds running alongside the item (clash-owned)
├── run.json           # the check run in flight + finished runs (clash-owned)
├── history/<NNN>/     # per-iteration snapshots (diff.patch + plan.md + annotations.json)
└── plan-history/      # every recorded revision of plan.md (index.json + NNNN.md), clash-owned
```

`review.md` and `agent-review.md` are deliberately two files: the first is
clash's record of **human** decisions, the second the reviewer's own findings.
One file would make ownership ambiguous exactly where concurrent writes happen.

`annotations.json` is one object, `{"annotations": [ … ]}`. clash also reads a
bare array of annotations — the shape an agent writes when it drops the
wrapper — and restores the object on its next write (`AnnotationsFile::parse`).
Refusing it wedged the item over a missing pair of braces: zero open comments,
no Diff view, and *Apply* failing with `expected struct AnnotationsFile`. Any
other malformed content is still an error, so a blind save never replaces
review data with nothing.

`handoff.md` is the **executor's** file: `clash-workflow` overwrites it at the
end of every phase, before the hand-back status write, and reviewers and
explainers never write it; clash and the GUI only read it (the item's
**↩ Hand-off** tab, and the next-step recommender reads its `**Next:**`). Shape, under ~25 lines:

```markdown
# Hand-off — <phase> · iteration <meta.iteration>
## Done
- …
## Unsure / needs your decision
- … (or "nothing")
## Checked
- tests/linters run and their result, or what could not be run and why
**Next:** <action> — <one-line reason>
```

Its `**Next:**` line uses the same vocabulary as a review round's (see *Agent
review rounds*).

`(project, slug)` — the directory path — is the identity; `meta.json` never
overrides it. Renaming an item (⚙ Settings, or **Rename…**) rewrites
`meta.title` only: the slug stays as created, because tab keys and recorded
sessions reference it. `meta.base` is likewise editable there. All JSON is lenient: unknown fields must be preserved on
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
- **Forge-driven transitions** (clash's PR refresh, no click): the item
  follows its **primary** PR — observed merged moves it to `done`, observed
  open-and-not-a-draft moves a `pr-draft` item to `pr-ready`. "Ready for
  review" clicked on GitHub, or a PR opened non-draft by a PR skill, is the
  validation step happening elsewhere, and an item that did not follow it was
  stranded: the only forward action at `pr-draft` flips a draft, and nothing
  was a draft any more. Only exactly `pr-draft` advances this way — a
  `reviewing` round hands back where it started and the next refresh moves it.
  Linked PRs never drive either move.
- **Everything else is clash-owned** (buttons in the GUI): approve, request
  changes, mark PR ready, done, abandon, reopen, launching a review round, and
  approving or starting the waves of a multi-wave item (see *Waves*).
  `pr-draft → implementing` and `pr-ready → implementing` exist for the last
  one: the next wave may start on a branch whose PR is already open.

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

The plan copy under `history/` is a round's frozen artifact. The plan's *own*
history is a separate, continuous store — `plan-history/index.json` plus
`plan-history/NNNN.md` — because tying versions to rounds loses every plan
written between them: the planning agent's first draft, an agent's rewrite
mid-round, a human's edit through *Edit plan.md*.
`fs::workflows::record_plan_version` keeps **one version per applied review
round**: it keys on `meta.iteration` (bumped only by request-changes) and
replaces that iteration's version in place while the round is still being
written, so an agent saving `plan.md` twice does not produce two entries. Being
idempotent on content makes it safe to call from anything that might have seen
a change — the FS watcher, the read path, and `workflow_request_changes`, which
closes the current version before the bump. Agents never write this directory; clash owns it like
`history/`. Items created before it adopt their round snapshots on first touch.

The GUI reads it in two tabs: **Plan** is the live document, **◫ Revisions** is
the browser (list + full text + `⇄ Changes` between any two revisions, through
`get_workflow_plan_diff(from, to?)` on revision numbers). The **Timeline**
additionally renders the note that caused each round and the code diff,
interleaved with the agent review rounds parsed from `agent-review.md`, and its
plan links resolve an iteration to a revision on click.

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
- **■ Stop agent** stashes every running agent of the item (recorded agent,
  explainers, any session whose registry `workflow_item` names it) and moves
  nothing. A stopped recorded agent is `agentResumable` — still a listed
  session — so the bar offers **▶ Resume** (act `open-session`, which resumes
  the same conversation) ahead of Relaunch / End round. A stopped round stays
  `running` in a check run until the item parks, so nothing relaunches it.
- `meta.json.review` records the round: `target`, `depth`, `publish`,
  `returnStatus`, `round`, `startedAt`. `meta.json.reviewRound` is the
  clash-owned *total* (like `iteration` — the agent never writes it).
- **Round numbers restart per target.** `## Review 1 — plan` and
  `## Review 1 — diff` are two different rounds: how many times a plan was
  reviewed says nothing about how many times the code has been, and one shared
  counter made the first code review of a well-planned item read as "round 7".
  The number is counted from this file (`next_review_round`), so it cannot
  drift from it, and `(target, round)` — not the number — is a round's
  identity. Every heading must therefore carry its target as the first word of
  the tail, which the skills' templates already do.
- `meta.review` is **never cleared** — it describes the round in flight while
  `status == reviewing`, and the *most recent* round afterwards (the reviewer's
  hand-back changes `status` and nothing else; clearing would need a second
  writer). Anything that wants "is a round running?" must gate on the status,
  not on the block's presence.
- **A round is applied by a change round, and clash records which one.** A
  reviewer never edits `plan.md` or the code, so its findings only become work
  when the item takes its next change round — where the executor reads the
  latest `agent-review.md` round as input. `↻ Apply review rN` does exactly
  that in one click: it composes the note from the round's own findings (the
  pure `applyReviewNote`, fed by `roundFindings`), calls the same
  `workflow_request_changes` the composer uses, and launches the executor.
  There is deliberately no second mechanism — a path that revised the plan
  without recording a round would leave the revision untraceable and
  unversioned, which is the bug the snapshotting flow above exists to prevent.
  `meta.json.appliedReviewKey` (clash-owned, like `reviewRound`) is stamped
  with the applied round's `"<target>:<round>"` on every request-changes, so a
  latest round whose key does not match means "a review landed and nothing has
  been done with it" — the state the item header shows as *not applied yet*,
  and the reason the stage's own approve button is demoted while it holds. A
  key rather than a number because numbers restart per target: three applied
  plan rounds would otherwise read as having covered code round 1.
- **The explainer explains two artifacts, in two forms, and judges nothing.**
  `Target: explain-plan` reads `plan.md` plus the code it will land in and
  writes `explain-plan.{md,html}` (what the implementation is going to do,
  before any of it exists); `Target: explain-diff` reads the diff and writes
  `explain-diff.{md,html}` (what the change did). Both run the `clash-explain`
  skill and write **only their own pair** — a round on one must never touch the
  other's, since a human reads the first to authorize the work and the second
  to review it. The two forms are not alternatives: the `.md` is the written
  walk-through (prose + mermaid, file and symbol names, open questions), the
  `.html` is one self-contained page of boxes and arrows — the parts, the repos
  and features they live in, what is new versus existing, the blast radius.
  clash renders the page in a sandboxed iframe with **scripts disabled**, so it
  must carry no `<script>`, no external resources and no mermaid (which needs
  JS and belongs in the `.md`); inline `<style>` and inline `<svg>` are the
  tools. There is **no verdict**: an earlier design had the plan explanation
  accepted or rejected, with a `meta.blueprint` block and a demoted approve,
  and it was removed — the explainer's value is being judgement-free, and the
  stage's own approve was already the decision.
  The two targets were originally called `blueprint` and `structure`. Both
  spellings are still **read** (a serde alias on the enum, and
  `ReviewTarget::canonical` for the round headings already on disk, so an
  existing item's rounds keep counting toward the same per-target tally), and
  neither is written any more: the feature says "explain" in its file names,
  its tabs and its buttons, and one feature with two vocabularies is how a
  word the UI never uses surfaces in a round heading. `ReviewTarget::explains()`
  is the predicate every "explainers are different here" site asks — naming one
  variant is what left the plan explanation counting as a pending review,
  demoting the stage's own approve over findings that do not exist.
- **Every round declares whether it should be applied**, as
  `**Apply:** yes|no — <reason>` next to its `**Verdict:**`. The reviewers'
  skills own the judgement (interactive rounds ask the human; autonomous ones
  weigh materiality, and both refuse when the findings need a decision an
  executor cannot make); `parse_round` reduces the line to a tri-state on
  `AgentReviewSummary.apply` — anything not recognizably yes/no stays `None`,
  because clash launches an agent off a `yes` and silence is not consent. The
  explainer writes no such line: an explanation is not findings.
- **Every reviewer round also recommends a next step**, as
  `**Next:** <action> — <reason>` on the line after `**Apply:**` (and the
  executor ends `handoff.md` with the same line). `<action>` is exactly one
  of: `apply` (worth a change round now), `approve` (nothing material left),
  `plan-review`, `code-review`, `deep-review` (a risky area: auth,
  migrations, concurrency, data loss, money, wide blast radius), `drift`
  (compare plan and diff), `answer-comments` (PR comments waiting on an
  answer). It must agree with `**Apply:**` — `yes` ⇒ `apply`, unless the
  reviewer judges `answer-comments` more urgent. It feeds the GUI's suggested
  next step as a recommendation only: clash's own ordered checks (a pending
  unapplied round, open comments) rank first, `Next:` is used where they are
  silent, and an unknown or missing action is ignored. Interactive rounds ask
  the human to confirm it with the apply call. The explainer writes none.
- **Auto-apply needs two signatures.** The check run's `autoApply` is the
  human's pre-authorization from the composer, carried into each round's
  kickoff as `Auto-apply: yes|no` so the skill knows whether its own `yes`
  fires (an interactive round is about to tell the human what happens next,
  and "I'll apply it" is a lie when clash is only going to recommend it). The
  run applies only the rounds that said `apply == true`, and only once every
  judging pass has settled — see *Check runs*. Neither signal alone spawns
  anything.
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
| `target` | `plan` \| `diff` \| `explain-diff` \| `explain-plan` (reads `structure`/`blueprint` too) \| `drift` | plan/diff derived from the launch status, never chosen; the rest only via their own explicit action (**◫ Explain changes** / **◫ Explain plan** / **⇄ Compare plan vs changes**). `ReviewTarget::requestable()` is the rule |
| `depth` | `standard` \| `deep` | `deep` reads the surrounding implementation and checks the artifact against it |
| `publish` | `local` \| `pr-comments` \| `respond-pr-comments` | what the round does beyond the item |
| `interactive` | absent \| `true` \| `false` | absent = the skill asks in-session; the composer's launch-time answer otherwise |
| `prUrls` | absent \| a list of PR URLs | the PRs the round is about, when the launcher picked any; absent = the item's own change (local diff, primary PR to talk to). Several is one round over several diffs — see *Per-PR action scope* |
| `prUrl` | absent \| a PR URL | the single-PR spelling `prUrls` replaced. Read-only, folded in by `review_pr_urls`; nothing writes it |
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
- **A thread is settled only when the authenticated user's reply is its last
  word; every other thread is open.** That covers the three cases a respond
  round has: a reviewer's comment nobody answered; a finding clash's own
  `pr-comments` round published with no decision posted under it (a published
  finding is a question, not an answer — counting only other people's comments
  hid seven findings with zero replies behind a button reporting nothing to
  do); and a thread the user answered that somebody answered back. A root of
  the user's own is therefore settled only once they reply *under* it.
  `gh::count_unanswered_review_comments(json, viewer)` is the implementation
  (viewer from `gh api user`, memoized per process; an unknown viewer falls
  back to "threads with no reply at all"), and the reviewer skill states the
  same rule — clash and the skill must agree on it or the button lies.
- **A reply goes in the thread**, through
  `POST /repos/{owner}/{repo}/pulls/<n>/comments/<root_id>/replies`.
  `gh pr comment` posts a *detached* issue comment: the thread still shows no
  answer and still counts as waiting, which is how a respond round that did
  its reading can look like a round that did nothing.
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

### Finding confidence

Every finding of every reviewer (`clash-plan-review`, `clash-pr-review`,
`clash-drift-review`) carries a **confidence** — how sure
the reviewer is that it is real — so the human knows which findings to
believe and which to dig into. It is graded by evidence, and it is
orthogonal to severity: a `low`-confidence blocker is the one to look at
first, not the one to skip.

| Level | Evidence |
|---|---|
| `high` | verified against the code: the exact path read, traced or run |
| `medium` | strong evidence, one link not verified |
| `low` | plausible, unverified — worth a look before acting on it |

Each comes with a one-line **basis** naming the evidence; below `high`, the
basis also says what would raise it. Low findings are reported, never
dropped. One shape per surface:

| Surface | Shape |
|---|---|
| `agent-review.md` | the finding's last line: `**Confidence:** <level> — <basis>` |
| `annotations.json` | `"confidence": "high"\|"medium"\|"low"` and `"confidenceBasis": "<basis>"` on the entry; both optional, absent on human comments |
| PR line comment | a small last-line footer, `<sub>🤖 <grade> · confidence <level> — <basis></sub>`, then the hidden `<!-- clash-pr-review F<n> -->` marker; the summary's items end `— <grade>, <level> confidence` |
| `review.md` digest | ` [confidence: <level> — <basis>]` after the body (`annotation_digest_line`) |

A finding that reaches clash **without** a confidence is shown as one, never
silently: an agent annotation with none badges `confidence ?` and its digest
line says `[confidence: unknown — the reviewer gave none]` (human comments
carry none by design). On the PR side, `clash-pr-review` runs a `jq` check
over every review it posts (comments or a verdict) before sending it and refuse to post a line comment missing its footer.

clash reads it leniently: `FindingConfidence` is case-insensitive, an
unrecognized value loads as `unknown` and a non-string one costs the field,
never the file — so old annotations and rounds load unchanged. The GUI badges
it (diff threads, the composer's queued comments, the Agent reviews tab, the
insert-findings picker's per-round tally); the apply note
(`applyReviewNote`) adds the round's tally and tells the executor to verify
`low`/`medium` findings before acting, and the executor's skill says the same
for annotations. Confidence weighs on `**Apply:**`: a round whose only
material findings are `low` answers `no` unless its reason says why applying
anyway is right, and `clash-pr-review` never requests changes on a `low`
finding alone. A drift round's `INTENDED` grade needs `high` confidence in
the authorization; anything less is an `ISSUE` for the human to decide.

### The review agent contract (clash-plan-review / clash-pr-review skills)

`Use the <clash-plan-review|clash-pr-review> skill. Workflow item directory: <abs path>. Target: <plan|diff>. Depth: <standard|deep>. Publish: <local|pr-comments|respond-pr-comments>. Round: <n>. Return to: <status>. Mode: <mode>.`

Every value is also in `meta.json.review`; repeating it in the prompt lets the
reviewer refuse impossible work before reading anything (a `plan` target with no
plan, a publish mode needing a PR that does not exist) and makes `Return to:`
impossible to miss. The reviewer's last act is always to restore that status.
Two optional trailing fields: `PR: <url>` — the PR the round talks to, when
the launcher picked one of several (multi-repo items answer reviewers per PR;
absent means the primary `meta.pr.url`, and a linked PR's calls must be
repo-scoped since it lives in another repository) — and `Interactive: yes|no`
— **absent means the skill asks in-session** before starting (the GUI
composer's "ask me when it starts" default omits the field). A code review
(verdict rounds included) also carries `Size: <small|medium|large> (<N> lines, <F> files).`
right after `Depth:` when the change was measured — see *Sized review rounds*.

Plan review and code review are **two separate, self-contained skills** — each
owns the whole job for its target: the judgement, the file contract
(`annotations.json` + `agent-review.md`, `### Published` mandatory) and the
status hand-back. They replaced a three-piece design (a `clash-review` harness
delegating diff judgement to the `/code-review` / `/review` built-ins), which
entangled the two review jobs through the shared harness and made neither
describable on its own.

**Code has one reviewer, `clash-pr-review`, whatever the round's outcome.**
There used to be two: `clash-code-review` for findings (local, posted as
comments, or answering threads) and `clash-pr-review` for a verdict, which
read the first skill and overrode parts of it. That split coupled things that
are independent — the convergence loop, the reading of the PR's discussion
and of the surroundings were only available with a verdict posted on GitHub,
so a thorough review before opening a PR, or one that respected settled
threads without posting anything, did not exist — while the two files
cross-referenced each other for the grades, the stack rules and the posting
voice. One skill now owns the review, and the **outcome** decides what differs:

| Outcome | Passes | May fix | Leaves the machine |
|---|---|---|---|
| `local` | `standard`: one pass + verification · `deep`: the loop | trivial mechanical fixes | nothing |
| `pr-comments` | as `local` | trivial mechanical fixes | one `COMMENT` review of line comments |
| `respond-pr-comments` | the threads are the work | bounded fixes the threads ask for | a reply in every open thread |
| verdict (`self-review`) | always the loop | nothing | `APPROVE` / `REQUEST_CHANGES` / `COMMENT` per PR |

Whenever the round has a PR, every outcome reads its discussion and verifies
each thread's outcome against the code. `clash-code-review` is in
`RETIRED_SKILLS`, so an unedited installed copy is removed at startup. The
wire targets did not change: a verdict round is still `Target: self-review`,
recorded under its own heading and numbered among the verdict rounds, so
existing items keep their history.

| Target | Engine |
|--------|--------|
| `plan` | `clash-plan-review` (embedded skill) |
| `diff` | `clash-pr-review` (embedded skill) |
| `explain-diff` / `explain-plan` | `clash-explain` (embedded skill — explains, never judges) |
| `drift` | `clash-drift-review` (embedded skill — compares the plan with what was built, and grades the gap) |
| `self-review` | `clash-pr-review` (embedded skill — the code review ending in a verdict: loops until a pass finds nothing new, then approves or requests changes on the PR) |

Every engine is a skill clash itself installs, so a review round needs no
third-party plugin present. A unit test asserts any skill named by
`review_engine_for` is in `SKILLS`; otherwise the round would die on an
unresolvable skill *after* a full session spawn. The retired `clash-review` and
`clash-code-review` are removed from `<claude_dir>/skills/` at startup
(`RETIRED_SKILLS`) so they can never shadow their replacements.

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
`Depth:` field tunes how hard the diff is read inside `clash-pr-review` (and
`deep` makes the round loop); a
plan has no hunks to read harder, so the GUI does not ask for depth on a plan
round — a choice with one real answer is not a choice.

### Explain rounds (clash-explain skill)

An **explain round** judges nothing, and it is the one round that **parks
nothing**: it runs alongside the item's other agents (see *Parallel rounds*
below). Two of them exist, one per artifact:

- **◫ Explain plan** (`Target: explain-plan`) reads `plan.md` and the code it will
  land in, and says what the implementation is going to do — offered wherever
  the item has a plan that is not being written right now.
- **◫ Explain changes** (`Target: explain-diff`) reads the diff and enough
  surrounding code, and says what the change did — offered from any stage past
  `plan-review` whose code is not being written right now, including finished
  items.

Each writes its pair: the `.md` organized by functional part (behavior first,
files second) with mermaid diagrams, risks and a reading order; the `.html` as
one graphical overview. The GUI renders them as the **◫ Plan explained** and
**◫ Changes explained** tabs, opening on the page and toggling to the text.
Both files are living documents — each round overwrites its own pair — while
the round still appends a `## Review <n> — <target> · …` entry to
`agent-review.md` (verdict = a one-line summary, `### Published` = which files
it wrote), so the Timeline stays truthful and clash can tell the round
finished. Rounds are unbounded like every side-trip: regenerate after each
change round. `Focus:` (optional, asked at launch) names what the round should
concentrate on.

### Drill-down explanations

An explanation is a **tree**: the overview (`explain-<target>.{md,html,json}`)
and one level per part someone zoomed into (`explain-<target>/<id>.{md,html,json}`,
ids `3` → `3.2` → `3.2.1`, three levels at most). Each level has its own
altitude — the C4 model's rule, so "deeper" is never the parent redrawn bigger:

| Level | Id | Shows |
|---|---|---|
| overview | — | the parts of the change (5–12) |
| part | `3` | its components and the flow between them, and its boundary contract |
| component | `3.2` | the mechanism: functions, states, invariants, `file:line` |
| code | `3.2.1` | an annotated walk-through of one function or hunk — a leaf |

Every level writes a `.json` index (`title`, `basis {iteration, head}`,
`nodes[]` with `id/title/summary/critical/why/files/isNew/leaf`, `edges[]`),
`.md` and `.html` first and the index **last**. Everything else is derived, by
the pure `application::explain_tree::build_tree`:

- **expanded**: a part's own index exists;
- **stale**: a level was drawn before its parent was redrawn, from another
  basis, or for a part its parent no longer lists (and everything under a
  stale level is stale). Nothing is deleted — a stale level is labelled and
  can be redrawn;
- **diveable**: a well-formed child id within the cap, not `leaf`. Ids are
  validated (`valid_node_id`) before they become a path.

**Cost is the human's choice.** A top-level explain round carries
`Detail: overview | critical | full` (setting `workflows.explain_detail`,
default `critical`, overridable per launch):
- `overview` writes the overview only;
- `critical` adds the first level of the parts the agent flags `critical`
  (at most 3 — "where attention pays", with a one-line `why`, never a grade);
- `full` adds the first level of every non-leaf part.

Deeper levels are always **dives**: `start_workflow_dive` launches one part
with a kickoff carrying `Node: <id>` (`build_dive_prompt`). A dive:
- writes only that part's three files and appends **nothing** to
  `agent-review.md`, so round tallies, the Timeline and "latest round" are
  untouched;
- is recorded in `explainers.json` with its `node`, and is finished once that
  node's index is written after its launch (`dive_finished`);
- claims `<item>#<target>#<node>`, so dives into different parts run side by
  side. It is refused while the overview it hangs under is still being drawn.

**The warm path.** When the explainer session that drew the overview is still
alive and finished, the dive is queued into it through the prompt queue — it
already holds the code, which is most of a dive's cost — otherwise a fresh
session starts. For the drift report, the warm session is the drift reviewer's
once it has handed the item back. A drift dive always runs `clash-explain`: it
explains a divergence and never re-grades it, so `drift.md`'s grades and
annotations stay the drift round's.

**Views.** A level may carry up to two extra drawings of itself, each one
answering a question the map answers badly: `flow` (one request step by step),
`state`, `data`, `before-after`, `rollout`, `dependency`. A view is one page,
`<level>.<view>.html` (`explain-diff.flow.html`, `explain-diff/3.state.html`),
listed in the level's index as `views[] {id, kind, title, why}`. A view id is
lowercase and starts with a letter (`valid_view_id`), so `3.flow.html` can
never read as a part. The level shows the listed views whose page exists, then
unlisted pages, capped at `MAX_VIEWS` (`build_tree`). The GUI turns them into
tabs between *◫ Diagram* and *☰ Write-up*, with `why` as each tab's tooltip.

Which views to draw is the explainer's call in an autonomous round. In an
interactive round it suggests them and the human picks. `Focus:` naming one
always gets it. The catalogue, and the legibility rules every drawing follows,
live in `skills/clash-explain/SKILL.md` → *The HTML page* / *Views*.

The GUI builds the breadcrumb, the parts strip, the view tabs and the dive
buttons from the index, **outside** the sandboxed frame. The frame document
starts with `<base href="about:srcdoc">`. Without it, a srcdoc frame resolves
links against the app's own URL, so `#node-3` loaded clash itself inside the
frame. `wfGuardExplainFrame` then handles every click in the drawing:
- a part link opens or dives that part;
- a web link goes through `openLink`;
- anything else does nothing.

A frame that navigates away anyway is put back on its drawing.

A drawing gets the whole pane, whatever width the agent wrote it for. The frame
document ends with `FRAME_BREATHE_STYLE` (`gui/dist/wf-share.js`), which lifts
any `max-width` on the page's `body` and scales every drawing marked by
`markWideDrawings` to the pane's width, as tall as the pane allows. Those are
outermost `<svg>`s whose viewBox is at least 400 units wide. Top-level prose
keeps a readable line length. The style comes *last* so it beats the page's
own rules. The saved page's explorer applies the same rule.

A part with no level of its own still has a page (`renderWfExplainPartPage`,
model `explainPartPreview` in `gui/dist/wf-explain-tree.js`). It shows what
the parent level already says about the part: summary, `why`, its in/out edges
with the neighbours, its `files` and its siblings. Its *Explain this part*
button starts the dive with the reader's question as `Focus:`. A click on a box
never starts an agent by itself. Fragment
watching (`wfWatchExplainFrame`) remains the fallback, so the page still runs
no script.

**◫ Explain a PR…** turns any PR URL into a review-only item
(`create_workflow_review`) and launches `explain-diff` on it with the chosen
detail. The local checkout is found by origin (`repo_origin_slugs` +
`matchRepoForPr`: an exact, unambiguous `owner/repo` match, else the repo
picker).

### Parallel rounds

A judging or writing agent — the executor, a plan/code reviewer, a drift round
— owns the item: it moves the status and may write `plan.md`, the code or
`annotations.json`, so there is at most one of them, and approval is gated
while it runs. An explainer writes only its own document pair and one appended
round entry, so it runs **beside** any of them instead of waiting its turn:

| | alongside it |
|---|---|
| **◫ Explain plan** | a plan review (`reviewing` from `plan-review`), the implementation (`implementing`, phase `implement`), any reviewer or drift round, the other explainer |
| **◫ Explain changes** | any reviewer or drift round of a built change (`reviewing` from `diff-review`/`pr-*`), the other explainer |

What still stops one is its own artifact being written, or not existing yet
— `WorkflowStatus::can_explain(target, meta.phase, returnStatus)`:

- the plan explanation never runs while the plan is being written (`planning`,
  or `implementing` on phase `revise`), nor at `draft`;
- the changes explanation never runs while code is being written
  (`implementing`), before any exists (`draft`, `planning`, `plan-review`, a
  review round launched from `plan-review`, `changes-requested` queued as
  `revise`);
- one explainer per target at a time — two would overwrite the same pair.
  Dives are per part (`<item>#<target>#<node>`), so several run at once; a
  dive waits for the overview it hangs under to finish.

The gate is checked at launch only. An executor may still be launched while an
explainer runs (that is the point); an explanation of a diff that then moves
describes the diff it read, and the next round regenerates it.

Three mechanics make it safe:

- **The explainer never writes `meta.json`**, not even the status: its kickoff
  carries no `Return to:`, and clash does not write `meta.review`,
  `meta.sessionId` or `meta.reviewRound` for it — those stay the owning
  agent's. It is recorded instead in **`explainers.json`** (clash-only; one
  record per target — `target`, `round`, `sessionId`, `startedAt` — replaced
  by the next launch of that target), because an agent's read-modify-write of
  `meta.json` would drop a record kept there. An explainer launch claims
  `<item>#<target>`, not the item, so it neither waits on nor blocks the
  item's own launches.
- **Finished is derived, never written.** A record is finished once
  `agent-review.md` holds its `## Review <round> — <target>` entry, and
  running while it is unfinished and its session is alive (with the usual
  launch grace). A session that died unfinished simply stops counting as
  running, so the button is offered again; **✕** forgets a record whose
  session is alive but will never finish. The GUI toasts "explanation ready"
  on the running → finished edge, since no status change announces it.
- **"The latest round" means the latest judgement.** An explainer lands
  whenever it finishes, so it routinely follows the review it ran beside.
  `latest_agent_review` / `latest_agent_review_section` (and the GUI's
  `latestAgentRoundFindings`) skip explanation rounds, so apply, the
  `appliedReviewKey` stamp and *Post round N* still act on the review, and
  `pendingReviewRound` reads the round's own heading target rather than
  `meta.review`. Every skill appends its round with a single shell append
  (`cat >> agent-review.md`), never a whole-file rewrite from an older read.

A code review is available at `diff-review`, `pr-draft` **and** `pr-ready`
(`WF_REVIEWABLE` / `can_request_review`), so an item that already has a PR can
still be reviewed without moving it backwards.

### Drift rounds (clash-drift-review skill)

A **drift round** (`⇄ Compare plan vs changes`, `Target: drift`) answers the one
question nothing else in the pipeline asks: *is this the change we agreed to
build?* The plan reviewer judges the plan, the code reviewer judges the diff,
and a diff can pass a code review on its own merits while delivering something
else — half a feature, an extra subsystem nobody signed off on, a different
mechanism than the one that was authorized. Only this round reads `plan.md` and
the diff together.

It is **a reviewer, not a third explainer**, and every difference follows from
that:

- **Gated like a review** (`can_request_review`), not like an explanation, and
  it additionally needs *both* sides of the comparison — a plan
  (`ReviewTarget::needs_plan`, so never on a `review-only` item) and an
  implemented change (`needs_diff`, so never at `draft` or `plan-review`, where
  nothing has been built). In practice: `diff-review`, `pr-draft`, `pr-ready`.
- **Its findings become work through the single existing mechanism.** Every
  divergence graded an issue whose remedy is in the code is written to
  `annotations.json` with `"author": "agent"`, so it enters the same triage
  loop as a code review's findings and one *Request changes* turns the set into
  an executor round. The round is therefore an ordinary pending review: it
  carries `**Apply:** yes|no`, `↻ Apply review rN` offers it, and
  `meta.appliedReviewKey` records `drift:<n>`. It is deliberately **not**
  `ReviewTarget::explains()` — that predicate drives the "no findings to apply"
  exemptions, and a drift round marked as explaining would grade divergences
  nobody could ever act on.
- **It writes an explanation-shaped document pair anyway** (`drift.md` +
  `drift.html`, the GUI's **⇄ Plan vs changes** tab), because a divergence is
  only arguable once both shapes are visible side by side. Same two forms and
  the same sandbox rules as the explanations: the `.md` is the written
  comparison with a mermaid graph of the plan's spine, every node badged with
  what happened to it; the `.html` is **one** hand-drawn map with the same
  badges — never two maps, since making the reader diff two pictures by eye is
  the work the round was launched to do for them.

Each divergence carries a **direction** (`MISSING` — promised, not delivered;
`EXTRA` — delivered, never planned; `DIFFERENT` — done by other means) and a
**grade**, which is the answer the human wanted:

| Grade | Means | Remedy |
|---|---|---|
| `INTENDED` | Deliberate and justified — authorized in a `review.md` iteration, or forced by something found during implementation. The code is right; the plan is now stale. | amend the plan |
| `BENIGN` | Real but consequence-free. | none |
| `ISSUE` | The item does not deliver what was agreed, or delivers what nobody agreed to, with a consequence someone will meet. | fix the code, or amend the plan |

Grading is on **consequence, never size**, and a plan-promised test, migration
or doc that is absent is an ISSUE rather than a nit — it is the part of a plan
most reliably dropped under pressure and the part a diff review cannot see,
because you cannot review what is not there. `review.md` is read as the
authorization log: a deviation the human asked for in a change request is not
drift from the agreement, it is the agreement being updated.

**The one remedy clash cannot apply for you is a plan amendment.** An executor
fix round never writes `plan.md`, so a round whose issues all need the plan
changed must answer `**Apply:** no` (with `**Next:** plan-review`) — otherwise clash would spawn an agent that
finds nothing it is allowed to do. Those entries go under
`### Plan amendments needed` in the report and in a `## Plan amendments needed`
section of `drift.md`, and the route is **↩ Move back to… → plan-review**, then
*Request changes*. Three places enforce the split so it cannot be applied by
accident: the composer's auto-apply detail says it, `roundFindingsAt` omits
that section (along with `### Intended deviations` and `### Benign` — pasting
divergences the round approved would ask an executor to undo them), and
`pendingReviewRound` returns nothing for a `drift` round at `plan-review`, so
"apply this to the code" is never offered on the stage where the plan is the
artifact.

Ground truth is `plan.md` and the diff — **not** `explain-plan.md` and
`explain-diff.md`. Those are read as context, and `explain-plan.md`'s numbered
action graph is adopted as the report's spine when it is current — its
`explainers.json` record's `iteration` equals `meta.iteration` and its round
entry is already in `agent-review.md` — (same numbers, so the two documents
can be read side by side); a stale one, or one being rewritten alongside the
drift round, is skipped and the spine numbered from `plan.md`, never waited
for. Comparing two explanations
would measure drift between two *documents*: an `explain-diff.md` written three
rounds ago describes code that no longer exists.

### Verdict rounds (clash-pr-review skill)

A **verdict round** (`Target: self-review`, called a *self-review* before the
code reviewers were merged) is a code review that ends by **posting a verdict
on the PR** — `APPROVE` or `REQUEST_CHANGES`, one GitHub review carrying line
comments on the code concerned and a summary. It is not a separate check: the
code review's composer and its checklist row offer it as the round's last
outcome, **Post a verdict on the PR**, whenever an open PR exists at a built
stage, and each PR chip's menu has *Post a verdict on <pr>…*, which opens that
composer with the verdict picked and the PR ticked. It is the only round
allowed to approve or request changes: every other outcome is forbidden to,
because its findings go to the human first. Here the post *is* the output, so:

- **It always publishes.** `ReviewTarget::publishes()` makes the launcher force
  `Publish: pr-comments` whatever the caller sent, and refuse an item with no
  PR before spawning (`no-pr:` — whose recovery offers only *Attach PR*, since
  a local verdict would be bounced straight back to the missing PR).
- **Gated like a code review, plus a PR.** `can_request_review`, `needs_diff`
  (never at `draft`/`plan-review`), and at least one PR on the item — but no
  plan (`needs_plan` is false): `review-only` items are its most common
  customer. In practice `diff-review`, `pr-draft`, `pr-ready`.
- **Its scope is a set of the item's open PRs.** On a multi-PR item the
  composer's scope group is the answer (its "this repository's own diff" row
  reads *Every open PR of this item* while the verdict is picked); the
  checklist row states the set and changes it through `pickPrScope(item,
  "selfReview")`. Candidates are the live PRs (a verdict on a merged one
  approves nothing) and the default is **all of them**, because the round's
  subject is the whole change (`wfVerdictUrls`). The pick rides
  `meta.review.pr_urls` and the kickoff's `PR:` line, and the skill posts one
  verdict per PR.
- **It loops.** One reading of a PR finds what that reading looked for, so the
  skill runs a Ralph loop over a ledger **outside the item directory**
  (`$TMPDIR/clash-pr-review/<owner>-<repo>-<n>-<sha7>/ledger.md` — the item
  tree is watched and its files are a contract). Pass 1 is breadth; every
  later pass re-reads the ledger and takes the thinnest angle (hunks read
  least, beyond the diff, clusters around verified findings, adversarial
  inputs, cross-file/cross-repo contracts, tests). Every new candidate is
  refuted against the code before it counts. The loop stops when a pass
  verifies nothing new, every hunk has been read (twice from different angles
  at `deep`), and the minimum ran (`standard` 2 / `deep` 3); the cap is 5 / 8,
  and hitting it while still finding `BLOCKER`/`RISK`s means *request
  changes*.
- **The PR's discussion is an input, not background.** Every review thread
  (with its resolved/outdated state, via GraphQL), every review, the
  conversation (bots included) and the commits are read, and each item gets a
  *verified* outcome against the code at the PR head: fixed, **not fixed**
  (resolved or "fixed in <sha>", but the problem is still there), still open,
  decided (a settled trade-off whose reasoning holds — respected, not
  re-raised), decided-but-wrong, or no longer applies. Outdated is not
  resolved, and resolved is a claim. Items still needing work become findings
  credited to the reviewer who raised them; the loop does not stop while an
  item is unverified, and the discussion is fetched again before posting.
  `### Prior discussion` is the report's record of it and is left out of
  pasted findings.
- **The post reads like a colleague's review, shaped to the case.** No fixed
  template: a clean approval is one line, one blocker is a sentence with its
  link, several are a short numbered list, a question for the author is asked
  as one, and a re-review says what it re-checked. Line comments are one point
  each, anchored where the fix goes, phrased as a question when confidence is
  low, with head-pinned permalinks for the code they mean and Conventional
  Comments prefixes (`blocking:`, `nit:`, `question:`) only where they change
  the reader's next move. Every ticket, PR, commit or doc it names is a link
  (never a guessed URL), and it never writes "LGTM" — the pre-post check
  refuses one. No description of the change, no praise, no pass
  count: those live in the ledger and the item's report. The skill's *Post*
  section is the style guide for everything it posts — comments, replies and
  verdicts alike.
- **The other side is read before it is blamed.** The skill lists the
  services, repositories and libraries the changed code reaches across a
  boundary and reads them at the version the change runs against (the pinned
  ref, not the default branch; docs at the pinned version for third-party
  ones). A finding that rests on another side's behaviour is capped at
  `medium` confidence until that side is read. Interactive runs ask which to
  read up front (one multi-select, after the opening question); autonomous runs
  read the ones a candidate finding or a risky path depends on.
- **The verdict is owned.** Before approving, the skill names the worst
  thing the change can do once merged and must have read the code that
  prevents it at `high` confidence (auth, data and migrations, payments,
  deletion, concurrency, cross-service contracts), or it posts `COMMENT`
  saying what it did not cover; every open `RISK` needs a stated reason to
  accept it. A change request blocks only on verified defects this PR
  introduces, each with what would make it approvable. See *Own the verdict*
  in the skill.
- **Three verdicts.** `APPROVE`, `REQUEST_CHANGES`, or `COMMENT` when neither
  is honest (part of the change could not be read, or what is open is a
  question only the author can answer).
- **The decision rule**: request changes on an open `BLOCKER`, a `RISK` a real
  input triggers, a claimed fix that is not in the code, a reviewer's blocker
  still open, CI failing because of the change, or a change that does not
  do what its description (or `plan.md`) says; approve otherwise, with the
  rest posted as non-blocking comments.
- **Grade and confidence are never dropped.** Every line comment ends with a
  small `🤖 <grade> · confidence <level> — <basis>` footer and a hidden
  `<!-- clash-pr-review F<n> -->` marker (later runs use the markers to avoid
  re-posting a finding; the old leading `🤖 **GRADE**` is still recognised),
  and a `jq` check over the review refuses to post one without them. The
  summary starts with the `<!-- clash-pr-review -->` marker and ends with a
  one-line automated-review disclosure.
- **The size of the change sets the effort.** Small / medium / large (lines and
  files changed) picks the models at launch and scales the loop's fan-out and
  passes — see *Lead and subagents*.
- **GitHub's two refusals are stated, not hidden.** A PR's author cannot
  approve or request changes on it — the usual case for an item's own PR — so
  the review goes out as `COMMENT` with the verdict stated first; a draft takes
  no review at all, so line comments and a summary comment are posted instead.
  The composer says both before launch; the round's `### Published` names the
  event actually sent.
- **It fixes nothing**: a verdict is about one commit. The findings on the
  primary PR also land as `annotations.json` entries (`"author": "agent"`), so
  a change-request verdict becomes a fix round through *Request changes* /
  `↻ Apply review` like any code review (`meta.appliedReviewKey` =
  `self-review:<n>`), and `### Passes` (the loop record) is omitted from the
  pasted findings.
- **Next-step assist** suggests it at `pr-ready` once per iteration — only
  after that iteration's code review, since the verdict is how that review
  ends on the PR rather than a second reviewer racing it — and a verdict round
  at an iteration counts as its code review. It is the code review row with
  the verdict picked, never pre-ticked and never run on autopilot: it posts on
  GitHub.
- **Numbered among themselves.** Verdict rounds keep the `self-review` heading,
  key (`self-review:<n>`) and tally, so the composer's title switches to
  *PR verdict — round n* when the verdict is picked, and the Timeline labels
  them *PR verdict n*.

The loop, the discussion and the surroundings are not verdict-only: a `deep`
code review runs the same loop, and every round with a PR reads its
discussion. What a verdict adds is the decision and the post. The skill also
runs **outside workflows** — "review PR <url>" or `/clash-pr-review 123` on
anyone's PR posts a verdict unless asked for comments only or nothing posted —
where it writes nothing but its ledger and reports in chat.

## Waves

Some items are one task; some are a whole project. A project is planned as
**waves** — slices of the plan that are implemented, reviewed and approved one
after another, **inside the one item** (same worktree, same branch, same PR).
The planner decides: a project gets a `## Wave <n> — <title>` section per
wave in `plan.md` and a `waves.json` beside it; a single task gets neither.

```json
{ "waves": [
  { "id": "w1", "title": "Schema", "summary": "…", "dependsOn": [], "areas": ["db/migrations/"] },
  { "id": "w2", "title": "API", "dependsOn": ["w1"], "areas": ["src/api/audit/"] },
  { "id": "w3", "title": "Admin screen", "dependsOn": ["w1"], "areas": ["web/src/admin/audit/"] }
] }
```

Two optional fields tie waves to the work outside the item: `tickets` (keys
the wave delivers) and `prs` (PR URLs that deliver it — empty when planning,
appended by the executor as it opens each wave's PRs, the only `waves.json`
edit allowed in `implement`/`pr`). `assign_prs` places each of the item's
PRs: a wave listing its URL claims it; else a ticket key in its head branch
claims it for the **one** wave listing that ticket (a key two waves share
places nothing, and `TT-1536` is not `TT-15360`); a PR nothing claims is
*unplaced*, which the roadmap says rather than guessing.

`waves.json` is the planner's (phases `plan`/`revise`); where the waves stand
is clash's, in `meta.waves`: `done` (approved ids), `scope` (the current or
latest round's ids) and `starts` (the commit each wave began from). The
schedule is the pure `application::waves`:

- **Order.** A wave is *ready* when every `dependsOn` wave is approved,
  *blocked* otherwise; *active* while its round is being implemented
  (`implementing`/`changes-requested`), *in review* once the round handed
  back (`diff-review`, a review round, the PR stages), *approved* after the
  human's click. Its *level* (dependency depth) is the board's column.
- **Together.** Two waves are *compatible* when no dependency chain links
  them and their `areas` are disjoint (path prefixes; anything after a glob
  character is dropped, which only widens an area). A wave with no areas is
  compatible with nothing. Compatible ready waves may be picked for **one**
  round: the executor implements them side by side — one batch of
  implementers per wave — and the human approves them together. One round,
  not one session per wave, because the waves share a worktree and a git
  index: two agents committing in one checkout is two writers on one branch.
- **Validation.** `parse_wave_plan` rejects a missing or duplicate id, an
  unknown dependency and a cycle; the GUI shows the reason and nothing
  starts. `check_start` refuses a pick whose waves are not ready or not
  pairwise compatible — before the launch creates a worktree, so a refusal
  leaves nothing behind.

The flow, on the GUI's buttons:

1. At `plan-review`, **✓ Approve plan → start waves…** opens the wave
   picker (the first ready wave ticked; another can be ticked only when
   compatible with the pick) and launches the `implement` round over the pick
   — `start_workflow_agent(phase: "implement", waves)`, which records
   `meta.waves.scope` and each wave's start commit in the pre-spawn meta write
   and adds `Waves: w2 (API), w3 (Admin screen) of 3.` to the kickoff.
2. The executor implements only those waves, commits each separately, and
   hands back at `diff-review` (or `pr-draft`). Reviews and drift rounds get
   `Waves: w2, w3 (changes since <sha>).`: they judge the waves' commits, with
   the rest of the branch as context — a drift round does not call a wave
   that has not started *missing*.
3. At `diff-review` / `pr-draft` / `pr-ready`, **✓ Approve w2, w3 → next
   waves…** (`approve_workflow_waves`: scope → done, status unchanged) then
   offers the waves that became ready; **… only** approves without starting.
   With nothing waiting, **▶ Start next waves…** starts the next round. A
   change request keeps the round's scope (the fix round continues the same
   waves); one made between waves launches with
   `Waves: none of N (a fix round …; start no wave)`.
4. The item's own approvals — **Approve → done**, **Approve → PR draft**,
   **Mark done** — are hidden while waves remain; on the last round they
   approve its waves on the way (`wfTransition`). The PR may be opened at any
   point: it simply carries more waves as they land.

The **Waves** tab is a roadmap, and the tab a multi-wave item opens on. It
starts with what is happening and what to do next, in sentences
(`waveNarrative`: "w1 · Foundations is waiting for your approval. 2 of 8 PRs
merged, 6 still open. Next: … then approve it — that unlocks w2."), a track
of every wave's state and the same approve/start buttons as the bar
(`wfWaveDecisionButtons`, one set of handlers). Then the steps in order —
waves grouped by dependency level, saying which of a step's waves may share
a round — each wave with its state, PR progress (`merged/total`), summary,
linked tickets (URLs resolved from `[trackers]` by the GUI layer) and PR
chips; the scheduling facts (waits for, can share a round with, areas, start
commit) are folded under *Scheduling*. The header chip reads `Waves 2/5 · w3
in progress`.

**Many PRs.** Past `WF_HEAD_PR_LIMIT` (3) PRs the header shows the primary
and one summary (`10 PRs · 2 merged · 7 open · 1 not refreshed`) instead of a
chip per PR; it opens the **Pull requests** tab (any item with two PRs or
more): every PR in one list, grouped by wave (open before settled, unplaced
last) or as one group without waves, each row with its state, role, stack
position, `head → base`, unanswered threads, the error of its last refresh
and its action menu, plus **↻ Refresh all** and *Open N open PRs*.
`refresh_workflow_pr` asks about linked PRs side by side, six at a time (one
after another, two `gh` calls each, a ten-PR item took half a minute), and
records each failure per URL in memory (`GuiState.pr_errors` →
`WorkflowItem.prErrors`) instead of dropping it, so a PR that could not be
reached is marked stale rather than shown as current. The time of the last
check is `WorkflowItem.prCheckedAt`, from the refresh throttle — a PR
record's own `lastCheckedAt` moves only when its state changed, so it cannot
say how fresh the list is. The
next-step strip suggests *approve waves* once the round is reviewed and
*start waves* when nothing waits; neither runs on autopilot. GUI model:
`gui/dist/wf-waves.js` (`gui/tests/wf_waves.test.js`,
`gui/tests/wf_waves_smoke.test.js`).

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
with optional trailing fields: `Workspace: repo|directory.` (see Workspace
below; absent means the item's worktree), `PR skill: <name>.` (from the
`workflows.pr_skill` config — see PR integration) and `Interactive: yes|no.`
(absent means the skill's opening question asks in-session).

### Ticket trackers — every kickoff

Every agent launch (executor, reviewers, drift, explainer, dives) ends with
`Ticket links: …` so the agent links each ticket key it writes. The value
comes from the `[trackers]` config through the pure `application::trackers`
module:

- a tracker exists once its location is set: `trackers.jira.site` (keys link
  to `<site>/browse/{key}`) or `trackers.linear.workspace` (a slug or any
  `linear.app` URL; keys link to `https://linear.app/<slug>/issue/{key}`);
- Jira and Linear keys share one shape, so a key belongs to the tracker whose
  `prefixes` list its prefix, and any other key to `trackers.primary` (which
  falls back to the only configured tracker);
- one template is written bare (`Ticket links: https://acme.atlassian.net/browse/{key}.`);
  with prefix routes it is a table
  (`Ticket links: PS, OPS → <jira>; any other key → <linear>.`);
- `trackers.primary = "none"` gives `Ticket links: none.` (keys stay unlinked,
  no question); with no tracker configured the field is omitted, and every
  skill's *Linking tickets* rule makes the agent ask the human for the host
  (interactive) or leave the key unlinked and say so (autonomous), never guess
  one.

The template is for ticket keys only: external documents keep their own URL.
Config schema v3 moved the old `workflows.jira_*` keys into `[trackers.jira]`
(`jira_base_url` → `site`; `email`, `api_token`, `transport`, `skill` keep
their names), and v1.142.0's single `workflows.ticket_url` onto the trackers
(a `linear.app` URL → `trackers.linear.workspace`, a host or `…/browse/{key}`
→ `trackers.jira.site`, `none` → `trackers.primary = "none"`; any other
template is dropped). The migration runs on load and on the next save.

### Workspace — where an executor round runs

Decided per launch by the pure `resolve_work_dir` (`application::workflow`),
first match wins:

| Item state | Round runs in | Kickoff |
|---|---|---|
| `meta.worktree` recorded | that worktree | *(absent)* |
| `repoPath` is not a git repository | `repoPath` | `Workspace: directory` |
| `meta.workInPlace` set, or a `plan` / `revise` phase | `repoPath` | `Workspace: repo` |
| otherwise (a round that writes code) | a new worktree + branch, recorded | *(absent)* |

- **The worktree is created by the first round that writes code, not by the
  first launch.** Planning only reads the repository, and a checkout can take
  most of a minute on a large repo — so a plan round costs no checkout, and an
  item abandoned at `plan-review` leaves no branch behind. (`revise` counts as
  a code round in `review-only`, whose worktree exists from creation.)
- **A directory that is not a git repository is never refused.** There is
  nothing to check out, so the agent works on its files directly and the
  skill skips every git and PR step. The item has no diff: the Diff tab is
  empty and a change round freezes an empty `diff.patch`.
- **`meta.workInPlace`** (⚙ Settings → Workspace, clash-only) is the human's
  choice to never create a worktree: every round runs in the repository's own
  checkout, on whatever branch is checked out, and the skill neither creates
  nor switches branches. It is refused once a worktree exists — the recorded
  worktree is the item's checkout from then on.
- **The item's diff is taken in its own checkout only**: the worktree, or
  `repoPath` for an in-place item. An item with neither (a plan not yet
  implemented) diffs as empty rather than showing the main checkout's
  unrelated changes. A recorded worktree that is missing stays an error, so a
  change round cannot freeze an empty diff over work that exists.

The mode is repeated in the prompt (it is also in `meta.json`) so a
`review-only` run knows before reading anything that it must not write a plan.
Like the reviewers, every executor phase opens by settling interactive vs
autonomous — what "interactive" means per phase (a requirements discussion
before the plan — restate the task, ask until it is unambiguous, plan only on
the human's confirmation — then approach options before writing `plan.md`;
confirmation before a `wontfix` or a plan deviation; title/body preview before
a PR) is defined in the skill.

On every run, the agent first reads: `meta.json`, `plan.md`, `review.md`
(top-to-bottom — it is the accumulated decision history), the `open`
annotations in `annotations.json`, and the latest `history/<NNN>/diff.patch`
for context.

- **Phase `plan`**: in interactive runs, first settle what is being built —
  the agent reads `meta.json.description` (the human's own goal/scope
  statement, the primary intent when present) and the title, restates the
  task, asks about everything unclear (or for the feature itself when
  neither says enough) and writes nothing until the human confirms; the
  agreed understanding opens `plan.md`, and autonomous runs record an
  **Assumptions** section instead. Then write/overwrite `plan.md`; finish by
  setting `meta.json.status = "plan-review"`.
- **Phase `revise`**: revise `plan.md` and finish with `"plan-review"`. The
  plan is the artifact here and clash has already decided that: a change
  round is launched on `revise` only when the changes were requested at
  `plan-review` (see *Which phase a change round runs in* below), so the
  agent must not re-derive the target from the note's wording. In
  `review-only` mode there is no plan, so `revise` is always `implement`, as
  it is for an older item queued before clash named the phase.
- **Phase `implement`**: set `"implementing"` while working; address every
  `open` annotation — set it `"addressed"` with a short resolution appended
  to its `replies`, or `"wontfix"` with a justification; commit on the item
  branch (never push, never `--no-verify`); optionally create a draft PR
  (`gh pr create --draft`) and write its URL into `meta.json.pr.url`;
  finish with `"diff-review"` (or `"pr-draft"` when a PR was created).

### Which phase a change round runs in

Requesting changes queues a round; clash picks its phase from the stage the
request was made at, and records it as `meta.phase` (clash-only, like
`iteration`): `plan-review` → `revise`, every other stage → `implement`. It is
recorded because the round may be launched later, and by then nothing else
knows — `changes-requested` says a round is waiting without saying about what,
and `implementing` is where a plan approval and a code change round both land.
The same field is what a relaunch after a dead agent resumes. An item queued
before the field existed falls back to `revise` — exactly what it would have
been launched on — while one already *at* `implementing` falls back to
`implement`: it has an approved plan either way, and the phase that cannot
rewind the pipeline fails visibly rather than silently.

Letting the agent decide instead is what this replaced, and it fails in one
specific way: a note about a design mistake reads as being about the plan even
when the code it describes is already written and in a draft PR. The round
then rewrote `plan.md`, finished at `plan-review`, and the human was asked to
approve a plan again while nothing in the PR had changed. So an `implement`
round never writes `plan.md` and never finishes at `plan-review`; when the
note argues the approach itself was wrong, it implements the right thing in
the code and says the plan now describes something else — the human amends it
via ↩ Move back to… → plan-review.

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
- Never touch `history/`, `plan-history/`, `explainers.json` or `run.json`,
  and never change `iteration`, `reviewRound`, `appliedReviewKey(s)`, `phase`
  or `waves` — clash owns all of them (of the fields, the first two are written atomically by the
  request-changes flow, the third by the review launcher, the last by
  whichever of the two started the round).
- Write `annotations.json` **only** while status is `changes-requested` or
  `implementing` — during review phases the GUI owns the file (this phase
  split is what makes concurrent writes safe). Only `open` annotations are
  work; `parked` ones are human-owned (kept back from the round) and must
  never be touched, like `addressed`/`wontfix` ones.
- Read-modify-write `meta.json`; keep unknown fields.
- Overwrite `handoff.md` at the end of every phase, before the hand-back
  status write (shape under *Layout*).
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
`lastCheckedAt`, `unansweredComments`, `headRef`, `baseRef`) via the forge.

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
observed on refresh moves the item to `done`, and `draft == false` with
`state == "OPEN"` observed on a `pr-draft` item moves it to `pr-ready` (the
pure `status_after_pr_refresh` decides both). The agent may create the draft PR
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

### Linked PRs (multi-repo work)

One piece of work often lands as several PRs (backend + frontend + contracts).
`meta.json.linkedPrs` is an array of the same PR-block shape as `meta.pr`, for
change requests in **other** repositories that belong to this item. Rules:

- **They never drive the item's status** while a primary exists. Only the
  primary `pr` moves the item (`MERGED → done`, the `pr-*` stages); linked PRs
  are tracked, refreshed and opened alongside it, nothing more. The one
  carve-out is the **linked-only** item — no primary, so nothing else can
  close it: all of its linked PRs observed merged (at least one) moves it to
  `done`, the same auto-close the primary's merge gives PR-flow items (the
  pure `linked_only_all_merged`).
- The entry's `url` is the identity (`owner/repo#number` after parsing — two
  spellings of one PR are one PR); every refresh call about a linked PR is
  scoped to the `owner/repo` its URL names, since the item's checkout points
  at a different repository.
- The refresh is best-effort per entry: a linked PR that fails to resolve
  keeps its previous recorded state rather than failing the primary's refresh.
  Every action that changes or posts to several PRs follows the same
  discipline (see *Per-PR action scope* below): each call is repo-scoped and
  best-effort, per-PR failures are reported, and the action fails only when
  nothing at all succeeded.
- The **agent may append to `linkedPrs`** when its work opened PRs in other
  repositories — a URL-only entry is enough (clash fills state/draft on the
  next refresh, exactly like the primary's contract). The agent must never
  remove or reorder entries; unlinking is the human's (GUI) action.
- The GUI's Diff tab can render a linked PR's diff (fetched via the forge —
  there is no local checkout of that repo), strictly **view-only**:
  annotations always anchor to the item's own diff. Rounds scoped to a chosen
  PR carry it in the kickoff's `PR:` field (see the review contract).

#### Per-PR action scope

An item with several PRs makes every PR-shaped action ambiguous, and the
ambiguity is not resolvable by a default: "ready for review" is a statement
about *one* repository's change, so flipping three at once is a release while
flipping one is a step. Each action therefore asks which PR — and the answer
travels to the backend as **PR URLs**, resolved against the item's own records
by the pure `select_prs` (a URL the item does not track is rejected rather
than acted on, so a stale frontend cannot run `gh` against a PR nobody
linked). Omitting the pick means the primary, which is what every action did
before the scope existed.

**The answer is a set, for every action** — one PR, several, or all of them.
A four-repo change where three repos are ready and one is not has no
expression as one-or-all, and that is the normal shape of the work.

Agent rounds are included in that, and an earlier version of this feature
wrongly excluded them on the grounds that "a round is one agent session parked
on one item". That is a constraint on agent *concurrency* — two rounds at once
would be two agents editing one item's files, which the `reviewing` status
forbids — and it says nothing about how many PRs one round may read. A
cross-repo review is in fact the case that needs the set most: judging the API
PR without the web PR that consumes it leaves the contract between them
unchecked. So a round over several PRs is **one round over several diffs**.

What is genuinely per-action is only which PRs it can act on, and which start
selected:

| Action | Candidates | Pre-selected |
|---|---|---|
| Open PR | every PR | all — read-only, and "Open PRs (3)" promises three |
| Mark PR ready | **drafts** only; a merged or already-ready PR is never offered | the primary |
| Post round to PR | every PR | the primary |
| Code review | every PR, plus "this repository's own diff" as its own row | the own diff (what an unscoped round always read) |
| Answer PR comments | every PR | every PR with threads waiting (the primary when no count is known) |

The pre-selection rule is "the answer the button already promised": that is
the right answer for the single-PR case and the one nobody has to think
about. Two entries above are the deliberate exceptions — `open` is read-only
and its label promises every PR, and the respond action advertises the item's
whole unanswered-thread count, so opening its dialog with only the primary
ticked would agree to a fraction of what the button just said. Outside those,
a *linked* repository is never pre-selected: announcing a second repo is the
thing the human ticks deliberately. An empty selection is not an answer, so
the dialog's confirm button is disabled until something is checked.

Three consequences are contractual rather than cosmetic:

- **Only the primary moves the item.** Marking a linked draft ready records
  its new state and leaves the status alone; the GUI says so out loud, or a
  linked-only flip reads as a stage transition that failed.
- **A round scoped to a linked PR reviews that PR's diff**, fetched from the
  forge, and its findings are published on that PR — not written as
  annotations, which would anchor to files this repository does not have. The
  reviewer skill states the same rule; clash pre-selects the matching publish
  mode as soon as a linked PR is picked.
- **A round over several PRs must report per PR.** The kickoff names them all
  (`PR: <url>, <url>`) and the skills require the round report to say which
  ones it read and, for a respond round, what it published where — a round
  that answered two repositories and reported one total cannot be audited.

#### PR stacks

A **stack** is a chain of PRs in one repository where each PR's base branch
is the previous PR's head: each layer's diff is only its own change, and the
layers merge bottom first. GitHub has no stack object — `gh pr view`'s
`baseRefName`/`headRefName` chain is all there is — so that chain is the
definition, and the item's own PRs (primary + linked, linked in the *same*
repository included) are the input. clash discovers nothing on GitHub by
itself; the PRs must be linked.

**Setting.** `workflows.pr_stacking` (bool, default **off**), overridden per
item by `meta.prStacking` (`on` | `off`, absent inherits) in the item ⚙
Settings tab. Off by default because it changes what reviews read and what PR
actions pre-select, and an item whose PRs merely share a repository must not
start behaving differently on upgrade. The effective value is the pure
`application::pr_stack::stacking_enabled`: the item override wins, and any
forge other than GitHub is always off.

**Ordering** is the pure `order_stack` over the PRs' recorded `headRef` /
`baseRef` (filled by every PR refresh and attach — no extra forge call, so an
unrefreshed PR is simply not placed yet). It returns one chain base → tip, or
no stack when nothing builds on anything, and it **reports instead of
guessing**: a fork (two PRs on one parent), two PRs from one branch, a cycle,
several chains, or a **gap** — two PRs of one repository starting from
different branches with neither building on the other, i.e. a middle layer
the item does not track. Closed PRs are never layers; a merged layer stays one
until GitHub retargets its child. The result rides `WorkflowItem.prStack`
(`layers: [{url, headRef, baseRef}]`) or `prStackIssue` (the reason, shown as
*⚠ stack unclear* on the item header and the ⇄ dashboard); nothing about it is
stored.

When a stack exists:

- **Order everywhere.** `itemPrs(meta, item.prStack)` splices the layers in
  base → tip where the first of them sat and tags each `stack: {pos, of,
  parent}`; chips show `2/3`, and every scope selection follows that order, so
  fan-outs walk parent before child.
- **Defaults.** A code or drift round's composer pre-ticks the whole stack
  (`prStackUrls`); *Answer PR comments* pre-ticks the stack plus any PR with
  threads waiting; a verdict already posts on every open PR. Mark-ready, post
  and open keep their own rules — readying a whole stack is a release.
- **The item's own diff** is taken from its parent layer's head when
  `meta.base` is empty (`item_stack_base`); from the trunk it would contain
  every layer below. Without a PR of its own yet the item is the *next*
  layer: its parent is the tip of the stack its linked PRs in the same
  repository form, or the only such PR.
- **Opening PRs.** *Create draft PR* targets `meta.base`, else that parent's
  head; when several linked PRs could be the parent it asks for the branch and
  saves it as the diff base instead of guessing (`stack-unclear:`). The
  executor's `implement` and `pr` kickoffs state the same answer as
  `Stacking: on.` (bottom layer), `Stacking: on (base: <branch>).` or
  `Stacking: on (unclear: <reason>).` — and say nothing when stacking is off,
  which the skill reads as "never base a PR on another PR's branch".
- **Kickoff.** Every review round records `meta.review.stack` and states it as
  `Stack: <url1> > <url2> > <url3>` (base → tip). It is context, not scope:
  `PR:` still says which PRs the round was pointed at. `clash-pr-review` then
  reviews **every layer** when `PR:` is absent (in
  an interactive round they first ask, multi-select, whole stack or which
  layers), measure each layer against its parent (`gh pr diff`, or
  `git diff origin/<parent head>...HEAD` locally), check sibling layers
  before calling anything missing, and report per PR in stack order. Run
  standalone, `clash-pr-review` finds the stack itself with `gh pr list
  --head/--base`, bounded to 10 layers and stopping at a fork.

An explicit diff base (⚙ Settings → Diff base) always wins over the stack's
answer.

### Sharing, export and webhook notifications (clash-side, informative)

These are clash features, not agent contract — listed here so a reader of this
file knows they exist and what they read. The GUI's share dialog composes one
markdown document from the item's files and sends **exactly the previewed
text**. The sections, each optional:
- summary and `handoff.md` ("where it stands");
- `plan.md`;
- the overview write-ups of the two explanations and of plan vs changes;
- `review.md` iteration notes;
- `agent-review.md` verdicts;
- open annotations;
- the current diff, one fenced block per file.

Agent-written documents keep their content, but their headings are pushed
under their section.

With **Contents & links** (`toc`) and at least two sections,
`build_share_markdown` adds:
- a `## Contents` list of every section and its parts;
- `[↑ Contents](#contents)` at the end of every section;
- links from each open comment to its file's diff heading.

The anchors are GitHub's heading slugs (`heading_slug`, repeats numbered by
`Slugger`), computed over every heading of the finished document.
`shareHeadingSlugs` in `gui/dist/wf-share.js` mirrors the rule: the `.html`
export gives each heading that id, and the preview follows a `#slug` link by
finding the heading with it, without adding ids to the app's DOM. The chat
presets leave `toc` off, because an anchor is dead text in a channel.

The document can go to a
clipboard, a `.md`/`.html` export, a Slack/Discord webhook configured in
`config.toml` (`workflows.slack_webhook` / `workflows.discord_webhook`), or a
ticket as one comment — Jira (`trackers.jira.*`; the markdown is converted
to Jira wiki markup) or Linear (`trackers.linear.*`; markdown through the
GraphQL API, the issue resolved from its key first). The ticket key is
prompted per send, pre-filled from `meta.ticket` (read under its old name
`jiraTicket` too) or the item's title/branch.

The `.html` export is the one destination that carries more than the
markdown: it is a single self-contained page meant to be sent to people
without clash. It works offline and opens by double-click, with no server and
no sidecar files. The dialog's **Explanation drawings** picker (`Every level` /
`Overview + parts` / `Overview only` / `None`) adds the explanation trees
(*Drill-down explanations* above), and the page then opens on an **explorer**
built for a reader rather than a copy of the GUI tab:

- the level's drawing fills the stage. Clicking a part zooms into it: the
  drawing grows until that box fills the screen while the part's own level
  grows out of it. **Zoom out** (Esc), the path bar or browser back shrinks it
  back into its box. `←`/`→` move to the next part of the same level;
- a part with no level in the page zooms into a card of what the level above
  says about it: summary, why it matters, how it connects, where it lives;
- a level with no drawing gets a map of its parts as cards;
- a reading panel beside the stage (`r` or **☰ Read** toggles it) names the
  level, lists its parts (hovering one lights it up in the drawing, clicking
  zooms in) and holds the write-up, or just the write-up's section about the
  part on a part card;
- a level's extra views (flow, rollout…) are angles of the same stage;
- the share document is the **Document** chapter, and each explanation is a
  chapter of its own. A light/dark/auto toggle sits in the header.

The explorer is `shareViewer` in `gui/dist/wf-share.js`. It is shipped inline
with `toString()`, so it must not use anything else in the module (a test pins
that). It reads the levels from one `<script type="application/json">` block
whose JSON has no `<` at all, so no string in it can close the element. The
document reads without script and prints; the explanations need JavaScript. A
share without explanations is the plain document and carries no script. Three
rules keep the page safe and honest:

- the agent's drawn `.html` goes into an `<iframe sandbox="allow-same-origin"
  srcdoc>` with no `allow-scripts`, exactly like the GUI's tab, and never into
  the page's own markup: it travels as data, and the explorer puts it in the
  frame;
- the levels are read only through the explanation readers
  (`get_workflow_explain_tree` / `_node`, `get_workflow_doc`), which serve
  whitelisted documents. clash's own files (`meta.json`, `explainers.json`,
  `run.json`) and session ids have no way in, and a test pins both readers
  and the markdown builder;
- markdown is rendered in the webview (sanitized, mermaid as inline SVG on an
  attached off-screen stage), so what is saved is what the GUI renders. Each
  diagram is drawn twice, once per mermaid theme, and the page's theme shows
  one (the reader can switch theme after the colours are baked). The ids inside
  each drawing are prefixed with its own, because mermaid gives some defs fixed
  ids (`arrowhead`) and a visible drawing would otherwise use the markers of the
  hidden one. A figure keeps its source behind Diagram / Source tabs made of two
  radios and `:checked` rules, so they work in the script-free page too;
- both page shapes share one document design (`shareDocSection` + `DOC_STYLE`):
  full width, a contents rail built from the h2/h3 ids (`shareDocOutline`, two
  sections or more), scroll-spy in the explorer page only.

Saves go through the native save dialog, seeded with a clash-derived name
(`default_share_export_path`: `clash-<project>-<slug>.<ext>` in Downloads). The
returned path is checked by `workflow_share::validate_export_path`: absolute,
no `..`, extension forced. The page is pure functions in `gui/dist/wf-share.js`
(`explainLevelsToShare`, `shareExplanationChapter`, `shareHtmlDocument`,
`shareViewer`). Write-up heading ids are prefixed with their level's anchor
(`x-explain-plan-3--…`) so they never collide with the document's.

Who posts each destination is a setting, not a fallback chain:
`trackers.jira.transport` / `trackers.linear.transport` /
`workflows.chat_transport` are `agent` (the
default) or `clash`. On `agent` the document is written under
`<clash data>/share/` and a Claude Code session is launched with the
destination stated in the kickoff (`workflow_share::share_prompt`) — routed
through `trackers.<tracker>.skill` / `workflows.chat_skill` when one is named,
otherwise using whatever tooling that session has connected (an MCP server for
the destination, say), which is how a share reaches services clash has no
client for. On `clash` the webhook/token transport above is used and nothing is
launched. Relevant to a skill author in one way: if a skill of yours is named
there, it will be invoked with a file path, a destination and an instruction
that the document is the message, to be posted as written — and if it is not
installed in that session, the same prompt tells the session to fall back to
its own tooling rather than stop. Nothing is ever posted without an explicit human action, with one
opt-in
exception: `workflows.notify_webhook` (`off` by default) announces items that
an **agent** parks at a decision state — the same events as the desktop
notification, suppressed for clash's own writes by the attention ledger.
Agents never call these; they just mean the files above have readers beyond
the GUI, which is one more reason to keep their formats stable.

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

## Check runs

Every agent pass that judges or explains an item **without moving it** — the
stage's review (which may end in a verdict on the PR), plan vs changes, the
two explanations, answering PR comments — is launched through one entry point
on the action bar: **🔍 Check…** opens a checklist (any subset, each row with
its own options: depth, how the review ends, focus, PRs; shared: agent, interaction mode,
auto-apply), and **▾** runs one pass on its own through that pass's own
composer. A pick runs as one **check run**, recorded in the item's clash-only
`run.json`:

- **Explainers start at once and run alongside**; they never park the item and
  never block the run.
- **Judging passes run one after another**, in a fixed order whatever was
  ticked first (`workflow_run::pass_rank`: plan review → code review → plan vs
  changes → answer comments → verdict, the verdict last because it should
  include everything found before it). A verdict is the code review row with
  *Post a verdict on the PR* picked, recorded as a `self-review` pass
  (`listedPasses` keeps it off the menus as a row of its own). Only one judging agent
  owns an item at a time, which is why they cannot run in parallel.
- **Every pass judges the same iteration.** Nothing is applied between them,
  even when a round says `Apply: yes`. A run whose iteration moves under it
  (someone requested changes) or whose stage changes (someone approved) closes.
- **Then at most one change round.** When every judging pass has settled, the
  rounds that are worth applying become one change-request note
  (`combinedApplyNote` in `gui/dist/wf-checks.js`, one section per round) —
  one fix round instead of one per check. With auto-apply on, clash applies
  the rounds that said `Apply: yes` itself; otherwise the strip offers
  **↻ Apply** (apply now, or edit the note first in the change composer). A
  run whose rounds all said `Apply: no` closes with nothing to apply.

One ticked pass is exactly the standalone round, so "run all", "run some" and
"run one" are one mechanism. Every round still writes its own
`## Review N — <target>` entry, with its own per-target number.

**`run.json` records what clash intended; everything that happened is read
back from the files the agents and clash already write.** That is what makes a
run resumable. The pure `application::workflow_run::reconcile` compares the two
on every step and settles each pass:

| Recorded | Seen on disk | Decision |
|---|---|---|
| pass `launching` | `meta.review` of its target, started after the launch | `running`, with its round and session |
| pass `launching` | nothing, past the 2-minute grace, no launch in flight | queued again (failed after 3 attempts) |
| pass `running`, item parked again | that target's round count reached the pass's round | `done` |
| pass `running`, item parked again | no round entry | `failed` — the round ended without a report; the run pauses |
| apply `pending`/`applying` | `meta.appliedReviewKeys` holds every key | closed as applied |
| apply `applying` | keys absent, past the grace | `pending` again |
| any | `meta.iteration` moved, or the item left the run's stage | closed |

Write order is the launchers' own: a pass is marked `launching` **before** its
spawn, and the apply is claimed (`workflow_run_begin_apply`, refused unless one
is pending) **before** `workflow_request_changes`, which stamps the run's keys
into `meta.appliedReviewKeys` in the same meta write that bumps the iteration —
that stamp is the proof the apply happened. The backend command
`workflow_run_step` holds a per-item lock, reconciles, writes `run.json` only on
a real change and launches what the reconcile asks for through the same
`launch_review` a click uses. The GUI calls it after every hand-back, at boot
and on every workflow refresh, so a run closed mid-way by a restart resumes on
the next launch; while clash is closed, nothing advances.

The human decides **before** (the checklist, or the strip's chips) and
**during**: the strip shows `Checks · 1 of 2 done · code review running · next:
plan vs changes` with **Skip next**, **Stop after this one**, **Retry** /
**Skip** on a failed pass, **↻ Apply** when findings wait, and **✕** to close
the run (a round already running carries on as an ordinary round). A pass that
failed for want of a PR asks for it and retries; a code review that was to post
its findings can run locally instead (`local` control) — a verdict or a reply
to a thread cannot, because their output is the post.

## Next-step assist

Every item's action bar opens with a **Suggested next** strip: what to do now,
the reason, and the checks that were already settled on the way to it. The pure
`wfNextStep` (`gui/dist/wf-next.js`, tests in `gui/tests/wf_next.test.js`)
walks an ordered list of checks per stage, for example at `diff-review`:

1. a review round is waiting to be applied (and did not say `Apply: no`) → **Apply**
2. comments are still open → **Request changes**
3. PR threads are waiting on you → **Answer PR threads**
4. the code has no review at this iteration → **Code review** (`deep` when an
   agent's `Next:` said `deep-review`)
5. there is a plan and nothing has compared it with the change yet, or an
   agent's `Next:` asked for it → **Compare plan vs changes**
6. otherwise → **Approve** (to the PR stage when a PR exists, else done)

The first that fires wins, except that **every check pass that is due is
recommended together**: when the winner is a check (3–5), the strip shows all
due checks as chips with one **▶ Run N checks** button, plus a refresh of any
explanation describing an older iteration when the comparison is among them.
Passes that post on GitHub (answering threads, a code review posting comments
or a verdict — `passPosts`) are shown unticked. Unticking a chip shrinks the run before it starts; **Options…** opens
the checklist with that pick. A run in flight replaces the recommendation with
its own strip (see *Check runs*).

Three properties are the design. The recommender is **the one owner of the
primary button**: `wfRenderNextStep` runs after the bar is rendered, picks
among the buttons that are actually there (`data-act`) or the checks the stage
allows, and moves the highlight, so a suggestion is always clickable and the
bar's gates stay the only gates; the per-stage `primary` classes survive only
for `assist = off`.
**Agents advise, clash ranks**: a round's or a hand-off's `**Next:**` is
consulted only where clash's own checks are silent, so no agent can recommend
approving over an unapplied round or an open comment. And "reviewed" means
**reviewed at this iteration**: every review launch stamps
`meta.reviewMarks[<target>] = { iteration, round }` (clash-only), and the round
counts once that target's tally reaches `round`, so a fix round makes its code
due for review again while a round still in flight does not count. Items
predating the marks count any past round as current rather than being nagged.

`workflows.assist` (`suggest` | `autopilot` | `off`, default `suggest`)
decides how far it goes. Under `autopilot`, every agent hand-back (after the
item's own run, if any, has advanced) starts the recommended checks **that post
nothing** as a check run: reviews, plan vs changes, explanation refreshes.
Approvals, PR flips, change requests (whose note is yours) and anything posted
on GitHub always wait. The runs it starts are autonomous and pre-authorized to
apply, so review → one fix round → review continues until the reviewers say
there is nothing to apply; `WF_AUTOPILOT_BUDGET` (3 runs per item, counted in
`run.json` so a restart cannot reset it) stops a reviewer that never does, and
any click on the item's bar resets the budget. Autopilot never starts a run
while one is open.

**Explanations follow the comparison.** A plan-vs-changes round is read next
to the two explanations, so launching one offers to refresh every existing
explanation written for an older iteration (`staleExplanations`: its
`explainers.json` record carries the `iteration` it was launched against; no
record counts as stale). The rows are pre-ticked and join the run as explainer
passes, and autopilot includes them without asking. They run **alongside**: the drift round never waits for them,
because its ground truth is `plan.md` and the diff, and it adopts
`explain-plan.md`'s numbering only when that explanation is current.

## Lead and subagents

Every workflow session runs on one pinned **lead model** per harness —
`workflows.lead_model` (default `claude-opus-5-5`) under Claude Code,
`workflows.omp_model` (default empty: omp's own) under OMP — rather than
whatever the user last selected, so rounds are reproducible — two review rounds on one item
are comparable because the reviewer was the same model both times. The lead
does the thinking: requirements, planning, splitting, integrating, judging.

Under `workflows.delegation = team` (the default) the lead may fan the work out
to parallel subagents, which run on `workflows.subagent_model` (default
`claude-sonnet-5-5`) under Claude Code and `workflows.omp_subagent_model`
(default empty: omp's own `task` role) under OMP. `team` is permission, not a quota: every skill makes the
lead **size the round first** and pick the smallest split that keeps quality —
no split when the work fits in a few reads, one subagent per independent area
(subsystem, repo, PR) otherwise, and a per-concern split only for large or
`deep` work. A subagent costs a brief, a cold re-read of the code and the
lead's check of its result, so one whose job is a single file read is slower
than doing it, and a fixed fan-out (four lenses on a three-line diff) is
ceremony. The table is the split **at full scale**:

| Skill | Fan-out | Second pass |
|-------|---------|-------------|
| `clash-workflow` · `plan` | one explorer per area the task touches | the lead checks the facts the plan rests on |
| `clash-workflow` · `implement` | the plan's non-trivial work units, in batches of disjoint files (one batch per wave when a round covers several compatible waves) | one verifier over the whole diff (the lead's own re-read for a small one) |
| `clash-plan-review` | one reviewer per area, split by section (architecture, quality, tests, performance) only for a large plan | refutation, batched per area |
| `clash-drift-review` | tracers over the plan's actions grouped by area, + one over unexplained hunks for a large diff | refutation, batched per file or area |
| `clash-pr-review` | one reviewer per area/PR applying every lens (correctness, tests, conventions, security/perf), split by lens only when large or `deep`; a looping round (`deep`, a verdict) takes a fresh wave **per pass**, each briefed with the pass's angle and the ledger's findings as "already found" | refutation of every new candidate, every pass |
| `clash-explain` | one mapper per area too big to read in a few files | the lead spot-checks every name it writes |

Verification is proportional too: the lead settles a claim itself when one
read does, and sends it to a fresh refuting verifier only when refuting it
takes real tracing — never one verifier per trivial finding. The final message
reports how the round was sized and split, so a round that delegated nothing
says why.

`revise` and `pr` stay with the lead: each writes one document, and a document
is one voice. The fan-out is for speed; the second pass is for the findings —
a claim checked by a subagent that did not make it is what keeps a parallel
review from being a louder one.

Two halves enforce it. The kickoff always states the choice
(`Delegation: team (subagents on <model>).` or `Delegation: solo.`, right after
`Mode:` — the pure `Delegation::clause`), and each skill's *Lead and subagents*
section says how to size it; the models themselves are pinned by the launch,
not left to the lead (`Agents::with_workflow_models`, pure rules in
`delegation_env` / `omp_subagent_overlay`):

| | Lead | Subagents |
|---|---|---|
| Claude Code | `--model <lead_model>` | `CLAUDE_CODE_SUBAGENT_MODEL=<subagent_model>` **and** `CLAUDE_CODE_SUBAGENT_MODEL_FORCE=1` |
| OMP | `--model <omp_model>` | `PI_CONFIG_FILES` += a clash-written overlay (`<clash data>/omp/workflow-models.yml`) setting `modelRoles.task` and `task.agentModelOverrides.task` |

Both pins exist because the obvious one is not binding. Claude Code reads
`CLAUDE_CODE_SUBAGENT_MODEL` only as a *fallback*: a per-call `model`, an agent
definition's `model:` frontmatter and the built-in Explore agent all beat it,
so without the force flag the lead or any custom agent could land elsewhere —
with it, everything but Explore runs on the pin. omp resolves a subagent's
model as per-call model → `task.agentModelOverrides[<agent>]` → the agent's own
`model` (the general-purpose `task` agent's is the `@task` role) → the session
model, so the overlay sets both the role and the override; overlays deep-merge
over the user's own `config.yml` (theirs → project → overlays), so their other
roles survive, and an existing `PI_CONFIG_FILES` is appended to, never
replaced. The skills therefore tell the lead never to pass a `model`, and to
brief the general-purpose agent (`general-purpose`, never `Explore`; omp's
`task`).

**Every relaunch re-applies them.** `claude --resume` and `omp --resume` keep
none of the first launch's flags or env, and a workflow session is resumed on
every clash restart (updates included), every ⟳ reload and every stashed
session brought back — so pinning only the first launch meant most workflow
sessions ran on the agent's default model with unpinned subagents. A workflow
session's registry entry carries `"workflow": true`, and `Agents::relaunch_with`
applies the same `with_workflow_models` for it, from the *current* settings
— except a sized review round's override, which rides the entry (below).
The lead alone writes the item's files, runs git, posts to PRs, asks the
human and changes the status; every brief says so. An empty subagent model
leaves the harness's own choice (Claude: inherit the lead's; omp: its `task`
role).

### Sized review rounds

A code review (`diff`, and its verdict rounds, `self-review`) picks its models
from the **size of the change** they read, so a big change gets stronger
finders and a tiny one does not burn the most expensive lead. Plan, drift and
explain rounds keep the regular pins. `launch_review` measures the change
before recording the round — the item's own diff (the one its Changes view
shows, `workflow_diff_text`) for an unscoped round or the primary PR, plus each
other picked PR's diff from the forge (`gh pr diff --repo`); an item with no
checkout to diff falls back to its primary PR's diff. The pure
`application::review_size` decides:

| Size | When | Lead | Subagents |
|------|------|------|-----------|
| `small` | ≤150 changed lines (additions + deletions) **and** ≤5 files, one PR | `workflows.small_review_model` (default `claude-sonnet-5-5`) | `workflows.subagent_model` |
| `medium` | anything else | `workflows.lead_model` | `workflows.subagent_model` |
| `large` | ≥800 changed lines **or** ≥25 files | `workflows.lead_model` | `workflows.large_review_subagent_model` (default `claude-opus-5-5`) |

More than one PR in scope (several picked, or a stack) is never `small`. An
empty tier setting falls back to the regular pin, and a measurement that fails
(or finds no change) is `medium` without a `Size:` field — never a failed
launch. `workflows.review_sizing = off` measures nothing: exactly the regular
pins and no `Size:`. **Claude Code only** — an OMP round still states its size
in the kickoff, but its models are the user's providers', so it keeps its own.

The kickoff carries `Size: <small|medium|large> (<N> lines, <F> files).` right
after `Depth:`, recorded as `meta.review.size` / `sizeLines` / `sizeFiles`, so
the skill scales its passes to the change. The delegation clause names the
subagent model the round really runs on. **A relaunch keeps the launch's
choice**: the override rides the session's registry entry (`lead_model` /
`subagent_model`, only where it differs from the pins), and
`Agents::relaunch_with` prefers it over the current settings — a resumed
round does not change models halfway through because the pins moved.

## Embedded skills — install as a decision

The five skills (`clash-workflow`, `clash-plan-review`, `clash-pr-review`,
`clash-explain`, `clash-drift-review`) are compiled into both binaries. Installing them under
`<claude_dir>/skills/` becomes a **decision only where it could lose work**:

- `sync_unattended` runs at every startup and needs no permission: it
  installs **missing** skills, refreshes every skill whose file is still
  exactly what clash last wrote, and removes retired dirs clash wrote. A new
  release shipping new skill text is not a question — nothing of the user's
  is at stake, and asking anyway trained people to dismiss the popup.
- What survives that is the real diff, and the only thing that asks:
  `plan_install`'s **locally-edited** skills (changed upstream AND
  hand-edited since clash last wrote them) and **retired-edited** dirs. The
  GUI shows a startup popup — **Keep my edits** / **Overwrite with the new
  skills** — or applies the `general.skills_update` setting silently when it
  is not `ask` (Settings → Workflows → *Skill updates*). Esc defers: asked
  again next launch.
- `apply_decision` performs the choice and stamps the manifest's
  `resolvedFingerprint` — a hash of the embedded set, so the question returns
  only when the skills actually change again (fingerprint, not version: dev
  builds change content without a bump). `sync_unattended` stamps **only when
  nothing is left to ask**, so it can never swallow a pending decision.
  `all` removes every retired dir; `untouched` (and the unattended sync)
  spares a hand-edited one for the same reason it spares a hand-edited
  current skill; `keep` removes none.
- The manifest (`.clash-skills.json`) records the per-skill hash **as last
  written by clash** — the "was it hand-edited since?" oracle, and now also
  what tells an abandoned retired dir from someone's fork of one. Kept local
  edits keep their old hash, so they stay detectable as edits forever.
- The TUI has no popup: it runs the same unattended sync, honors a non-`ask`
  setting, and otherwise logs a pointer to the GUI.
