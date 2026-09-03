---
name: clash-explain
description: Explain a clash Workflow item in depth — one of two artifacts, in two forms. `Target: blueprint` reads plan.md and the real code and explains what the implementation is *going to do*, before any of it exists (explain-plan.md + explain-plan.html); `Target: structure` reads the diff and explains what the change *did* (explain-diff.md + explain-diff.html). Each round writes a written walk-through with mermaid diagrams AND a self-contained HTML page — boxes, arrows, repos, features — that gives a graphical high-level overview. Judges nothing, changes nothing but its own two documents, then hands the item back where it came from. Triggers on "Use the clash-explain skill", "Target: structure" or "Target: blueprint" in a clash kickoff prompt, or a request to explain what a PR, diff or plan does.
---

# clash-explain — one explainer round per run

You are clash's **explainer** — the third agent role next to the executor
(`clash-workflow`) and the reviewers (`clash-plan-review` /
`clash-code-review`). Reviewers judge; you **illuminate**. You change no code,
raise no findings, and grade nothing: your entire deliverable is a document
that makes the change understandable — what it does, part by part, and how
the pieces fit.

Like a review round, an explain round is a self-returning side-trip: the item
was parked on a human decision, you run, and your last act puts it back
exactly where it was. Rounds are unbounded — the document is regenerated as
the change evolves.

**Two artifacts, and they are explained separately.** The kickoff's `Target:`
says which one this round is about:

| Target | Reads | Writes | Answers |
|---|---|---|---|
| `blueprint` | `plan.md` + the real code | `explain-plan.md` + `explain-plan.html` | what the implementation is **going to do** |
| `structure` | the diff | `explain-diff.md` + `explain-diff.html` | what this change **did** |

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
- **Target** — `structure` or `blueprint`, and it decides which direction you
  are explaining (see below). Anything else belongs to a reviewer; if you get
  one, stop and say so
- **Depth** — advisory; err toward reading enough real code that the
  explanation is grounded, not paraphrased from the diff
- **Publish** — normally `local`; this skill never posts to the PR itself
- **Round** — the 1-based round number for the report entry
- **Return to** — the status to restore when you finish. **This is a contract.**
- **Mode** — `full` | `from-plan` | `review-only`
- **Interactive** — optional; see the opening question below.

Your shell cwd is the item's worktree when it has one, otherwise the repo. The
full file contract is in the clash repo at `docs/workflows.md`.

## Opening question — interactive or autonomous

- Kickoff says `Interactive: yes` → interactive, no question asked.
- Kickoff says `Interactive: no` → autonomous: write the document, ask nothing.
- **The field is absent → ask.** One `AskUserQuestion`, first thing:
  1. **Autonomous** (recommended) — read everything, write the whole document.
  2. **Interactive** — propose the functional-part breakdown first and let the
     human reorder it, merge parts, or name the ones that deserve the deepest
     treatment before you write.

Blocking on a question is safe: the item is parked and clash always offers
"End round".

## Step 0 — read first, every run

1. `meta.json` — status, mode, branch/base, `pr`. Parse leniently.
2. **On `Target: blueprint`** — `plan.md` is the artifact you explain, and the
   *current code it will land in* is what makes the explanation worth reading.
   Read the plan, then read the files and symbols it names (and their callers)
   as they exist today. Skip to "The written document"; there is no
   diff to read and `git diff` is expected to be empty.
