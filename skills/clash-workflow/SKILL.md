---
name: clash-workflow
description: Execute one phase of a clash Workflow item (plan | revise | implement | pr) in one of its entry modes (full | from-plan | review-only). Invoked by clash's GUI with a kickoff prompt naming the item directory, phase and mode. Reads meta.json/plan.md/review.md/annotations.json, does the work in the current worktree, addresses open diff annotations, and hands the item back to the human via a status transition. Triggers on "Use the clash-workflow skill", "Workflow item directory:", or when asked to run a clash workflow phase.
---

# clash-workflow — one pipeline phase per run

> **Agent CLI.** clash launches this skill under Claude Code or OMP (oh-my-pi).
> `AskUserQuestion` below names the structured-question tool: in OMP it is `ask`
> (a `questions` array, each with `id`, `question`, `options`, optional `multi` and
> `recommended`). Every other instruction is the same under both.

You are the executor half of clash's Workflows feature. clash (the GUI) is
the cockpit: the human creates items, reviews your output, annotates diffs,
and clicks the buttons. You do the work and communicate **exclusively
through the item's files** — clash watches them and updates live. The full
contract lives in the clash repo at `docs/workflows.md`; this skill is its
implementation.

The kickoff prompt gives you:
- **Item directory** — absolute path to `<workflows_root>/<project>/<slug>/`
- **Phase** — `plan` | `revise` | `implement` | `pr`
- **Mode** — `full` | `from-plan` | `review-only` (also in `meta.json.mode`;
  a missing value means `full`)
- **PR skill** — optional; the skill to open pull requests with (see the PR
  steps below)
- **Stacking** — optional, only in `implement` and `pr` phases; whether this
  item's PRs form a GitHub stack, and what its PR builds on. See "PR stacking"
  below — it decides every PR's `--base`.
- **Waves** / **Signed off** — optional, only on a multi-wave item: which
  waves this round covers, their branches, and which waves are accepted. See
  "Waves" below.
- **Decisions** — optional: the questions earlier hand-offs asked the human,
  and their answers. See "Decisions" under the hand-off note below.
- **Interactive** — optional; see the opening question below
- **Delegation** — `team` (also what an absent field means) or `solo`,
  optionally naming the subagent model: `team (subagents on <model>)`. See
  "Lead and subagents" below.
- **Workspace** — optional; what your shell cwd is:
  - absent — the item's own git worktree. All code work happens there, on the
    already-checked-out branch.
  - `repo` — the repository's main checkout, shared with the human. A `plan`
    or `revise` phase only reads it (clash creates the item's worktree when a
    round first writes code). In an `implement` or `pr` phase the human chose
    to work in place: work on the branch that is checked out. Never switch,
    create or delete branches, never add worktrees, never stash or discard
    changes you did not make — the human decides whether the work gets its
    own checkout.
  - `directory` — not a git repository. Work on the files there directly and
    run no git at all: skip every commit, push and PR step below and say so in
    your final message. A `pr` phase has nothing to open: stop and report it.
- **Repos** — present instead of `Workspace` on a **workspace item**: the
  item lives in a folder of repositories, not one, and your cwd is that
  folder. See "Workspace items" below.
- **Setup** — optional, in code-writing phases: each repository's own
  worktree setup (`repo: \`script\``), which a fresh worktree has not run.
  Run it from the worktree root before you need what it provides (tests,
  builds, commit hooks that call bundled tools); it may be slow, so not when
  the round only reads code. A setup that fails or that you skip goes in the
  hand-off's *Checked*, with what it stopped from running. Never commit
  `--no-verify` to get around a hook the setup would have made work.

## Workspace items

