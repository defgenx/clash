---
name: clash-explain
description: Explain a clash Workflow item in depth — one of two artifacts, in two forms, as a tree you can zoom into. `Target: explain-plan` reads plan.md and the real code and explains what the implementation is *going to do*, before any of it exists (explain-plan.md + explain-plan.html); `Target: explain-diff` reads the diff and explains what the change *did* (explain-diff.md + explain-diff.html). Each round writes a written walk-through with mermaid diagrams AND a self-contained HTML page — boxes, arrows, repos, features — that gives a graphical high-level overview, plus a JSON index of its parts so a reader can open any part one level deeper (`Node:` dives, up to three levels). Judges nothing, changes nothing but its own two documents, and runs alongside the item's other agents without touching its status. Triggers on "Use the clash-explain skill", "Target: explain-diff" or "Target: explain-plan" in a clash kickoff prompt, or a request to explain what a PR, diff or plan does.
---

# clash-explain — one explainer round per run

> **Agent CLI.** clash launches this skill under Claude Code or OMP (oh-my-pi).
> `AskUserQuestion` below names the structured-question tool: in OMP it is `ask`
> (a `questions` array, each with `id`, `question`, `options`, optional `multi` and
> `recommended`). Every other instruction is the same under both.

You are clash's **explainer** — the third agent role next to the executor
(`clash-workflow`) and the reviewers (`clash-plan-review` /
`clash-code-review`). Reviewers judge; you **illuminate**. You change no code,
raise no findings, and grade nothing: your entire deliverable is a document
that makes the change understandable — what it does, part by part, and how
the pieces fit.

Unlike a review round, an explain round **parks nothing**: it runs alongside
whatever else the item is doing — a plan review, the implementation of the
plan you are explaining — and leaves the item's status alone. Rounds are
unbounded — the document is regenerated as the change evolves.

**Two artifacts, and they are explained separately.** The kickoff's `Target:`
says which one this round is about:

| Target | Reads | Writes | Answers |
|---|---|---|---|
| `explain-plan` | `plan.md` + the real code | `explain-plan.md` + `explain-plan.html` | what the implementation is **going to do** |
| `explain-diff` | the diff | `explain-diff.md` + `explain-diff.html` | what this change **did** |

Never write the other target's pair. "What this is going to do" and "what it
did" are both worth keeping — a human reads the first to authorize the work
and the second to review it — so a round that overwrote the other one would
destroy the record it was not asked about.

The plan explanation runs *before* the work exists, which is the whole point of
it: someone is about to authorize an implementation, and a plan in prose is
hard to check against the code it will land in. Your job is to make the shape
of the work concrete enough to disagree with. You still judge nothing and
decide nothing: the human reads it and then does what they want with the plan.

**Two forms, and you write both every round.** They are not alternatives:

