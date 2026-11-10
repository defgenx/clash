# Workflows — user guide

A pipeline manager for AI-assisted development, in the [desktop app](gui.md):
one **item** per piece of work, taken from idea to plan to code to PR by Claude
Code (or OMP) agents, with you approving every step. Everything is plain files,
so an item's whole history stays readable outside clash. Workflows are
GUI-only.

This guide covers what you see and click. The agent-side file contract and the
design behind each rule are in [workflows.md](workflows.md).

- [Starting an item](#starting-an-item)
- [The pipeline](#the-pipeline)
- [The action bar](#the-action-bar)
- [Reading an item](#reading-an-item)
- [Projects in waves](#projects-in-waves)
- [Reviewing the diff yourself](#reviewing-the-diff-yourself)
- [Requesting changes](#requesting-changes)
- [Check runs](#check-runs)
- [Agent review rounds](#agent-review-rounds)
- [Applying a round](#applying-a-round)
- [Explaining](#explaining)
- [Comparing plan vs changes](#comparing-plan-vs-changes)
- [Code review and PR verdicts](#code-review-and-pr-verdicts)
- [Going back](#going-back)
- [Pull requests](#pull-requests)
- [Many pull requests](#many-pull-requests)
- [Multi-repo work](#multi-repo-work)
- [PR stacks](#pr-stacks)
- [Ticket trackers and sharing](#ticket-trackers-and-sharing)
- [Agents, models and skills](#agents-models-and-skills)
- [Storage](#storage)

## Starting an item

The `+` button on the WORKFLOWS section asks how the item starts:

| Mode | Starts at | Use it when |
|---|---|---|
| **Full workflow** | `draft` | you have an idea: an agent plans, you approve, it implements |
| **From a plan I already have** | `plan-review` | the plan exists: paste it, point at a markdown file, or pick a scratch note. No planning agent runs |
| **Review only** | `diff-review` | the code is already written: give a **PR** (URL or number) or a **local branch** and get just the review loop |

Give it a title, a project and a repo (picked from your open sessions and
existing projects, or **Browse…**), and optionally a **description** of the
goal. The planning agent reads the description first, which saves half the
requirements questions.

**Review only** resolves the PR through `gh`, checks its branch out (reusing an
existing worktree of it), and diffs against the PR's own base. A PR URL is
looked up in its own repository, so a link to another repo is refused by name
rather than resolved to whatever local PR shares the number. No plan is written
and no draft-PR stage runs, since the PR isn't clash's.

Planning reads the repo in place. The first round that writes code gets its own
git worktree and branch, unless the directory isn't a git repository or the item
is set to work in place.

## The pipeline

`draft → planning → plan-review → changes-requested → implementing →
diff-review → pr-draft → pr-ready → done`, plus `abandoned` from anywhere and
`reviewing` while an agent review round runs.

- The **PR stages are optional**: approving a diff can close the item outright,
  for repos that merge straight to their default branch.
- A **pipeline stepper** at the top of every item shows the mode's stages, where
  the item is, and how many rounds it has been through.
- Decision stages (plan review, diff review, PR draft) badge the sidebar, fire a
  desktop notification and appear in the
  [attention inbox](gui.md#attention-inbox).
- An item **follows its primary PR**: merged on GitHub → `done`; flipped to
  ready for review on GitHub → `pr-ready` on the next refresh.

## The action bar

Every item has an action bar in three labeled zones — at the bottom in the
Tabs view, as the next-step card in Workbench (where the Item zone is its
**More** menu; on top of the content in a narrow tab, in the right-hand rail
in a wide one):

- **This step · stays here** — **🔍 Check…** and **▾** (every review and
  explanation, see [Check runs](#check-runs)), opening the PR or the session.
- **Continue · moves the item** — the decisions that advance the pipeline.
- **Item · back, park, share** — the item as a whole.

Above the zones, a **Suggested next** strip says what to do now and why, along
with what it already checked:

1. a review round came back and isn't applied yet → **↻ Apply**
2. comments are still open → **✎ Request changes…**
3. PR threads are waiting on you → **⇄ Answer PR threads**
4. this iteration's plan or code has no review yet → **⌕ Plan review** /
   **⌕ Code review**
5. nothing has compared the change with the plan → **⇄ Compare plan vs
   changes**
6. a ready PR has no verdict for this iteration (after its code review) →
   **⌕ Code review** ending in *Post a verdict on the PR*
7. otherwise → **Approve**

Due checks (3–6) are suggested **together**, as chips with one **▶ Run N
checks** button: untick a chip to leave it out, or **Options…** for the full
checklist. Checks that post on GitHub start unticked. Anything else points at a
button that is on the bar and moves the highlight there. While a check run is in
flight, the strip shows its progress instead. The agents advise too, through a
`**Next:**` line at the end of every review round and of the executor's
hand-off, but clash's own checks rank first. "Reviewed" means reviewed **at this
iteration**: after a fix round, the code is due for review again.

`workflows.assist` sets how far it goes (*Settings → Workflows → Next-step
assist*):

- `suggest` (the default) highlights the step.
- `autopilot` also starts the suggested checks after each agent hand-back, as a
  check run allowed to apply what its rounds say is worth applying — only checks
  that post nothing. It stops before every approval, PR flip, change request and
  GitHub post, and after 3 runs on an item until you click something there (the
  count survives a restart).
- `off` gives the plain bar.

## Reading an item

An item tab opens in one of two views (*Settings → Appearance → Workflow item
view*), over the same documents and actions:

- **Workbench** (default) — a grouped outline on the left: **Build** (Plan,
  Diff), **Feedback** (Change requests, Agent reviews, Hand-off), **Explained**
  (Plan explained, Changes explained, Plan vs changes), **History** (Revisions,
  Timeline), with ⚙ Settings at the bottom. Counts and open comments show as
  badges, a document not written yet is dimmed. The main column opens on the
  **next-step card** — the suggested step with its reason, this step's actions
  and the decisions that move the item — with the Item actions (move back,
  share, abandon…) folded into its **More** menu, so "where am I" (the stepper)
  and "what now" sit together above the content.

  When the tab is wide (about 1180px and up), Workbench becomes a cockpit:
  the outline on the left, the document at full height in the middle, and a
  **rail** on the right that stays put whatever you read. It holds the
  next-step card, the last review, and three cards: **Waves** (where the
  project stands, a chip per wave), **Pull requests** (counts, when they were
  last checked, a chip per PR with its state, refresh) and **Agents** (the
  item's sessions, running first — click to open). The header then drops its
  PR and wave chips, which the rail repeats. A card whose tab is open is left
  out of the rail. In a narrow tab (a split pane, a small window) the layout
  is the one above.
- **Tabs** — one row of tabs above the content and the action bar below it.

The grouping is the pure `gui/dist/wf-outline.js` (tested); both views are
built from one sub-view list, so they can never disagree about what an item
has. The **?** in an item's header walks through its tab in either view (it
also runs by itself the first time you open an item).

| Tab | Holds |
|---|---|
| **Plan** | the current `plan.md`, rendered; *Edit plan.md* opens it in your editor |
| **◫ Revisions** | every plan version (one per applied review round), any version's full text, and **⇄ Changes** between any two |
| **Diff** | the change, with line comments; its source picker can also show a linked PR's diff (view-only) |
| **Change requests** | *your* notes, one section per round — what the next agent round reads first |
| **↩ Hand-off** | the workflow agent's note from its last phase: what it did, what it is unsure about, what it tested, what it suggests next |
| **Agent reviews (n)** | every review round's verdict and findings, with jump chips, opening on the latest |
| **◫ Plan explained** / **◫ Changes explained** | the explainer's two documents (see [Explaining](#explaining)) |
| **⇄ Plan vs changes** | the latest plan-vs-changes comparison |
| **Timeline** | one newest-first feed of every change round (your note, the plan diff, the plan as it stood, the code diff) and every review round |
| **⚙ Settings** | per-item knobs: title, description, diff base, agent CLI, PR skill, ticket, default interaction mode, work in place, session-name prefix, Jira ticket — plus the item's facts |

Workflow sessions are named by item and job (`Auth refactor · implement`,
`· plan review r2`, `· code review r1`, `· explain plan`, `· plan vs changes r1`,
`· answer PR comments`), so the sessions list says what each agent is doing.

## Projects in waves

An item can be one task or a whole project. When the planner sees a project,
it splits the plan into **waves**: slices you implement, review and approve
one at a time, in order, inside the same item (one branch, one worktree, one
PR). Each wave says which waves it needs first and which parts of the code it
touches. A single task has no waves and works as before.

- **Approve plan → start waves…** asks which waves to start. The first ready
  one is ticked. You can tick another only when it doesn't depend on the
  first and touches different code; the agent then builds them in parallel,
  in one round, and you approve them together.
- When the round comes back, review it as usual (your diff review, a code
  review, plan vs changes). The reviewers look at the round's waves.
- **✓ Approve w2 → next waves…** approves the round's waves and offers the
  ones that became ready. **✓ Approve w2 only** approves without starting
  anything; **▶ Start next waves…** picks up from there later.
- **Request changes** during a wave sends the fix round back to the same
  waves.
- **Approve → done** and **Approve → PR draft** appear once the last waves
  are in review, and approve them on the way. You can open the PR whenever you
  like; it collects the waves as they land.

A project in waves opens on its **Waves** tab, a roadmap:

- At the top, in plain sentences, what is happening and what you do next
  ("w1 · Foundations is waiting for your approval. 2 of 8 PRs merged, 6 still
  open. Next: get its PRs merged, then approve it — that unlocks w2."), a
  track of every wave's state, and the approve/start buttons.
- Then the steps in order. Each wave shows what it delivers, its state, how
  many of its PRs are merged, its tickets (linked to your tracker) and its PRs
  (click to open, right-click for the PR's actions). *Scheduling* folds away
  what it waits for, what it can share a round with and the code it touches.

The tab uses the space it has: in a wide tab the "now" panel and a summary
of the PRs stay on the left while the steps fill the rest, with a step's waves
side by side; in a narrow one (a split pane, a small window) it is a single
column.

A PR belongs to a wave when the wave lists it (the agent adds each PR it
opens) or when its branch names a ticket only that wave lists. The header
shows `Waves 2/5 · w3 in progress`, and the suggested next step points at
the wave buttons. A
`waves.json` that can't be used (a wave that depends on itself, a loop, an
unknown wave) shows the reason and blocks starting: request changes on the
plan so the planner fixes it.

## Reviewing the diff yourself

Hover any diff line and press `+` to leave a GitHub-style comment. Threads
support reply, edit, resolve, *won't fix* and *park* (kept, but not sent with
the next round). The ↑/↓ buttons in the diff header step through open comments,
expanding collapsed files on the way. Comments re-anchor by content when the
diff moves between iterations, and are never dropped: one that can't be placed
lands in an orphan tray.

## Requesting changes

**✎ Request changes…** opens a composer. The note is appended verbatim to
`review.md` and is the first thing the next round reads, so it is effectively
that round's prompt. The composer offers:

- a **What to change / Why / Out of scope** template;
- *Insert review findings…* from any review round;
- a preview of exactly what will be recorded;
- the queued comments as rows you can untick (park), jump to or delete;
- **After recording**: record only, or launch the fix round now, choosing how it
  runs and, optionally, a different executor skill.

⌘↵ sends, and a dismissed composer keeps its draft.

Recording a round freezes the iteration first (diff, plan and comments under
`history/`), so every round has a before and after. Where the request lands
decides what the agent may touch:

- At **plan review**, the round revises the plan.
- Everywhere else, it changes the **code** and never rewrites `plan.md`. To
  amend the plan, use **↩ Move back to… → plan-review** first.

When the item has a PR, the fix round pushes so the PR follows.

## Check runs

Every review and explanation starts from one place on the action bar:

- **🔍 Check…** opens a checklist of every check the stage allows (plan or code
  review, plan vs changes, explain plan, explain changes, answer PR comments),
  the suggested ones ticked. Each row has its own options (depth, how a code
  review ends — findings kept here, posted as comments, or a verdict on the PR —
  focus, which PRs) and a **Run only this →** link; at the
  bottom, which agent, how the rounds run, and whether to apply the findings
  when the run finishes.
- **▾** runs one check on its own, through that check's own dialog.

A pick runs as one **check run**:

- explanations start at once, alongside everything else;
- the other checks run one after another over the same code, in a fixed order
  (a verdict last);
- then at most **one** change round for every round worth applying, instead of
  one fix round per check — automatically if you allowed it, otherwise through
  **↻ Apply** on the strip (apply now, or edit the note first).

While it runs, the strip shows `Checks · 1 of 2 done · code review running ·
next: plan vs changes`, with **Skip next**, **Stop after this one**, **Retry** /
**Skip** for a failed check, and **✕** to close the run. The run is saved in the
item's `run.json`, so quitting clash or an update restart doesn't lose it: it
picks up where it stopped on the next launch.

## Agent review rounds

**⌕ Plan review** (at plan review) and **⌕ Code review** (diff review and both
PR stages) — from **🔍 Check…** or **▾** — hand the item to a reviewer agent. A
round is a side trip: the item goes to `reviewing` and comes back exactly where
it started. Rounds are unbounded, and nothing advances until you approve. Rounds
are numbered per target, so the first code review is *Code review 1* however
many plan reviews came before it.

The launch composer shows the round's whole shape before anything spends tokens:

| Choice | Options |
|---|---|
| **Which change?** | multi-repo items only: this repo's own diff and/or any of the item's PRs, in one round |
| **Depth** | code and drift rounds: `deep` (default; traces callers, invariants and existing tests) or `standard` |
| **Findings** | keep local (default) or also post to the PR — one review with line comments, never an approval; on a draft PR the summary goes as a comment, since GitHub refuses reviews on drafts |
| **Interaction** | ask when it starts (default), interactive, or autonomous |
| **Apply when done** | let a round that says "apply" start the fix round by itself |

**Interactive or autonomous is your call.** An interactive round asks its open
questions in one batch, each with a recommended answer. It then walks you
through its findings before writing anything: you keep, drop or regrade each
one, and plan issues come with lettered options. It asks before any trivial fix
or PR post. An autonomous round decides alone and reports at the end. The
executor phases start with the same question.

Code findings arrive as **diff comments** graded `BLOCKER`/`RISK`/`GAP`/`NIT`,
which you triage like your own. Every finding also carries a **confidence**
badge — `high` (verified in the code), `medium` (one link not traced) or `low`
(plausible, unverified) — with the evidence in its tooltip, so you know which
ones to believe and which to dig into first; applying a round tells the agent to
verify the low ones before acting. The verdict and plan findings go to **Agent
reviews** (`agent-review.md`).

- When a round finishes, a toast and a strip on the item say what it concluded
  and what it published.
- **↗ Post round N to PR** shares an existing round later, with no new review.
- A reviewer may fix only trivial mechanical issues, after asking, and never
  rewrites what it reviews.
- While a round runs, approval and the comment editor are locked. **End round**
  always unlocks them, so a crashed reviewer can't wedge an item; a dead
  executor gets **⚠ Relaunch agent**.
- **■ Stop agent** stops any agent running on the item (executor, reviewer or
  explainer) without moving the item and keeps the conversation, so
  **▶ Resume agent** carries on where it left off. Stop a workflow agent this
  way rather than with *Kill session*, which removes the session the item still
  records.

## Applying a round

Every round ends with `**Apply:** yes|no — reason` (are the findings worth a
revision round?) and `**Next:** <action> — reason`. clash shows the call on the
button: **↻ Apply plan review 2 → revise plan · recommended** (or *not needed*).
One click composes the note from the round's findings, records it as the next
change round and launches the agent. *Edit the note first…* opens the composer
pre-filled, for "apply 1a and 3b". Until a round is applied, the item header
says **not applied yet** and the stage's own Approve steps back.

## Explaining

**◫ Explain plan** reads `plan.md` and the code it will land in, and explains
what the implementation is *going* to do before it exists. **◫ Explain changes**
reads the diff and explains what the change *did*. Each run writes two forms:

- a written walk-through with mermaid diagrams;
- one hand-drawn page that shows what will be done and how it works: the goal in
  a sentence, the parts as plain-language boxes laid out in the order things
  happen, the main path numbered ①②③ with a step list, and what is new or
  changed marked on the map. It is rendered sandboxed with scripts off.

Some changes are better seen from another angle, so a level can carry up to two
extra **views** beside the map:

- **flow**: one request or job, step by step across the components;
- **state**: a lifecycle's states and transitions;
- **data**: the schema or payload before and after;
- **before-after**, **rollout** (what must land in which order) and
  **dependency**.

In an interactive round the explainer suggests the views it thinks would help,
and you pick. In an autonomous round it decides, and the tab's tooltip says why.
Asking for one in *Focus* ("draw the request flow") always gets it.

The tab opens on the drawing. **◫ Diagram / views / ☰ Write-up** switches form,
and **⤢** gives the drawing the whole tab. An explainer judges nothing, runs
alongside the item's other agents without blocking it, and can be told what to
focus on.

### Zooming into a part

An explanation is a tree, not one page: the overview lists its parts, and each
part can have a level of its own — what is inside it, then the mechanism, then
the annotated code (three levels at most). The strip above the document is a
breadcrumb back to the overview plus one chip per part:

- **▸** opens a part that is already explained;
- **⊕** explains it one level deeper (an agent round you pay for only when you
  click);
- **●** marks a part the explainer flagged critical, with the reason on hover;
- **⟳** marks a level drawn before the one above it was redrawn.

Clicking a box in the drawing, a chip, or **⊕ about this part** next to a
section of the write-up opens that part. A part nobody has explained yet opens
on a page of what is already known about it:

- its summary and why it matters;
- what it receives from and sends to its neighbours, each one a click away;
- the files it lives in, which open in your editor;
- a link to what the write-up says about it.

From that page, **⊕ Explain this part** goes one level deeper. You can type the
question you want answered first or pick one of the suggestions ("What could
break?", "Draw the flow through it"). While a part is being explained, the page
says so and links to the session. A dive goes into the session that drew the
overview when it is still open, because that session already has the code in
context.

**How much is written up front** is a choice per launch, defaulting to
*Settings → Workflows → Explanation detail* (`workflows.explain_detail`):

- **Critical parts** (`critical`): the overview plus the inside of what it flags
  critical. This is the default.
- **Overview only** (`overview`): the cheapest.
- **Every part** (`full`): costs the most.

### Explaining any PR

**◫ Explain a PR…** (the ◫ button on the WORKFLOWS header, or the PR dashboard)
explains anyone's pull request. Paste the URL, and clash finds your local
checkout of that repository, opens a review-only item for the PR and starts the
explanation. A code review of the same PR, or a verdict on it, is then one
click away.

## Comparing plan vs changes

**⇄ Compare plan vs changes** reads `plan.md` and the diff together, to answer
the question neither review can: did we build what we agreed to? Every
divergence has:

- a direction: **missing**, **extra** or **different**;
- a grade on consequence, not size: **intended**, **benign** or an **issue**.

A test or migration the plan promised and the change skipped is an issue. Issues
arrive as diff comments, so ↻ Apply or Request changes turns them into a fix
round. Drift resolved by amending the plan is reported instead, because a fix
round never rewrites `plan.md`: move the item back to plan review for that.
Launching it offers to refresh any explanation written for an older iteration
alongside it, pre-ticked; it runs in parallel, and the comparison doesn't wait
for it.

## Code review and PR verdicts

**⌕ Code review** has one reviewer, and you choose how the round ends:

- **Keep the findings local** — they land as diff comments on the item and in
  *Agent reviews*. Nothing leaves your machine.
- **Also post them to the PR** — one review of line comments. It never approves
  or requests changes.
- **Post a verdict on the PR** — the round reviews the PR like a human reviewer
  and posts **approve** or **request changes**, as one GitHub review with line
  comments on the code concerned and a summary. Offered once an open PR exists.

**Answer PR comments** is its own check: it replies in every open thread and
fixes what the comments ask for.

How hard the round looks is the depth: **standard** is one pass with every
finding verified; **deep** reviews in a loop, pass after pass, each hunting for
what the earlier ones missed, until a full pass finds nothing new. A verdict
always loops. Every round on an item with a PR also reads the PR's discussion,
so it doesn't re-raise what was settled there.

What a verdict round does:

- It reads everything already on the PR (review threads, comments, reviews and
  fix commits) and checks each claimed fix against the code. A thread marked
  resolved whose problem is still there becomes a finding credited to the
  reviewer who raised it, and decisions settled in the discussion are respected.
- It writes like a colleague would: short line comments on the line where the
  fix goes, a question when it is unsure, links to the code it means
  (permalinks, which GitHub shows as snippets) and to every ticket, PR or doc it
  mentions, never "LGTM", and a summary whose shape depends on the case (a
  one-line approval, a single blocker, a list, a question for the author, a
  re-check after fixes).
- Every finding still ends with a small 🤖 footer giving its grade and
  confidence, a check refuses to post a comment without one, and the summary
  ends with a line saying it is an automated review.
- Before calling something a bug in how another service, repository or library
  behaves, it reads that side at the version the change uses (asking you which
  ones first in an interactive run).
- How hard it looks depends on the size of the change: a small PR gets a lighter
  model and no fan-out, a large one stronger reviewers on every pass.

On an item with several PRs, the composer's *Which change?* rows pick the PRs; a
verdict with none ticked posts on every open one, one verdict each. A PR's own
right-click menu offers *Post a verdict on <PR>…*, which opens the composer with
that PR ticked. Like every review composer, it also asks which agent runs the
round (Claude Code or OMP) unless the agent setting fixes one. Verdict rounds
are numbered among themselves: *PR verdict 2* is the item's second verdict.

It needs a PR, not a plan, so review-only items get it too. GitHub doesn't let a
PR's author approve or request changes on it, and a draft takes no review, so in
those cases the verdict is posted as a comment that states it. The findings also
land as diff comments, so a change request can become a fix round.

The skill behind every code review, `clash-pr-review`, works outside workflows
too: ask Claude Code to "review PR <url>" (or run `/clash-pr-review 123`) on
any PR. On its own it posts a verdict unless you ask for comments only or for
nothing to be posted.

## Going back

**↩ Move back to…** lists the stages behind the current one, with what each is
for. Clicking a passed stage in the stepper does the same. Only the stage moves:
the plan, diff, comments, PR and rounds all stay, and no agent runs. A finished
or abandoned item offers **↩ Reopen at diff review**.

## Pull requests

- **Create draft PR…** offers two ways to write the description: from the plan
  (free, instant) or by the agent from the real diff (spends tokens). A branch
  that was never pushed is pushed first.
- **✓ Mark PR ready** flips the draft once you have validated it. **✓ PR is
  ready → PR ready** records a PR already flipped on GitHub. **✓ Mark done**
  closes the item at either PR stage.
- **⇄ Answer PR threads** launches an agent that reads every review thread,
  fixes the trivial ones with commits, replies in each thread, and queues the
  rest as comments for you. The button shows how many threads are open (a thread
  is settled only when your reply is its last word, so findings clash itself
  posted count until they get a decision) and says *all answered* when nothing
  is.
- **PR skill** (*Settings → Workflows*, default
  `hivebrite-engineering:github-pr`, `none` disables, per-item override in ⚙
  Settings): agent-written PRs go through that skill, so they follow your org's
  titles, templates and ticket links.
- **Ticket links** in PR bodies and agent output: see
  [Ticket trackers and sharing](#ticket-trackers-and-sharing).
- **`workflows.forge`** (`auto` | `github` | `none`): `auto` detects GitHub from
  the origin remote (GitHub Enterprise included). `none` hides every PR feature.
- A command that needs a PR the item doesn't know asks for its URL, attaches it
  and retries. A review round can run locally instead.

## Many pull requests

With more than three PRs, the item's header shows the primary PR and one
summary — `10 PRs · 2 merged · 7 open` — instead of a chip per PR. Click it
for the **Pull requests** tab, which every item with two PRs or more has:

- every PR in one list, grouped by wave when the item has waves, open ones
  first;
- per PR: its state, whether it is the primary, its place in a stack, its
  branch and base, threads waiting for an answer, and the error if the last
  refresh could not reach it;
- **↻ Refresh all** asks GitHub about every PR at once, and the list says
  when it was last checked;
- **Open N open PRs**, and **⋯** on each row for that PR's actions.

In a wide tab the groups sit side by side.

## Multi-repo work

**🔗 Link a PR…** attaches PRs from other repositories. They show as chips in the
item header and refresh with the primary. Only the primary PR moves the item's
status; an item with *only* linked PRs closes when all of them merge. Once an
item has several PRs, every PR action asks which ones (any subset, with an
all/none toggle):

| Action | Pre-ticked |
|---|---|
| **Open PRs (n)…** — the first in a split pane, the rest as browser tabs | all |
| **✓ Mark PR ready…** — drafts only; only the primary moves the item | the primary |
| **↗ Post round N to PR…** | the primary |
| **⌕ Code review** — one round over several diffs; a linked PR's findings are posted to that PR | this repo's own diff |
| **⇄ Answer PR threads…** | every PR with open threads |

Every PR chip also has a right-click menu with that PR's actions. The **⇄ PR
dashboard** (button on the WORKFLOWS section) lists every item holding a PR
across all projects, decisions first and merged last.

## PR stacks

With **PR stacking** on (*Settings → Workflows → PR stacking*,
`workflows.pr_stacking`, off by default, GitHub only; per-item override in ⚙
Settings), an item whose linked PRs build on each other on GitHub — each one's
base branch is another's head — is treated as a stack:

- chips list the PRs base → tip and show each layer (`2/3`);
- code reviews and *Answer PR threads* pre-tick the whole stack;
- the item's diff is taken against its parent PR's branch;
- review rounds are told the order (`Stack:` in the kickoff) and review each
  layer against its parent, checking the other layers before calling anything
  missing. In an interactive round the reviewer first asks whether to review the
  whole stack or which layers. Run on its own, `/clash-pr-review` finds the
  stack itself.

PRs that don't order into one chain (a fork, a loop, a missing middle layer)
show **⚠ stack unclear** with the reason and are treated as unstacked.

## Ticket trackers and sharing

Everything about ticket systems (Jira, Linear) and chat destinations (Slack,
Discord) is in this one section. The settings live in *Settings → Trackers &
sharing*; the config keys are listed in
[config-file.md](config-file.md#settings).

### Trackers and ticket links

clash knows two ticket systems, **Jira** and **Linear**, and can use both at
once (handy mid-migration). Configure them in *Settings → Trackers & sharing*:

- **Jira**: the site (`yourorg.atlassian.net`). Keys link to `…/browse/KEY`.
- **Linear**: the workspace, as its slug (`acme`) or any `linear.app` URL. Keys
  link to `https://linear.app/acme/issue/KEY`.
- **Key prefixes** per tracker (`PS, OPS` for Jira, `ENG` for Linear). Jira and
  Linear keys look alike, so a key belongs to the tracker that lists its prefix.
  Any other key belongs to the **primary tracker** (`trackers.primary`, which
  falls back to the only configured tracker; `none` turns linking off).

Every agent clash launches is told the routing in its kickoff, so every ticket
key it writes in a plan, a review, an explanation or a PR is a link to the
right tracker. With no tracker set, the agent asks you for the host
(interactive rounds) or leaves the key unlinked and says so (autonomous ones).
External documents always keep their own URL. The rule agents follow is in
[workflows.md → Ticket trackers](workflows.md#ticket-trackers--every-kickoff).

### Sharing an item

**↗ Share…** composes one document from the item. You tick what goes in:

- summary;
- where it stands (the agent's latest hand-off note);
- plan;
- plan explained / changes explained (the explanations' write-ups);
- plan vs changes;
- change rounds;
- agent reviews;
- open comments;
- diff.

Six presets fill the boxes: *Summary*, *Status update*, *Explain it*, *Review
packet* (the default), *Hand-over* and *Full dossier*. Anything the agent hasn't
written yet is listed but disabled, with the reason.

With **Contents & links** on (every preset but the chat-sized ones):

- the document opens on a linked table of contents;
- every section ends with a **↑ Contents** link;
- the diff gets one heading per file;
- each open comment links to its file's diff.

The anchors follow GitHub's heading rule, so they work in the preview, in a
saved `.md` on GitHub and in the `.html` page.

### Destinations

The preview **is** the payload. It can go to:

- the clipboard;
- a `.md` file, or a self-contained `.html` page to send to someone without
  clash. The page opens offline by double-click. The document takes the whole
  window, with a contents rail that follows your reading, and every mermaid
  diagram has **Diagram** / **Source** tabs and follows the page's light or dark
  theme. With **Explanation drawings** set to a depth, it also carries the
  item's explanations and opens on an explorer: the drawing fills the screen,
  clicking a part zooms into it, and Esc zooms back out into its box. A reading
  panel beside it lists the parts and holds the write-up, and `←`/`→` step
  between parts. The drawings stay sandboxed. Both saves use the system save
  dialog, then offer **Reveal in Finder** and **Copy path**;
- Slack or Discord;
- a ticket as one comment: **Post to Jira…** and/or **Post to Linear…**, one
  button per tracker that is configured or primary. The key is pre-filled from
  the item's remembered ticket (⚙ Settings → Ticket) or detected in its title or
  branch.

### Who posts

Who posts a share is a per-destination setting (`trackers.jira.transport`,
`trackers.linear.transport`, `workflows.chat_transport`):

- **An agent session** (the default) posts it through a skill you name
  (`trackers.jira.skill`, `trackers.linear.skill`, `workflows.chat_skill`), or
  else with whatever tooling that session has connected (a Jira or Linear MCP
  server, say).
- **clash itself** posts over HTTPS: the webhooks (`slack_webhook`,
  `discord_webhook`), the Jira email and API token, or the Linear API key. Jira
  receives the document as wiki markup; Linear as markdown.

Neither route is a fallback for the other, and each button names its route.

### Decision notifications

**Notify decisions** (`workflows.notify_webhook`: `off` (default), `slack` or
`discord`) announces every item an *agent* parks at a decision stage on the
configured webhook. Your own clicks never post.

## Agents, models and skills

- **Agent CLI**: workflow sessions run on Claude Code or OMP. `workflows.agent`
  defaults to `ask` at every step and can be fixed per item. See
  [OMP sessions](sessions.md#omp-sessions).
- **Lead and subagents**: every workflow session runs on one pinned **lead
  model**. Under `workflows.delegation = team` (the default) the lead sizes the
  round first: small work stays with the lead, and large work is split across
  parallel subagents on a pinned **subagent model** — explorers, implementers
  in waves of disjoint files, or one reviewer per area, followed by
  verification. `solo` keeps everything in one session.
  [Details](workflows.md#lead-and-subagents).

  Each harness has its own pair, in *Settings → Models*:
  - Claude Code: `workflows.lead_model` (default `claude-opus-5-5`) and
    `workflows.subagent_model` (default `claude-sonnet-5-5`), forced on every
    subagent except Claude Code's built-in Explore agent;
  - OMP: `workflows.omp_model` and `workflows.omp_subagent_model` (both empty by
    default, meaning omp's own choice), the subagent one pinned through a
    per-run omp config overlay that leaves your `config.yml` alone.

  The models are applied at launch and again every time the session is resumed
  (after a restart, an update or a reload).
- **Sized reviews**: code reviews (verdict rounds included) are sized by the change they
  read (`workflows.review_sizing`, default `auto`, Claude Code only): a small
  change (≤150 lines, ≤5 files, one PR) runs its lead on
  `workflows.small_review_model` (default `claude-sonnet-5-5`), a large one
  (≥800 lines or ≥25 files) its subagents on
  `workflows.large_review_subagent_model` (default `claude-opus-5-5`), and a
  resumed round keeps the models it launched with.
  [Details](workflows.md#sized-review-rounds).
- **Skills**: the agent side is five skills embedded in the binary:
  - `clash-workflow` — the executor: plans, implements, opens PRs, writes the
    hand-off;
  - `clash-plan-review` — the plan reviewer;
  - `clash-explain` — the explainer;
  - `clash-drift-review` — plan vs changes;
  - `clash-pr-review` — the code reviewer, for every outcome: local findings,
    PR comments, answered threads, or a verdict (also usable on its own).

  Startup installs missing skills and refreshes the ones you never edited. clash
  asks (*Keep my edits* / *Overwrite*) only when an upgrade would replace a
  skill you edited by hand; `general.skills_update` (*Settings → Workflows →
  Skill updates*) pins the answer. The ☰ button on the WORKFLOWS section lists
  every installed skill and badges the ones clash manages.

## Storage

`~/.claude/clash/workflows/<project>/<item>/` (or `workflows_dir`) holds:

- `meta.json` and `plan.md`, with `plan-history/` for its versions;
- `review.md` (your decisions) and `agent-review.md` (the reviewers' rounds);
- `annotations.json`;
- `explain-plan.*`, `explain-diff.*` and `drift.*`;
- `handoff.md`, `explainers.json` and `run.json`;
- `history/<NNN>/` round snapshots.

The contract for what agents may write, and when, is
[workflows.md](workflows.md).