`Repos: from the ticket and the plan — this item lives in the workspace <dir>…`
means no repository is fixed in advance. Read the ticket (its key is in
`meta.json`'s `ticket`, or in the title/description) and the plan to decide
which repositories under `<dir>` the work changes, and name them in the plan
(and in each wave's `areas`, as `<repo>/…` paths).

- **Worktrees.** In each repository you change, work in the worktree the
  field names — `<dir>/<repo>-worktrees/<branch>` on the item's branch, or
  `<dir>/<repo>-worktrees/<wave branch>` for a wave. Create it when it is
  missing, from the repository's default branch, with the command the field
  gives (`git -C <dir>/<repo> worktree add ../<repo>-worktrees/<branch> -b
  <branch> origin/<default>`; `git -C <dir>/<repo> fetch origin` first).
  **Never commit in a repository's main checkout** — it belongs to the human.
  clash finds the item's repositories again from those branches and its PRs,
  so the branch name is the link: never use another one.
- **Already in play** lists the repositories that already hold the item's
  branches, with their worktrees: continue there.
- Git and PR rules below apply per repository, run inside its worktree. Every
  PR is opened from its worktree's branch and recorded per "Recording PRs".
- A `plan` or `revise` phase reads the repositories' main checkouts and
  creates nothing.

## Opening question — interactive or autonomous

Before doing anything else, settle how this run goes:

- Kickoff says `Interactive: yes` → run interactively, no question asked.
- Kickoff says `Interactive: no` → run autonomously: no questions, decide
  alone, report your calls in the final message.
- **The field is absent → ask.** One `AskUserQuestion`, first thing:
  1. **Interactive** (recommended) — check in at the phase's decision points
     before acting on them.
  2. **Autonomous** — decide alone; every judgement call is reported at the
     end instead of asked.

What "interactive" means per phase:
- `plan` — first hold the requirements discussion (phase step 1: restate the
  task, ask the open questions in one round with your recommended answers,
  proceed only on the human's confirmation), then — before writing `plan.md` — present the 2–3 viable
  approaches with a recommendation and ask which to plan around.
- `revise` — when the change-request note is ambiguous or conflicts with an
  earlier decision in `review.md`, ask instead of guessing.
- `implement` — ask before deviating from the plan, before marking any
  annotation `wontfix`, and before creating the optional draft PR.
- `pr` — show the title and body before creating the PR.

Blocking on a question is safe: the human launched this session from clash and
is watching it. Wait for answers; never time out and decide for them. At any
point they may answer "continue autonomously" — from then on, stop asking.

## Modes — read this before anything else

- **`full`** — the canonical pipeline described below. You write the plan.
- **`from-plan`** — the human supplied `plan.md`; you never wrote it. Treat it
  as the approved intent. Otherwise identical to `full`: you only ever get
  `revise` (plan changes) or `implement`.
- **`review-only`** — **there is no plan and there never will be one.** The
  code already existed when the item was created; the human is reviewing a
  branch or PR (`meta.branch`, `meta.pr`) and your only job is to address their
  diff annotations. In this mode:
  - Never create or write `plan.md`; never set status `plan-review`.
  - Phase `revise` always behaves exactly like `implement`.
  - Always finish at `diff-review`.
  - The branch is pre-existing and shared, so **do push** after committing
    (see Git below) — this is the one mode where pushing is expected.

## Step 0 — always read first (fresh, every run)

1. `meta.json` — mode, status, iteration, branch/base, PR info, and
   `description` (the human's own statement of what this item is about —
   when present, it outranks the title as the intent). Parse leniently.
2. `plan.md` — the current plan, and `waves.json` when the item is a
   multi-wave project (see "Waves"), with `meta.waves.done` saying which
   waves are signed off.
3. `review.md` — the accumulated decision history (`## Iteration N` sections
   with the human's notes + the open annotations at each round). Later
   sections override earlier ones. Read the **latest** section in full — it
   is this round's instruction. Earlier sections are history: on a short file
   read them all; once the file is long, list its headings
   (`grep -n '^## Iteration' review.md`) and read an earlier section only
   when the latest note refers back to it or you need why a decision was
   made. A note that applies a review round by reference ("read them there")
   means that round's section of `agent-review.md` is the work list — read
   it in full.
4. `annotations.json` — every annotation with `"status": "open"` is work you
   MUST address this run (see phase `implement`). Ones with
   `"author": "agent"` came from a review round; treat them exactly like the
   human's — except that one may carry `"confidence": "medium"|"low"` (and a
   `confidenceBasis`): verify it against the code before changing anything
   for it, and when it does not hold, mark it `wontfix` with a reply saying
   what you checked. Findings in `agent-review.md` carry the same as a
   `**Confidence:**` line.
5. `agent-review.md` — the review rounds run on this item, if any. Read the
   latest; its open findings are work.
6. The latest `history/<NNN>/diff.patch` if present — what the human last
   reviewed, useful context for what changed since.

## You are the executor, not the reviewer

Two separate skills run review rounds on the same item — `clash-plan-review`
(judges `plan.md`) and `clash-pr-review` (judges the diff). They write
findings into `agent-review.md` and `annotations.json` and never touch
`plan.md` or the code beyond trivial fixes. Two consequences for you:

- `agent-review.md` is **input** for you — read the latest round before
  implementing or revising; its findings are the same kind of work as the human's
  annotations. Never write to it.
- `reviewing` is not a status you may write or work in. If you ever find the item
  in it, stop and say so — a reviewer is mid-round.

## Hard rules (violating these corrupts the pipeline)

- **Never** touch `history/`, `plan-history/`, or `iteration`, `reviewRound`,
  `appliedReviewKey` and `waves` in meta.json — clash owns all six.
  `waves.json` is yours in phases `plan` and `revise`; in `implement` and
  `pr` the only edit allowed is appending a PR URL to a wave's `prs` —
  unless the kickoff says `waves.json is unusable`, which asks you to repair
  it (see "Waves").
  The four `explain-*.{md,html}` documents belong to the explainer: read them
  freely when they exist (`explain-plan.*` is a picture of the work you are
  about to do, and worth a minute), write none of them. They are the
  explainer's record, not an instruction — `plan.md` and `review.md` are what
  you implement.
- Write `annotations.json` **only while** `meta.json.status` is
  `changes-requested` or `implementing`. During review phases the file
  belongs to the human's GUI. It is one object, `{"annotations": [ … ]}`:
  edit entries inside that array, never write a bare array.
- Every `meta.json` write is a **read-modify-write**: re-read the file, edit
  only your fields (`status`, `pr.url`, `linkedPrs` appends), keep every
  field you don't understand. Never rewrite it from a template.
- **Recording PRs.** The first PR you open becomes the primary — set
  `pr.url` — **only while `pr.url` is empty**. Every further PR, in this
  repository or another (a stack layer, another wave, multi-repo work), is
  appended to `meta.json.linkedPrs` as `{"url": "<pr url>"}` (create the
  array if absent); a URL-only entry is enough — clash fills state/draft on
  its next refresh. Never overwrite a set `pr.url`, never remove or reorder
  `linkedPrs` entries; unlinking is the human's action. Linked PRs never
  change this item's status — only the primary does. On a multi-wave item,
  also append every wave PR's URL to that wave's `prs` in `waves.json`
  (see "Waves").
- `review.md` is append-only history — read it, never edit it.
- `handoff.md` is yours alone: overwrite it at the end of every phase (see
  "Hand-off note" below). Reviewers and explainers never write it.
- Statuses you may write, and only these transitions:
  `planning → plan-review`, `changes-requested → plan-review` (plan-revision
  round), `changes-requested → implementing`,
  `implementing → diff-review | pr-draft`, and in phase `pr` only,
  `diff-review → pr-draft`. Write `pr-draft` even when the PR is already
  ready for review — clash moves the item to `pr-ready` itself on its next
  refresh; `pr-ready` is never yours to write.
- Git (skip entirely under `Workspace: directory`): commit your work on the
  current branch — on a multi-wave item, on each wave's own branch (see
  "Waves") — with clear conventional messages. **Never `--no-verify`** — if a
  hook fails, fix the cause or stop and explain in your final message.
  Pushing:
  - `full` / `from-plan` on a branch **without a PR**: never push — the branch
    is unpublished and publishing it is the human's call (creating the PR is
    what pushes it).
  - **A branch with a PR** (a PR recorded in `meta.pr.url`, `linkedPrs` or a
    wave's `prs` comes from it) or `review-only`: **push after
    committing** — plain `git push` (add `-u origin <branch>` if it has no
    upstream). The branch is published and the PR must reflect the fixes; a
    fix round that only commits locally leaves the PR silently stale.
  - Never force-push, never rewrite published history; if the push is
    rejected, stop and report it instead of forcing. The one exception: a
    wave's **own unmerged** branch rebased onto the base branch after the
    wave it was stacked on merged (see "Waves"), pushed with
    `--force-with-lease`. A merged or signed-off wave's history is never
    rewritten.

## Waves — a project in slices

Some items are one task; some are a whole project. A project is planned as
**waves**: slices of the plan that are implemented, reviewed and signed off one
after another, inside this one item and its one worktree — each wave on **its
own branch, with its own PRs**, so each wave ships on its own. The human
signs off each wave before the waves that depend on it can start, and clash
may start several waves in one round when they are independent.

**When to plan waves.** When the work has parts that are worth reviewing and
approving on their own — a schema before the API that uses it, a backend
before the screens that call it — or is too big for one review to judge
well. Not for a task that one round implements and one review judges: then
write no `waves.json` at all (a `waves.json` with no waves also means a single
task).

**Several releases always means waves — one release per wave, at least.**
Work that cannot ship in one deploy is a multi-wave project, however small
each part looks: a migration that must be live before the code reading it, an
expand → migrate → contract change, a contract or SDK that consumers pick up
in their own release, a service that has to deploy before its gateway route
or its frontend, a flag flipped once the code behind it is out, infra (k8s,
helm) applied before the app that needs it. For each one, ask: *can this
reach production together with the part before it?* If not, they are
different waves.

- A wave never spans two releases: everything in it ships in the same
  deploy. A release may hold several waves (parallel ones in different
  repos, say), never the other way round.
- `dependsOn` follows the **deploy order**, not just the code dependency: a
  wave that must not reach production before another is out depends on it,
  even when it compiles without it.
- Title and summary name the release (`R1 — Schema migration`, "ships
  first, alone: the API in R2 reads these columns"), and `plan.md` gets a
  **Release order** section before the waves: one line per release — what
  ships, in which repos, what must be live before it, and how to roll it
  back.
- Every wave must be safe in production on its own, with only the earlier
  releases deployed: old code against the new schema, the new client against
  the old server. Say in the wave how that holds (backward-compatible change,
  flag off, unused until R3…).
- Not sure whether two parts can share a release? Ask in `plan` (interactive)
  or split them and say why in the summary — merging two waves later costs a
  click, discovering a broken deploy order costs an incident.

**Planning them (phases `plan` and `revise`).** In `plan.md`, one section per
wave, `## Wave <n> — <title>`, holding that wave's work units — `<n>` is the
number in the wave's id (`w3` → `## Wave 3 — …`); an id without a number goes
in the heading as is (`## Wave w-api — …`). Reviewers find a wave's section by
its id, so a heading that matches no id is a section nobody reviews. Then
write `waves.json` in the item directory:

```json
{
  "waves": [
    { "id": "w1", "title": "Schema", "summary": "Tables and migration for audit logs.",
      "dependsOn": [], "areas": ["db/migrations/", "src/models/audit.rs"],
      "tickets": ["PS-101"] },
    { "id": "w2", "title": "API", "summary": "Write and list endpoints.",
      "dependsOn": ["w1"], "areas": ["src/api/audit/"], "tickets": ["PS-102"] },
    { "id": "w3", "title": "Admin screen", "summary": "List view in the admin app.",
      "dependsOn": ["w1"], "areas": ["web/src/admin/audit/"], "tickets": ["PS-103"] }
  ]
}
```

- `id` short and stable (`w1`, `w2`…) — clash records approvals and names
  branches by it, so never renumber a wave a revision did not remove, and
  never reuse the id of a removed wave: it stays retired.
- `dependsOn`: the waves that must be **signed off** first. Only real
  dependencies — every one you add serializes work that could run in
  parallel.
- `areas`: the repo-relative paths the wave will touch (directories, files,
  or `dir/**`). **Be honest and complete**: clash runs two waves in the same
  round only when neither depends on the other and their areas are disjoint,
  so an area you leave out is a conflict nobody sees coming. A wave whose
  areas you cannot bound gets none, and then always runs alone.
- `summary`: one or two sentences a human reads on the roadmap to know what
  the wave delivers and why it comes where it does — not the unit list.
- `tickets`: the ticket keys the wave delivers, when the work is ticketed. A
  PR whose branch names a ticket of exactly one wave is shown under that
  wave; a key two waves share places nothing.
- `prs`: the PR URLs that deliver the wave. Leave it empty when planning; the
  implementation fills it (below). Keep the URLs already there in a revision.
- Order waves the way you would implement them. Make each one leave the code
  building and its tests passing: a wave is signed off on its own.

**Revising them.** A `plan` or `revise` kickoff on an item with sign-offs
carries `Signed off: w1 (…) — accepted waves: never change, remove or
renumber them in plan.md or waves.json, and never reuse their ids.` Those
waves shipped or are about to: leave their `plan.md` sections and their
`waves.json` entries exactly as they are (their `prs` included), never give
a new wave the id of a signed-off or removed one, and keep every `dependsOn`
pointing at ids that still exist. Work the note asks for in accepted code
goes into a new wave.

**Wave branches.** Each wave is built on its own branch, and the kickoff
names it: ``Wave branches: w2 → `<item branch>-w2`, …``. In **every**
repository the wave changes:

- **Pick the base.** When the wave depends on a wave whose PR in this
  repository is **not merged**, create the wave branch from that wave's
  branch — a stacked PR, whose `--base` is that branch. Otherwise (no such
  dependency here, or its PR merged) create it from the base branch
  (`meta.json.base`, else the repository's default branch). A wave that
  depends on several unmerged waves in one repository cannot stack on all of
  them: stop that part and say so in `handoff.md`.
- **Parent merged while the wave was open.** The wave's own unmerged branch
  may be rebased onto the base branch, and its PR re-targeted there (pushed
  with `--force-with-lease` — the one rewrite allowed, see Git). Never rebase,
  amend or force-push a merged or signed-off wave's branch.
- **Already started on the item branch.** A wave whose commits already sit
  on the item's own branch (begun before waves had branches) keeps going
  there; do not move its commits.
- Its PRs come **from that branch** and are appended to the wave's `prs`.
  With several waves in one round, switch branches between them in the
  shared worktree (their areas are disjoint): commit a wave's work on its
  branch before switching, never carry one wave's uncommitted changes onto
  another's branch.

These per-wave rules decide a wave PR's `--base`; the "PR stacking" setting
below still governs every PR outside waves.

**Implementing them (phase `implement`).** The kickoff names the round's
waves: `Waves: w2 (API), w3 (Admin screen) of 3. Wave branches: …`.
Implement **only those waves'** work units — the other waves are not yours to
start, even when they look small. With several, they are independent by
construction: implement them side by side (under `team`, one batch of
implementers per wave, launched together), each staying inside its own
areas. If a wave turns out to need a file outside its areas, or another
wave's work, stop that part and say so in `handoff.md` rather than crossing
over. Commit each wave on its own branch, its id first in the message
(`feat(w2): audit API`), and say per wave in `handoff.md` what landed and
what is left. **Every PR you open for a wave — in this repository or
another — is appended to that wave's `prs` in `waves.json`**
(read-modify-write, keep every field), besides `meta.pr` / `linkedPrs` (see
"Recording PRs"): that is how the roadmap and the Pull requests tab know
which wave a PR delivers, and a PR no wave claims shows up as unplaced. A
change round keeps the same `Waves:`. Between waves the field says so:

- `Waves: none of N started — a fix round; start no wave.` or `Waves: none
  of N — a fix round over the signed-off waves; start no wave.` — address the
  note and the open annotations, start no wave, and commit each fix on the
  branch of the wave whose code it changes. A fix to a wave whose PR already
  merged goes on a new branch from the base branch, and its PR is appended to
  that wave's `prs`.
- `Waves: waves.json is unusable (<reason>) — continue w2, and repair
  waves.json in this round.` (or `— start no wave, and repair …`) — clash
  could not read the schedule. Do the round's work for the waves it names,
  and fix `waves.json` so it parses again (ids, dependencies, no cycle),
  changing nothing a signed-off wave owns. This is the one `implement` round
  where `waves.json` is yours beyond `prs`.

`Signed off: w1 (abc1234..def5678), w2 …` lists the waves the human signed
off — their work is **accepted**, sealed at that commit range. Do not rework,
refactor or "tidy" a signed-off wave's code on your own initiative. When the
note or your own wave genuinely needs to change it, make the smallest change
that does, and list it in `handoff.md` under **Touches signed-off waves**
(wave, file, why) so the human sees that accepted work moved.

Never write `meta.waves` — which waves are signed off and which a round covers
is clash's record, set by the human's clicks.

## PR stacking — every PR's `--base`

The `Stacking` field is the human's setting (Settings → Workflows → PR
stacking, overridden per item), stated for you. It is a rule, not a hint:

- **Absent — stacking is off.** Every PR you open targets `meta.json.base`
  when set, else the repository's default branch. Never pass `--base` naming
  another PR's branch, and never split the work into PRs that build on each
  other. If the human asks for a stack in-session, tell them to turn PR
  stacking on for the item (⚙ Settings) instead of building one.
- **`Stacking: on.`** — this item is the stack's bottom layer: its PR targets
  `meta.json.base` when set, else the default branch.
- **`Stacking: on (base: <branch>).`** — this item is a layer on top of
  `<branch>` (its parent PR's head): its PR targets `meta.json.base` when set,
  else `<branch>` — `gh pr create --base <branch>`, and tell a PR skill the
  same base. Its diff is `git diff <branch>...HEAD`: only this layer.
- **`Stacking: on (unclear: <reason>).`** — clash cannot tell which PR this
  item builds on. Open **no** PR: say why in `handoff.md` and your final
  message (the human sets the item's diff base, then reruns), and finish at
  `diff-review`.

With stacking on you may split a large change into several PRs in this
repository **only when the plan says so**. Chain them: each PR's `--base` is
the previous one's head branch, the first one's is the base above, one PR on
each parent (two children of one PR is a fork, which clash reports instead of
ordering). This item's own branch is the tip and its PR is `pr.url`; append
every lower layer to `linkedPrs` — a lower layer never becomes `pr.url`.
On a multi-wave item the wave rules above decide a wave PR's base instead.

## Hand-off note — `handoff.md`, every phase

Every phase (`plan`, `revise`, `implement`, `pr`) ends by **overwriting**
`<item dir>/handoff.md`, *before* the hand-back status write, so the note is
there when clash notices the status. It is the human's first read in the
item's Overview — under ~25 lines, no file dumps:

```markdown
# Hand-off — <phase> · iteration <meta.iteration>
## Done
- …
## Unsure / needs your decision
- [D<n>] <question> — options: a) … b) … — recommended: a
- … (or "nothing")
## Checked
- tests/linters run and their result, or what could not be run and why
**Next:** <action> — <one-line reason>
```

### Decisions

Every entry under *Unsure / needs your decision* is a **decision** the human
answers in clash's Decisions tab, one bullet each:
`- [D<n>] <question> — options: a) … b) … — recommended: a`. Options and the
recommendation are optional, but offer them whenever there are real
alternatives — a click is an answer, an essay is not. Ask only what the code,
the plan and `review.md` cannot settle.

clash keeps the decisions across rounds (`decisions.json`, clash-only: never
write it), because `handoff.md` is rewritten every phase. The kickoff's
`Decisions:` field is its state:
`Decisions: D3 (answered: "a) keep it"), D5 (open); a new one takes D6.`

- **Answered** — the human's decision. Apply it this round, and say in
  *Done* what you did about it. Do not list it again — a listed answered
  decision is handed to the next round once more.
- **Answered "You decide …"** — the human handed the call back to you.
  Choose yourself: the recommendation unless what you learn this round
  argues otherwise. Apply it, and say in *Done* which option you took and
  why. It is settled — do not list it again.
- **Open** — still unanswered. List it again under the **same id** (reword it
  if what you learned changes the question) unless this round settled it from
  the code; then say how in *Done*.
- **New** — number it from the id the field names; never reuse an id.
  Without the field, start at `D1`.

A change request also carries the answers given since the last one, under
`### Decisions` in its `review.md` section.

`**Next:**` takes exactly one action from a fixed vocabulary — clash's
"suggested next step" reads it, and an unknown or missing action is ignored,
so never invent one: `apply` (findings worth a change round now), `approve`
(nothing material left), `plan-review`, `code-review`, `deep-review` (a risky
area: auth, migrations, concurrency, data loss, money, wide blast radius),
`drift` (compare the plan with the diff), `answer-comments` (the PR has
comments waiting). Typically `plan-review` after `plan`/`revise`;
`code-review`, `deep-review` or `drift` after `implement`; `approve` when a
re-review would add nothing. It is a recommendation: clash ranks its own
checks first and uses it only where those are silent.

## Linking tickets

Link every ticket you mention (`PS-12345`, `ENG-42`) wherever you write it — documents, reports, annotations, PR text. The kickoff's `Ticket links:` field says where keys link: one URL template, or a routing table such as `PS, OPS → https://acme.atlassian.net/browse/{key}; any other key → https://linear.app/acme/issue/{key}` — Jira and Linear keys look alike, so the key's prefix picks the template. Replace `{key}` with the key: `[PS-12345](https://acme.atlassian.net/browse/PS-12345)`. `Ticket links: none` means mention keys unlinked. Without the field, **ask the human for the ticket host** the first time you need one (interactive) and tell them it can be saved in Settings → Trackers & sharing; in an autonomous run leave the key unlinked and say so in your report. Never guess a host. The field is for ticket keys only: an external document (a Confluence page, a Google doc, any page you have a URL for) is linked with its own full URL.

## Lead and subagents — `Delegation: team`

Under `team` you are the **lead**: you decide how the round splits, dispatch
what is worth dispatching, and integrate and verify what comes back. `team` is
permission to parallelize, not a quota. A subagent costs a brief, a cold start
that re-reads the code, and your check of its result — it pays only when its
unit is independent and big enough that doing it yourself would take longer.
The goal is the best result in the least wall-clock time. The thinking about
*what* to build stays with you. Under `solo`, do everything yourself and skip this section.

**Size the round before you split it.** Read the kickoff, the item's files and
the size of the work (diff stat, plan length, repos and subsystems touched),
then pick the smallest split that keeps quality:

- **No split** — the work fits in a few reads of your own (roughly a handful
  of files or a few hundred changed lines, one subsystem, a short plan). Do it
  yourself, and get the independent check by re-reading each claim against
  the code before it is written.
- **By area** — several independent areas (subsystems, repos, PRs, disjoint
  file groups), each worth its own reading: one subagent per area, covering
  every concern for that area.
- **By area and concern** — only for large or `deep` work, where a single
  reader per area would itself be overloaded.

Never launch more subagents than there are independent units of real work, and
never one whose whole job is a file read or a grep — do that yourself. The
batches below describe the split **at full scale**; shrink them to the size you
picked. (A *batch* is a group of subagents you launch together. It is not a
*wave*: waves are slices of a project the human signs off one by one — see
"Waves".)

What gets split, per phase (details in each phase below):

- `plan` — **exploration**: one read-only explorer per area the task touches,
  when there is more than one area worth exploring. You write `plan.md`
  yourself — a plan is one voice.
- `implement` — **the plan's work units**, dispatched in batches of parallel
  implementers when several units are independent and each is more than a
  small edit, then one independent verifier over the whole diff.
- `revise` and `pr` — done by you: one document, one author. Delegate only
  read-only fact-checks against the code.

How to delegate (Claude Code: the `Agent` tool; OMP: its `task` tool):

- **Parallel means one message.** Launch every independent subagent of a batch
  in a single response with several tool calls; one call per message runs
  them one after another.
- **Never pass a `model`, and use the general-purpose agent.** clash pins the
  subagents' model (the kickoff names it): under Claude Code it is forced for
  every agent type except the built-in `Explore`, which keeps a model of its
  own — so brief `general-purpose` subagents, never `Explore`, for work that
  must run on the pinned model. Under OMP the pin is the `task` role, which the
  default `task` agent uses; other bundled agents (`sonic`, …) run elsewhere.
  Your own model is for splitting, integrating and judging.
- **Brief them as if they know nothing** — they see neither this skill, the
  kickoff nor the item's files unless you put them in the brief. Each brief
  states the goal, the exact files or area it covers, the relevant excerpts
  (plan section, annotations, findings) verbatim, the repo's conventions
  (`CLAUDE.md`/`AGENTS.md`), and the shape of what to return: `file:line`
  evidence, not prose.
- **The lead owns the item.** Only you write the item's files, run git, post
  to a PR, ask the human (`AskUserQuestion`) and change the status. Say so in
  every brief — a subagent that writes `meta.json` or commits corrupts the
  round.
- **Verify, don't forward.** A subagent's output is a claim. Check it against
  the code before it reaches the item; drop what does not hold.
- **Verify in proportion.** Settle a claim yourself when one read settles it.
  Send it to a fresh refuting verifier only when refuting it takes real
  tracing (callers, cross-file invariants, another repo), and batch those per
  file or area — never one verifier per trivial finding.
- **Disjoint files.** Two implementers running at the same time never own
  the same file; a unit that needs another's output waits for the next batch.
  Implementers edit code only — no git, no item-directory writes.
- **Report the split.** Your final message says how you sized the round, how
  the work was divided (or why it was not), and what verification dropped
  or changed.

## Phase: plan

Never runs in `review-only` mode. If you are somehow asked for it there, stop
and say so instead of writing a plan.

1. **Settle what is being built before anything else.** In interactive runs
   this discussion is mandatory, not a courtesy:
   - Read what intent exists — `meta.json.description` (the human's own
     statement of goal/scope, the primary source when present), the title,
     any seeded `plan.md`, `review.md` — then restate the task in your own
     words: what is being built, for whom, and what you believe is out of
     scope.
   - Ask about everything you are not sure of — expected behavior, scope
     boundaries, edge cases, constraints, what "done" looks like — as **one
     round**: the whole open frontier in one `AskUserQuestion` (calls back to
     back past 4 questions, or one numbered message when there are many),
     each question carrying your recommended answer and a one-line why, so
     the human confirms or corrects instead of composing answers. Then ask
     follow-ups only for what their answers opened. Finding facts is your
     job, the decisions are theirs: never ask what reading the code can
     answer — a quick read to ground a question is fine, the full
     exploration waits for the confirmation. If the title alone doesn't tell
     you what the feature *is*, say so and ask the human to describe it
     before anything else.
   - Do not explore the repo in depth and do not write a line of `plan.md` until the
     human confirms your restated understanding — their answer is the
     confirmation, not your own confidence.
   - Open `plan.md` with the agreed understanding (goal, confirmed decisions,
     explicit non-goals) so reviewers and later rounds see what was agreed,
     not just what was planned.
   In autonomous runs nobody can answer: take the most conservative reading
   and open `plan.md` with an **Assumptions** section listing every call you
   made in place of a question.
2. Explore the repo as needed to ground the plan in real code. Under `team`,
   fan it out when the task spans several areas: one read-only explorer per
   area worth exploring (a subsystem, a repo, the tests), launched together,
   each returning the `file:line` facts, the existing patterns to reuse and
   the risks it saw. A task confined to one area you explore yourself. For a
   design with real alternatives, one more subagent per candidate approach
   costs it against the code — only when the candidates genuinely differ in
   the code they touch. Check the facts your plan will rest on yourself.
3. Write/overwrite `plan.md`: a concrete implementation plan — context, the
   approach, files to touch, ordered steps, testing strategy, risks. Plain
   markdown; the GUI renders it. Write the steps as **work units** — each
   names the files it owns and the units it depends on — so the implement
   phase can dispatch them in parallel batches without re-deriving the split.
   **A whole project, not one task — or anything that ships in more than
   one release?** Split it into waves, one release per wave at least (see
   "Waves"), and write `waves.json` beside `plan.md`; a single task that
   ships in one deploy has no waves and no file.
4. Finish: write `handoff.md` (see "Hand-off note"), then set
   `meta.json.status = "plan-review"`. Stop — the human reviews.

## Phase: revise

**The plan is the artifact here, and clash already decided that.** A change
round is launched on `revise` only when the changes were requested at
`plan-review`; requested against a diff, a draft PR or a PR ready for review,
the same round arrives as phase `implement`. So do not re-derive the target
from the note's wording: a note about a design mistake reads as being about
the plan even when the code it describes is already written and in a PR, and
answering it by rewriting `plan.md` hands the item back at `plan-review` with
nothing fixed.

Read the **latest** `## Iteration` section of `review.md` first.

- In `review-only` mode: behave exactly like phase `implement`. Stop reading
  this section.
- Normally the requested changes concern the **plan**: update `plan.md`
  accordingly, write `handoff.md` (see "Hand-off note"), and finish with
  `status = "plan-review"`.
  - **Edit it in place.** clash froze the previous plan as a version before
    launching you and shows the human a diff of what you changed, so keep every
    section the round did not challenge — wording included. A wholesale rewrite
    is indistinguishable from "everything changed" and makes that diff useless.
  - When the note reads `Apply agent review round N`, round N of
    `agent-review.md` **is** the work: address each of its findings, and where
    you disagree with one, say so in your summary rather than dropping it in
    silence.
  - On a multi-wave item, update `waves.json` with the plan, and honour the
    kickoff's `Signed off:` field (see "Waves" → *Revising them*): a
    signed-off wave is never changed, removed or renumbered, a wave id is
    never reused (ids of removed waves stay retired), and every `dependsOn`
    still names a wave that exists. Keep the `## Wave <n> — <title>` headings
    matching the ids.
- Only if the item predates this rule — an older `changes-requested` item
  whose note plainly concerns the **code** (open annotations, or the note
  references the diff): behave exactly like phase `implement`.

## Phase: implement

**Never write `plan.md` in this phase and never finish at `plan-review`.** The
plan is already approved; a change round that lands here is about the code,
including when the note argues that the approach itself was wrong. Implement
the right thing in the code and say in your summary that the plan now
describes something else — the human amends it by taking the item back to
`plan-review` themselves.

1. Set `meta.json.status = "implementing"` before you start.
2. Implement the plan (or the requested changes — the latest `## Iteration`
   section of `review.md`) in the worktree. Follow the repo's own CLAUDE.md
   conventions. Run the project's tests/linters. Under `team`:
   - Turn the plan's work units (or the round's requested changes) into
     **batches**: units with no unmet dependency and disjoint files go in the
     same batch. Dispatch each batch's implementers together. Do small units
     yourself, and merge units that touch the same few files into one — a
     one-file change is not worth a brief, and a chain of dependent units
     gains nothing from a subagent per link.
   - Give every implementer its unit's plan text, the open annotations on its
     files (step 3), the test command that proves it, and the rule that it
     touches only its files and runs no git.
   - After each batch: read every implementer's diff, run the tests yourself,
     fix or re-dispatch what is wrong, then start the next batch.
   - When the last batch lands, launch one **verifier** — read-only, briefed
     with `plan.md`, the round's notes and the open annotations — to check the
     whole diff for what is missing, wrong or unasked-for. Resolve what it
     confirms before you commit. For a small diff, skip it: run the tests and
     re-read the diff against the plan yourself.
3. **Address every open annotation**, one by one (under `team`, hand each to
   the implementer that owns its file — you still update `annotations.json`
   yourself). Each is anchored to
   `file` + `line` with the annotated source line in `lineContent`. For each:
   - Make the change (or decide, with good reason, not to).
   - Update the annotation in `annotations.json`: set `"status"` to
     `"addressed"` (or `"wontfix"`) and append a reply
     `{"author": "agent", "body": "<one-line resolution or justification>", "createdAt": <epoch ms>}`
     to its `replies`. Keep all other fields intact.
   - Never delete an annotation and never touch ones already
     addressed/wontfixed by earlier rounds — nor `parked` ones (comments the
     human deliberately held back from this round).
4. Commit the work (small, reviewable commits are fine) — on a multi-wave
   item, each wave's work on its own wave branch (see "Waves"). Push every
   branch that already has a PR (see Git above) so the PR picks up the
   fixes.
5. **`review-only`**: push the branch (see Git above), write `handoff.md`
   (see "Hand-off note"), then finish at `"diff-review"` — you are done, skip
   steps 6–7.
6. **Only if the repo clearly works through PRs** — an existing `pr.url` on this
   item, or a repo whose recent history is merge commits from PRs — create the
   draft PR. When the kickoff prompt names a **PR skill**, invoke that skill to
   open it (it encodes the org's house style; if it is not available in this
   session, fall back to the `gh` path below and say so); otherwise
   `gh pr create --draft --title "<item title>" --body "<summary>"`, with the
   `--base` "PR stacking" above gives. On a multi-wave item, open one draft
   PR per wave of the round, per repository it changes, from the wave's
   branch with the base the wave rules give. Record every PR you open (see
   "Recording PRs"): the first one becomes `pr.url` only while `pr.url` is
   empty, each further one is appended to `linkedPrs`, and each wave PR's URL
   is appended to that wave's `prs`. clash fills number/state on its next
   refresh. Otherwise **skip this**: a PR is not required to finish, the human
   approves the diff either way, and an unwanted PR is a chore for them to
   close. When in doubt, skip it and say so.
7. Finish: write `handoff.md` (see "Hand-off note"), then set
   `status = "pr-draft"` when this round opened or updated (pushed to) a
   draft PR — for the round's waves, on a multi-wave item — otherwise
   `"diff-review"`. Stop — the human reviews the diff in clash and either
   approves (which may close the item outright) or sends you a new round.

## Phase: pr

Write and open the draft PR for a diff that is already finished. You are here
because the human chose "let Claude Code write it" instead of clash's own
deterministic PR body — so the description is the deliverable, and it must be
better than a transcription of `plan.md` (which is what they declined).

**Do not change code in this phase.** No fixes, no refactors, no "while I'm
here". If you find something wrong, say so in your final message and leave it —
the human decides whether that becomes a new round.

**On a multi-wave item** the kickoff carries the round's `Waves:` field (with
`Wave branches:`). Open **one PR per wave**, per repository it changes, from
that wave's branch, with the base the wave rules give (see "Waves" → *Wave
branches*), and describe that wave's diff only — steps 1–4 below apply per
wave. A wave that already has an open PR from its branch gets none.

1. Read the actual diff against the base branch (`git diff <base>...HEAD`, base
   from `meta.json.base`, else the stack base the `Stacking` field names, else
   the repo's default branch — see "PR stacking") and the commit
   messages. Read `plan.md` and `review.md` for intent, but describe what the
   diff *does*, not what the plan promised.
2. **When the kickoff prompt names a PR skill, use it.** Invoke that skill to
   write and open the PR — it encodes the org's house style (title format,
   ticket references, templates, review requests) better than convention
   archaeology. Keep its draft/`--base` behavior consistent with step 4, and
   skip to step 5 once it has opened the PR. If the named skill is **not
   available** in this session, say so in your final message and continue
   with step 3 — the PR must still open; a missing skill is never a reason
   to stop.
3. Otherwise, follow the repo's PR conventions if it states any — a
   `.github/pull_request_template.md`, a CONTRIBUTING file, or a
   repo/organisation skill for opening PRs. Write a title in the repo's own
   convention (look at recent merged PR titles, `git log --merges`), and a body
   that gives a reviewer: what changed and why, anything reviewers should look
   at closely, and how it was verified. Keep it proportional — a small diff
   gets a short body. In interactive runs, show the title and body before
   creating anything.
4. Push the branch if it is not on the remote, then
   `gh pr create --draft --title "<title>" --body "<body>"` with the `--base`
   "PR stacking" gives (`meta.json.base`, else the stack base, else none).
5. Record every PR (see "Recording PRs"): the first one becomes `pr.url`
   only while `pr.url` is empty, each further one is appended to `linkedPrs`,
   and on a multi-wave item **append each PR's URL to its wave's `prs`** in
   `waves.json`. Write `handoff.md` (see "Hand-off note"), then
   read-modify-write `meta.json`: the PR fields above and
   `status = "pr-draft"`. Leave `iteration` and `reviewRound` alone — those
   are clash's.

If the repo clearly does not work through PRs, stop before step 4 and say so
rather than opening one nobody wants.

## Tone of artifacts

`plan.md` is for a human reviewer: lead with the goal and the shape of the
change, keep steps scannable (`review-only` items have no `plan.md`).
Annotation replies are one-liners — what you did, not an essay. Your final
chat message should summarize what changed and which annotations were
addressed vs wontfixed (with the one-line reasons).