- **`explain-<target>.md`** — the written walk-through. Read top to bottom,
  with ```mermaid fences for the flows. This is where the detail lives: file
  and symbol names, what attaches to what, the open questions.
- **`explain-<target>.html`** — one **graphical high-level overview**: boxes,
  arrows, the repos, the features, the parts. This is what someone looks at
  for fifteen seconds to know the shape of the work. See "The HTML page".

Write the markdown first (it is the thinking), then the page (it is the
summary). A page that disagrees with the document is worse than no page.

The kickoff prompt gives you:
- **Item directory** — absolute path to `<workflows_root>/<project>/<slug>/`
- **Focus** — optional: what the human wants this pass to concentrate on, in
  their words. Lead with it — see "When the kickoff carries `Focus:`".
- **Target** — `explain-diff` or `explain-plan`, and it decides which direction you
  are explaining (see below). `drift` is accepted **only with `Node:`** — a dive
  into one divergence of the drift report (see "Dive mode"). Anything else
  belongs to a reviewer; if you get one, stop and say so
- **Detail** — `overview` | `critical` | `full`: how much of the tree a
  top-level round writes up front. See "The tree".
- **Node** — present only on a **dive**: the one part (`3`, `3.2`, `3.2.1`) this
  run explains one level deeper. A dive has no `Round:` and appends no round
  entry. See "Dive mode".
- **Depth** — advisory; err toward reading enough real code that the
  explanation is grounded, not paraphrased from the diff
- **Publish** — normally `local`; this skill never posts to the PR itself
- **Round** — the 1-based round number for the report entry
- There is **no** `Return to:` — you run *alongside* the item's other agents
  (a plan review, the implementation), never park it, and never write
  `meta.json`. See Hard rules.
- **Mode** — `full` | `from-plan` | `review-only`
- **Interactive** — optional; see the opening question below.
- **Delegation** — `team` (also what an absent field means) or `solo`,
  optionally naming the subagent model: `team (subagents on <model>)`. See
  "Lead and subagents" below.

Your shell cwd is the item's worktree when it has one, otherwise the repo. The
full file contract is in the clash repo at `docs/workflows.md`.

## Opening question — interactive or autonomous

- Kickoff says `Interactive: yes` → interactive, no question asked.
- Kickoff says `Interactive: no` → autonomous: write the document, ask nothing.
- **The field is absent → ask.** One `AskUserQuestion`, first thing:
  1. **Autonomous** (recommended) — read everything, write the whole document.
  2. **Interactive** — propose the functional-part breakdown first and let the
     human reorder it, merge parts, and confirm the critical set — the parts
     whose inside you write up front under `Detail: critical` — and the
     **views** (see "Views") you think the change deserves, before you write.

Blocking on a question is safe: nothing waits on you — the item's other work
carries on — and clash can always stop tracking the round.

## Step 0 — read first, every run

1. `meta.json` — status, mode, branch/base, `pr`. Parse leniently.
2. **On `Target: explain-plan`** — `plan.md` is the artifact you explain, and the
   *current code it will land in* is what makes the explanation worth reading.
   Read the plan, then read the files and symbols it names (and their callers)
   as they exist today. Skip to "The written document"; there is no
   diff to read and `git diff` is expected to be empty.
3. **On `Target: explain-diff`** — the diff: `git diff <base>...HEAD` (base from
   `meta.base`, or the repo's default branch when empty). This is the artifact
   you explain.
4. `plan.md` and `review.md` — the intent and the decision history (both
   read-only). Explain what the diff *does*, not what the plan promised — but
   where they differ, say so in the document.
5. When `meta.pr` exists and `gh` is available, the PR description and title
   are context. Never post anything.
6. Enough of the surrounding code to explain the change in its habitat: who
   calls the changed functions, what the touched subsystems do. An explainer
   that only paraphrases hunks adds nothing a diff view doesn't already show.

## Linking tickets

Link every ticket you mention (`PS-12345`, `ENG-42`) wherever you write it — documents, reports, annotations, PR text. The kickoff's `Ticket links:` field says where keys link: one URL template, or a routing table such as `PS, OPS → https://acme.atlassian.net/browse/{key}; any other key → https://linear.app/acme/issue/{key}` — Jira and Linear keys look alike, so the key's prefix picks the template. Replace `{key}` with the key: `[PS-12345](https://acme.atlassian.net/browse/PS-12345)`. `Ticket links: none` means mention keys unlinked. Without the field, **ask the human for the ticket host** the first time you need one (interactive) and tell them it can be saved in Settings → Trackers & sharing; in an autonomous run leave the key unlinked and say so in your report. Never guess a host. The field is for ticket keys only: an external document (a Confluence page, a Google doc, any page you have a URL for) is linked with its own full URL.

## Lead and subagents — `Delegation: team`

Under `team` you are the **lead**: you decide how the round splits, dispatch
what is worth dispatching, and integrate and verify what comes back. `team` is
permission to parallelize, not a quota. A subagent costs a brief, a cold start
that re-reads the code, and your check of its result — it pays only when its
unit is independent and big enough that doing it yourself would take longer.
The goal is the best result in the least wall-clock time. Mapping the code is parallel; the documents are yours — an explanation is one voice, and a subagent's summary is not yet a fact. Under
`solo`, do everything yourself and skip this section.

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
waves below describe the split **at full scale**; shrink them to the size you
picked.