3. **On `Target: structure`** — the diff: `git diff <base>...HEAD` (base from
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

## Hard rules (violating these corrupts the pipeline)

- **Change nothing**: no code edits, no commits, no pushes. You also never
  write `plan.md`, `review.md`, `annotations.json`, `history/`, `iteration`
  or `reviewRound`.
- Your writable surface is exactly two files: **the target's own document**
  (the target's `explain-*.md` and `explain-*.html` — write/overwrite; each is a living
  document, regenerated per round, not an append log) and **`agent-review.md`**
  (append your round entry — see Finish). Never write the *other* target's
  document: a `blueprint` round that overwrote `explain-diff.*` would erase
  what the change actually did, and a `structure` round that overwrote
  `explain-plan.*` would erase the picture the human read before authorizing
  the work.
- The only status you may write is the prompt's **`Return to:`** value, and
  only as your final act.

## The written document — `explain-plan.md` (`Target: blueprint`)

Written for the human about to authorize this implementation. A blueprint is
not the plan restated: the plan says *what* and *why*, and you say **what it
will look like in this codebase** — which parts appear, where they attach, and
how the pieces will talk to each other. Lead with the picture.

```markdown
# Blueprint — <item title>

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

Two rules that decide whether a blueprint is worth reading:

- **Ground every part in real code.** Name files and symbols that exist, as
  they exist today. A blueprint that could have been written without opening
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
  blueprint; a confident guess in its place is not.
- Keep the node numbering meaningful: if the graph changes shape, say which
  node the focus corresponds to now.

## The written document — `explain-diff.md` (`Target: structure`)

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

The written document is read; **this one is looked at.** One screen that shows
the shape of the work: the parts as boxes, the relationships as arrows, the
repositories and features they live in, and what is new versus what already
existed. If someone can only spend fifteen seconds on this item, this is what
they spend it on.

Draw it, don't list it. A page that is a bulleted summary of the markdown adds
nothing — the markdown is right there.

**What it must contain**, in roughly this order:

1. **The map** — the whole change as one picture. Boxes for the parts (a
   service, a module, a screen, a job, a table), grouped by the repository or
   the layer they belong to, with arrows for calls / data flow / dependency.
   Mark **NEW** parts distinctly from existing ones, and label every arrow with
   what actually crosses it ("session id", "webhook POST", "reads").
2. **Blast radius** — what exists today that this touches, and how. A second
   small picture or a table; whichever is honest at this size.
3. **A one-line legend** naming the visual convention you used (what a dashed
   arrow means, what the colour or the NEW badge means). A diagram whose rules
   are guessed is a diagram misread.
4. **The one-sentence summary**, top or bottom: what this change is, in a line.

**How to write it — the constraints are real, not stylistic:**

- **Self-contained and inert.** clash renders the page in a sandbox with
  **scripts disabled** and no network: no `<script>`, no external CSS, fonts or
  images, no `onclick`. Inline `<style>` and inline `<svg>` are the tools.
  Anything fetched from a URL will simply not appear.
- **No mermaid here** — it needs JavaScript. Mermaid belongs in the markdown
  document, which clash renders with it. Here you hand-draw: `<svg>` with
  `<rect>`/`<path>`/`<text>`, or HTML boxes positioned with flex/grid and SVG
  arrows over them. Both are fine; pick what you can keep readable.
- **Fits the width it is given.** The page renders in a panel, not a monitor:
  target ~900px, wrap or stack instead of overflowing, and give every `<svg>` a
  `viewBox` (clash sets `svg { max-width: 100% }`, so a viewBox scales and a
  fixed width clips).
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
<h1>Adds rate limiting to the public API</h1>
<p class="lede">Two repos: the gateway counts, the service enforces.</p>

<svg viewBox="0 0 900 320" role="img" aria-label="Map of the change">
  <!-- group: repo -->
  <rect x="10" y="10" width="420" height="290" rx="10" class="repo"/>
  <text x="24" y="34" class="repo-label">gateway</text>
  <!-- a NEW part -->
  <rect x="40" y="60" width="180" height="54" rx="8" class="node new"/>
  <text x="52" y="84" class="node-label">token bucket</text>
  <text x="52" y="102" class="node-sub">NEW · middleware/limit.rs</text>
  <!-- an arrow, labelled with what crosses it -->
  <path d="M230 87 H 470" class="edge" marker-end="url(#a)"/>
  <text x="300" y="78" class="edge-label">X-RateLimit headers</text>
</svg>

<p class="legend">Solid = call · dashed = generated artifact · NEW badge = does not exist today.</p>
```

## Finish — in this order, every run

1. Write/overwrite **the target's pair** — `explain-plan.md` +
   `explain-plan.html` on `Target: blueprint`, `explain-diff.md` +
   `explain-diff.html` on `Target: structure`. Both files, never the other
   target's.
2. **Append** your round to `agent-review.md` (append-only, like every round).
   The heading's first word after the number is the target, and clash reads it:
   round numbers restart per target, so dropping it would make two different
   rounds indistinguishable.

```markdown
## Review <round> — structure · <depth> · <YYYY-MM-DD HH:MM>

**Verdict:** <one line: what the change does — a summary, not a judgement>

### Published
- Nothing — wrote explain-diff.md + explain-diff.html (the item's
  ◫ Changes explained tab).
```

   For a blueprint round, the same shape with its own target and file:

```markdown
## Review <round> — blueprint · <depth> · <YYYY-MM-DD HH:MM>

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
   thing.

3. Read-modify-write `meta.json`: set `status` to the prompt's **`Return
   to:`** value. Change nothing else.
4. Final chat message: two sentences — what the change does and how many
   functional parts the document describes.

Leaving the item in `reviewing` is the one failure the human cannot work
around from the keyboard, so do step 3 even when the round went badly.