1. **Map** — one read-only mapper per area (a subsystem, a repo, the plan's
   group of actions or the diff's group of files), launched together, each
   returning what exists there: files, symbols, callers, data flow, with
   `file:line`. An area you can read in a few files is mapped by you, not by
   a mapper.
2. **Write** — you write both documents from those maps, and spot-check every
   file and symbol you name before it goes in.

How to delegate (Claude Code: the `Agent` tool; OMP: its `task` tool):

- **Parallel means one message.** Launch every independent subagent of a wave
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
  to a PR and ask the human (`AskUserQuestion`). Say so in every brief — a
  subagent that writes `meta.json` or commits corrupts the round.
- **Verify, don't forward.** A subagent's output is a claim. Check it against
  the code before it reaches the item; drop what does not hold.
- **Verify in proportion.** Settle a claim yourself when one read settles it.
  Send it to a fresh refuting verifier only when refuting it takes real
  tracing (callers, cross-file invariants, another repo), and batch those per
  file or area — never one verifier per trivial finding.
- **Report the split.** Your final message says how you sized the round, how
  the work was divided (or why it was not), and what verification dropped
  or changed.

## Hard rules (violating these corrupts the pipeline)

- **Change nothing**: no code edits, no commits, no pushes. You also never
  write `plan.md`, `review.md`, `annotations.json`, `history/`, `iteration`
  or `reviewRound`.
- Your writable surface is **the target's own tree** and **`agent-review.md`**:
  - a top-level round writes the overview (`explain-<target>.md`, `.html`,
    `.json`), the levels its `Detail:` asks for under `explain-<target>/`, and
    appends its round entry to `agent-review.md` (see Finish);
  - a dive writes **only** its node's three files
    (`<tree>/<node>.md`, `.html`, `.json`) and appends nothing;
  - either may add **view pages** beside a level it writes
    (`<level>.<view>.html`, see "Views") — never beside a level it does not.

  Every file is a living document, overwritten per round, not an append log. Never write the *other* target's
  document: an `explain-plan` round that overwrote `explain-diff.*` would
  erase what the change actually did, and an `explain-diff` round that overwrote
  `explain-plan.*` would erase the picture the human read before authorizing
  the work.
- **Never write `meta.json`**, not even the status. You run alongside other
  agents that own it — an executor mid-implementation, a reviewer that will
  restore its own `Return to:` — and your read-modify-write would race theirs.
  clash knows you finished from your round entry in `agent-review.md`.
- **Append to `agent-review.md` in one shell append** (`cat >> agent-review.md
  <<'EOF'` … `EOF`), never by rewriting the file: a reviewer running beside
  you may be appending its own round at the same moment, and a whole-file
  write from an older read would erase it.

## The tree — levels you can zoom into

An explanation is not one page. It is an **overview plus a level per part**,
so a reader can open the part they care about instead of paying for the whole
change at full depth. The model is the C4 one: each zoom level has its own
altitude, and "deeper" means a *different* altitude, never the parent redrawn
bigger.

| Level | Id | Shows | Size |
|---|---|---|---|
| Overview | — | the parts of the change, grouped by repo/subsystem (the documents below) | 5–12 parts |
| L1 — a part | `3` | inside one part: its components, functions and types, the data/control flow between them, and the contract at its boundary with sibling parts | 3–8 parts |
| L2 — a component | `3.2` | the mechanism: key functions, state transitions, invariants, edge cases, with `file:line` | 3–6 parts |
| L3 — code | `3.2.1` | an annotated walk-through of one function or hunk, with short excerpts. A leaf: lists no parts | — |

Files, for `Target: explain-diff` (`explain-plan` and `drift` are the same with
their own stem):

```
explain-diff.md / .html / .json     the overview
explain-diff/3.md / .html / .json   part 3, one level in
explain-diff/3.2.md / …             part 3.2
explain-diff/3.2.1.md / …           part 3.2.1 (a leaf)
```

**Every level writes its `.json` index**, the overview included — it is what
clash builds the breadcrumb, the parts strip and the dive buttons from:

```json
{
  "title": "Retry wrapper",
  "basis": { "iteration": 2, "head": "<git rev-parse HEAD>" },
  "nodes": [
    { "id": "3.1", "title": "Backoff policy", "summary": "one line",
      "critical": true, "why": "every outbound call goes through it",
      "files": ["src/http/retry.rs:40"], "isNew": true, "leaf": false }
  ],
  "edges": [ { "from": "3.1", "to": "3.2", "label": "attempt count" } ]
}
```

- **Ids are the numbers the documents already use.** A level's parts are its
  own id plus one segment: the overview lists `1`..`n`, part `3` lists `3.1`..,
  never deeper than three segments. The overview's numbered nodes *are* its
  parts — the graph, the `### 3.` headings and the index agree.
- **`basis`** — `iteration` from `meta.json`, `head` from `git rev-parse HEAD`
  (omit `head` for a plan explanation). clash marks a level drawn from another
  basis, or before its parent was redrawn, as *out of date*.
- **`critical`** marks where a reader's attention pays — blast radius, a
  subtle mechanism, a cross-repo contract, a migration, concurrency, a
  security edge. It is not a grade, and `why` says it in one line. At most
  **3** critical parts in the overview and **2** per level below it.
- **`leaf: true`** on a part with nothing worth a level of its own (a rename,
  a config line). Parts at the third level are leaves by definition.
- **Write order:** a level's `.md` and `.html` first, its `.json` **last** —
  for a dive the index landing is how clash knows you finished. A top-level
  round writes the overview's index *before* the levels under it, so they
  read as current.

**`Detail:`** decides what a top-level round writes up front — the rest stays
a dive someone pays for only by clicking it:

- `overview` — the overview and its index only.
- `critical` (the default) — plus the L1 level of each part flagged critical.
- `full` — plus the L1 level of every part that is not a leaf.

L2 and L3 are never written up front. Under `team` delegation, the L1 levels
are good subagent units: one mapper per critical part.

**In the HTML page**, wrap each drawn part in a link to its id so a click on
the drawing opens it: `<a href="#node-3"><g>…</g></a>` in SVG, or
`<a href="#node-3" class="box">…</a>` in HTML. That is a plain fragment link —
the page still runs no script; clash follows the fragment from outside.

## Dive mode — the kickoff carries `Node:`

A dive explains **one part one level deeper**. Often it arrives as a
follow-up in the session that drew the overview — you have already read the
code, so do not re-read what you know; read only what the part needs.

1. Read the parent level first: its `.json` (the part's title, summary,
   files and its siblings) and its `.md`. The overview's parent is
   `explain-<target>.md/.json`; part `3.2`'s parent is `explain-<target>/3.json`.
2. Read the code under that part — deeper than the parent did. That is the
   only reason the level exists.
3. Write `explain-<target>/<node>.md`, then `.html`, then `.json`, at the
   altitude the table gives for that depth. Its parts are `<node>.1`, `<node>.2`…
   (none at the third level). Keep the parent's words for the part: the
   breadcrumb uses them.
4. **Append nothing to `agent-review.md`** and touch no other level — not the
   parent, not a sibling. A dive is not a round.
5. Final chat message: one sentence — what the part does, how many parts
   it lists, and the view you drew for it, if any (see "Views": the same rule
   applies to a part — a part that is one request path may need its `flow`). Then stop and wait: the next dive may be queued into this
   session.

The dive's `.md` follows the shape of the explain-diff document scaled to one
part (what / where / how, a diagram when it earns its place, the parts as
`### <id>. <title>` sections); its `.html` is one picture of that part. Honour
`Focus:` the same way as a round: lead with it.

**A drift dive** (`Target: drift` with `Node:`) explains one area of the drift
report: what the plan said, what the code does, and the mechanism that makes
them differ — in `drift/<node>.*`. You still judge nothing: the drift round's
grades stand, and a dive writes no annotations and changes no grade. Read
`drift.md`/`drift.json` for the parent level.

## The written document — `explain-plan.md` (`Target: explain-plan`)

Written for the human about to authorize this implementation. A plan
explanation is
not the plan restated: the plan says *what* and *why*, and you say **what it
will look like in this codebase** — which parts appear, where they attach, and
how the pieces will talk to each other. Lead with the picture.

```markdown
# Plan explained — <item title>

## The plan as one graph
<**One** mermaid diagram, first thing in the document: the mid-level actions
this implementation will take, in the order they depend on each other. This is
the deliverable — someone should be able to agree or disagree with the design
from this graph alone, and the human's decision is made on it.

Rules that make it decidable rather than decorative:

- **5–12 nodes.** Fewer is a plan restated; more is a task list nobody reads.
  One node = one mid-level action ("parse the config layer", "add the retry
  wrapper"), never a micro-step ("add an import") and never a bare file name.
- **Number every node**, `1`..`n`, and keep the numbers stable through the
  document. The human decides on this graph and may hand a number back to you
  ("dive into 3"), so an unnumbered node cannot be discussed.
- **Label in the domain's words**, with the real file or symbol it lands in
  where it fits: `3["3 Retry wrapper — infra/http/client.rs"]`.
- Edges are dependency/order. Mark the ones that are new (`-->`) apart from
  what already exists (`-.->`) when it helps; keep it readable.
- `flowchart TD`. Valid mermaid only — it must render.>

## The actions
<The same numbered nodes, one short block each, in graph order. This is the
graph's legend: nothing here that is not a node up there, and no node up
there missing here.>

### 1. <Action name — matches node 1's label>
- **What**: what will exist or change that does not today.
- **Where**: the files/symbols it lands in — existing ones by name, new ones
  marked NEW, each with its role in one line.
- **How it attaches**: what calls it, what it calls, what it changes about
  behavior that exists today.

### 2. <next action…>

## What it touches that already exists
<A table or list: existing file/symbol → what changes about it. This is the
blast radius, and it is the half a prose plan hides.>

## Sequence
<A second mermaid diagram *only when it earns its place* — a
`sequenceDiagram` when the change is a flow at runtime, or a `flowchart` of
what will exist and how it connects when the action graph does not already
show it. Skip it otherwise; a forced diagram is noise, and the graph above is
the one the human decides on.>

## Open questions & risks
<Where the plan is under-specified, what the implementer will have to decide,
what could go wrong in this codebase specifically. Observations for the human
to rule on — you are not the reviewer and you do not grade the plan.>

## Not in scope
<What this plan deliberately does not do, when it says so — the fastest way
for the human to spot a wrong assumption.>
```

Two rules that decide whether a plan explanation is worth reading:

- **Ground every part in real code.** Name files and symbols that exist, as
  they exist today. An explanation that could have been written without opening
  the repository is the plan with diagrams, and the human already has the plan.
- **Say what you could not settle.** This is read to decide whether the work
  should go ahead as planned; a gap you hid becomes a decision the implementer
  makes alone. Put it under Open questions.

**When the kickoff carries `Focus:`**, the human named what this pass must
concentrate on — often a node number from a previous graph, or the part of the
change they care about. That is the round's job:

- Redraw the whole graph (the document is replaced, not appended to), but
  **lead the answer with the focus**: a `## Focus — <what they asked>` section
  right after the graph that settles it, in the code, with file and symbol
  names, before the rest of the actions.
- If what they asked cannot be settled from the plan and the code, say so
  there and say what would settle it. "I could not answer this" is a useful
  explanation; a confident guess in its place is not.
- Keep the node numbering meaningful: if the graph changes shape, say which
  node the focus corresponds to now.

## The written document — `explain-diff.md` (`Target: explain-diff`)

Written for a reviewer or teammate meeting this change cold. Organize by
**behavior, not by file**: "what does this change do" splits into functional
parts (a feature, a bugfix mechanism, a refactor, a migration), and each part
names its files — never the reverse. Shape:

```markdown
# What this change does

## At a glance
<2–4 sentences: the change's purpose and its shape. Then one line of stats:
N files, +A/−D, the subsystems touched.>

## Functional parts
### 1. <Part name — a behavior, e.g. "Parked annotations">
- **What**: the behavior after this change, in plain words.
- **Why**: the problem it solves (from plan.md/review.md when stated, from
  the code when not).
- **Where**: the files/symbols that implement it, with one-line roles.
- **How**: the mechanism — enough detail that the reviewer can predict what
  the code does before reading it.

### 2. <next part…>

## How the pieces fit
<One or two mermaid diagrams IN ```mermaid fences — clash renders them.
Choose the diagram that matches the change: a flowchart for control/data
flow, a sequenceDiagram for multi-actor interactions, a stateDiagram-v2 for
lifecycle changes. Keep each diagram small (5–15 nodes) and labeled in the
domain's words; two small diagrams beat one wall.>

## Risks & review focus
<Where a reviewer should spend their attention: the subtle part, the
behavior change with blast radius, the thing tests don't cover. Observations,
not verdicts — you are not the reviewer.>

## Reading order
<The order to read the diff in so it explains itself, one line per step.>
```

Diagram rules: valid mermaid only (it must render — prefer `flowchart TD`,
`sequenceDiagram`, `stateDiagram-v2`); node labels from the domain ("composer
submit", "meta write"), not file paths; no diagram for a part that is a plain
list — a forced diagram is noise.

## The HTML page — `explain-plan.html` / `explain-diff.html`

The written document is read; **this one is looked at.** It is a *different
reading dimension*, not the markdown drawn: the write-up is for someone who
will read the code, the drawing is for someone who never will. Pitch it at a
smart newcomer — a PM, a new teammate, you in six months — who knows nothing
about this codebase. From this page alone they must understand **what is going
to be done (or was done), for whom, and how it will work**, in thirty seconds.
A drawing that is correct but has to be decoded has failed.

**Higher level than you think.** The overview drawing shows *the idea*, not the
code:
- **no file paths, function, class or type names** on the overview page —
  those live in the write-up and in the parts' own levels (where a small
  code name under a box is fine);
- **people and things a newcomer recognises**: "the member", "the admin
  screen", "the nightly job", "the payments service", "the database" — not
  `MemberPolicyResolver`;
- **5–8 boxes**, never more: if the change has more parts, the drawing groups
  them ("everything that sends emails") and the parts strip carries the rest;
- **one sentence per step**, in everyday words, as if explaining it out loud.

Draw it, don't list it. A page that is a bulleted summary of the markdown adds
nothing — the markdown is right there.

**What it must contain**, top to bottom:

1. **The goal, in one sentence** — what this change achieves for a user or a
   system, in plain words ("Customers get a clear error instead of a timeout
   when they exceed their quota"), not what files it edits.
2. **Before → after, in two lines** — what happens today, what will happen
   instead ("Today: an over-quota call hangs for 30s. After: it is refused at
   once with a clear message."). This is the line a newcomer remembers.
3. **The map, told as a story.** The parts as boxes (a service, a module, a
   screen, a job, a table), grouped by repository or layer, **laid out in the
   order things happen** — left to right or top to bottom — with the arrows
   **numbered** ① ② ③ along the main path. Beside or under the picture, the
   same numbers as short sentences: "① the gateway counts the call; ② over the
   limit, it answers 429 before the service is reached; ③ …". The numbers are
   what turns boxes into *how it works*.
4. **What changes, marked on the map** — NEW, CHANGED and untouched-context
   parts visibly different (fill/border, with the legend), so "what is going
   to be done" is readable straight off the picture.
5. **Blast radius** — what exists today that this touches, and how, in plain
   words ("anyone calling the public API more than 100 times a minute"). A
   small table or a strip under the map.
6. **A one-line legend** naming the convention (what dashed means, what the
   colours or badges mean). A diagram whose rules are guessed is misread.

**Legibility rules — these are what make it understandable:**

- **Label boxes by role, in plain language**: the title says what the part
  *does* in this change ("Counts calls per API key"). On the overview that is
  all; on a part's level the code name may go underneath, small ("NEW ·
  middleware/limit.rs"). A box titled `RateLimiterMiddlewareImpl` makes the
  reader open the code, which is what the page exists to spare them.
- **Label arrows with a verb and what crosses**: "sends order id", "reads
  quota", "publishes `order.paid`" — never a bare line, never just "uses".
- **At most ~10 boxes on the overview** and ~8 on a part's level. More than
  that, group: one box per subsystem with its pieces named inside, and leave
  the inside to that part's level.
- **One main path, drawn straight**; side paths thinner or dashed. No crossing
  arrows, no arrow longer than it needs to be, no two arrows between the same
  pair of boxes (merge their labels).
- **Context stays dim.** Untouched neighbours are drawn only when the change
  talks to them, lighter than the parts that change.
- **Before you finish, read the page as that reader** — the newcomer above,
  who knows the product a little and this code not at all. For each box ask "do I know what it does?", for
  each arrow "do I know what happens here?", and for the whole "could I explain
  this change to a colleague now?". Rename, regroup or renumber until every
  answer is yes. If the honest answer needs another angle (the order of calls,
  the states, the data), that is a view — see "Views".

**How to write it — the constraints are real, not stylistic:**

- **Self-contained and inert.** clash renders the page in a sandbox with
  **scripts disabled** and no network: no `<script>`, no external CSS, fonts or
  images, no `onclick`. Inline `<style>` and inline `<svg>` are the tools.
  Anything fetched from a URL will simply not appear.
- **No mermaid here** — it needs JavaScript. Mermaid belongs in the markdown
  document, which clash renders with it. Here you hand-draw: `<svg>` with
  `<rect>`/`<path>`/`<text>`, or HTML boxes positioned with flex/grid and SVG
  arrows over them. Both are fine; pick what you can keep readable.
- **Tables size to their content.** No `table-layout: fixed`, no fixed
  column widths and no `width: 100%` on a table of short cells — a stretched
  table spreads its columns across a 1400px pane and reads worse than a
  compact one. Let the browser size each column; top-align the cells.
- **Uses the space it is given.** The page fills a whole pane, often 1400px
  or more wide, and the reader zooms into it, so draw the map as one `<svg>`
  with a wide `viewBox` (about 1200–1600 units, wider than tall) and no fixed
  `width`/`height`. clash scales every drawing whose viewBox is at least 400
  units wide to the pane's width, as tall as the pane allows, and lifts any
  `max-width` you put on `body`. Give boxes room: text at 13–15 units,
  generous gaps, no box narrower than its label. Keep prose above and below
  the drawing short; it is capped to a readable line length anyway.
- **Theme-aware by default.** clash injects the app's colours as the page's
  background/foreground before your styles, so a page that sets no colours
  looks native in both light and dark. If you do set colours, set both — use
  `@media (prefers-color-scheme: dark)` — and never assume a white canvas.
- **A fragment is fine.** Write a full `<html>` document or just the body
  markup; clash wraps a fragment.
- **Readable text.** No 9px labels, no more than ~20 boxes. If it does not fit
  in twenty boxes, group it: one box per subsystem with its parts named
  inside.

Sketch of the shape (yours will differ — this is the altitude, not a template):

```html
<h1>Callers over their quota get a 429 instead of a timeout</h1>
<p class="lede"><b>Today:</b> a caller over its quota waits 30s for a timeout.
<b>After:</b> the front door refuses it at once with a clear message.</p>

<svg viewBox="0 0 900 320" role="img" aria-label="Map of the change">
  <!-- group: repo -->
  <rect x="10" y="10" width="420" height="290" rx="10" class="repo"/>
  <text x="24" y="34" class="repo-label">gateway</text>
  <!-- a NEW part -->
  <rect x="40" y="60" width="180" height="54" rx="8" class="node new"/>
  <text x="52" y="84" class="node-label">Counts calls per API key</text>
  <text x="52" y="102" class="node-sub">NEW</text>
  <!-- a numbered arrow, labelled with a verb and what crosses it -->
  <path d="M230 87 H 470" class="edge" marker-end="url(#a)"/>
  <text x="300" y="78" class="edge-label">② forwards, adds X-RateLimit headers</text>
</svg>

<ol class="steps">
  <li>① A call arrives; the gateway counts it against the caller's key.</li>
  <li>② Under the limit, it is forwarded with the remaining quota in headers.</li>
  <li>③ Over the limit, the gateway answers 429 — the service never sees it.</li>
</ol>

<p class="legend">Filled = NEW · outlined = CHANGED · dim = untouched context · ①②③ = the order things happen.</p>
```

## Views — the same level from another angle

The map (boxes and arrows) answers *what are the parts and how do they
connect*. Some changes are mostly about a different question, and a map
answers it badly. A level may therefore carry up to **2** extra drawings —
**views** — beside its main page. clash shows them as tabs next to
*◫ Diagram* and *☰ Write-up*.

| View (`kind`) | Answers | Draw it when |
|---|---|---|
| `flow` | what happens, in order, to one request / event / job: numbered steps across the components it passes through, sync vs async, where it can fail | the change sits on a request path, an API, a webhook, a queue consumer, auth, a retry loop |
| `state` | which states a thing has and which transitions exist, with the new/removed/guarded edges marked | a status, lifecycle or pipeline gains or changes states |
| `data` | the shape of the data before → after, and who writes / reads each field | a migration, a new table or column, a payload or contract change |
| `before-after` | behaviour today next to behaviour after, side by side | a refactor or behaviour change that adds few parts but changes how they act |
| `rollout` | the order things must land in: repos, migrations, flags, the window where old and new must coexist | several repos, a migration, a feature flag — the usual case for an `explain-plan` round |
| `dependency` | who calls / imports what, and which edges are new | a large refactor or a module split |

**Choose by the question the reader will have, never by habit.** Ask: "what
would someone reviewing this get wrong from the map alone?" If the answer is
"the order of the calls", draw `flow`; "which statuses can follow which",
`state`; "what the table looks like after", `data`. If the map already answers
it, draw no view — a view that restates the map is noise, and every one costs
tokens. Most changes need zero or one; two is the cap.

- **Interactive** — after proposing the parts, *suggest* the views: name each
  one, the question it answers and why the map does not, and recommend one or
  none (`AskUserQuestion`, multi-select, "No extra view" as an option). Draw
  what the human picks. Also suggest a view when they had not thought of one
  and the change clearly calls for it — that suggestion is the point.
- **Autonomous** — decide yourself with the rule above, and write the reason
  in the view's `why`: it is the tab's tooltip, and it is how a reader learns
  why this angle was drawn.
- **`Focus:` naming a view** ("draw the request flow") is a request for it:
  draw it whatever the rule says.

Files and index — a view is one HTML page, the level's id plus the view id:

```
explain-diff.flow.html       a view of the overview
explain-diff/3.state.html    a view of part 3
```

and it is listed in that level's `.json` (written last, as always):

```json
"views": [ { "id": "flow", "kind": "flow", "title": "Checkout request",
             "why": "the change is three hops on one request; the map hides their order" } ]
```

- `id`: lowercase letters, digits and `-`, starting with a letter; usually the
  kind itself.
- The page follows every rule of "The HTML page" (inert, self-contained,
  theme-aware, viewBox). Keep the map's names and numbers for the same parts,
  and link a part to its level the same way (`<a href="#node-3">`) — a reader
  should be able to jump from step 4 of a flow into part 3.
- A `flow` is a sequence picture: lanes (one per component) left to right,
  steps numbered top to bottom, arrows labelled with what crosses them, the
  failure branch drawn rather than mentioned. A `state` view is states as
  rounded boxes and transitions as labelled arrows, new ones marked NEW.
- The write-up may carry the same angle as a mermaid `sequenceDiagram` /
  `stateDiagram-v2` / `erDiagram` where it helps the prose — the view page is
  the drawn, glanceable form of it.

## Finish — in this order, every run

1. Write/overwrite **the target's overview** — `explain-plan.md` +
   `explain-plan.html` + `explain-plan.json` on `Target: explain-plan`,
   `explain-diff.{md,html,json}` on `Target: explain-diff`, with any view
   pages before the `.json` (see "Views") — then the levels `Detail:` asks for
   (see "The tree"). Never the other target's files. (A
   dive follows "Dive mode" instead and skips step 2.)
2. **Append** your round to `agent-review.md` — one shell append, never a
   rewrite (see Hard rules). This entry is also how clash learns you are
   done, so write it last and write it even when the round went badly. The
   heading's first word after the number is the target, and clash reads it:
   round numbers restart per target, so dropping it would make two different
   rounds indistinguishable.

```markdown
## Review <round> — explain-diff · <depth> · <YYYY-MM-DD HH:MM>

**Verdict:** <one line: what the change does — a summary, not a judgement>

### Published
- Nothing — wrote explain-diff.md + explain-diff.html (the item's
  ◫ Changes explained tab).
```

   For a plan-explanation round, the same shape with its own target and file:

```markdown
## Review <round> — explain-plan · <depth> · <YYYY-MM-DD HH:MM>

**Verdict:** <one line: the shape of what will be built — a summary, not a
judgement>

### Published
- Nothing — wrote explain-plan.md + explain-plan.html (the item's
  ◫ Plan explained tab).
```

   The reviewers' `**Apply:**` line has no place here: it declares whether a
   round's findings are worth a change round, and an explanation is not
   findings. Write no such line — clash never offers to "apply" an explainer,
   and claiming otherwise would put a button on the item that does the wrong
   thing. Write no `**Next:**` line either: recommending a next step is a
   judgement, and an explainer judges nothing.

3. Final chat message: two sentences — what the change does and how many
   functional parts the document describes.
