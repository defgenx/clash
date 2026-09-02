---
name: clash-explain
description: Explain a clash Workflow item's changes in depth — read the diff (or PR), organize what it does by functional part, draw mermaid diagrams of the flows it touches, and write the explainer to structure.md for clash's Structure tab, then hand the item back where it came from. Judges nothing and changes nothing; the deliverable is understanding. Triggers on "Use the clash-explain skill", "Target: structure" in a clash kickoff prompt, or a request to explain what a PR or diff does.
---

# clash-explain — one structure round per run

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

The kickoff prompt gives you:
- **Item directory** — absolute path to `<workflows_root>/<project>/<slug>/`
- **Target** — always `structure` for this skill (anything else belongs to a
  reviewer; if you get one, stop and say so)
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
2. The diff: `git diff <base>...HEAD` (base from `meta.base`, or the repo's
   default branch when empty). This is the artifact you explain.
3. `plan.md` and `review.md` — the intent and the decision history (both
   read-only). Explain what the diff *does*, not what the plan promised — but
   where they differ, say so in the document.
4. When `meta.pr` exists and `gh` is available, the PR description and title
   are context. Never post anything.
5. Enough of the surrounding code to explain the change in its habitat: who
   calls the changed functions, what the touched subsystems do. An explainer
   that only paraphrases hunks adds nothing a diff view doesn't already show.

## Hard rules (violating these corrupts the pipeline)

- **Change nothing**: no code edits, no commits, no pushes. You also never
  write `plan.md`, `review.md`, `annotations.json`, `history/`, `iteration`
  or `reviewRound`.
- Your writable surface is exactly two files: **`structure.md`**
  (write/overwrite — it is a living document, regenerated per round, not an
  append log) and **`agent-review.md`** (append your round entry — see
  Finish).
- The only status you may write is the prompt's **`Return to:`** value, and
  only as your final act.

## The document — `structure.md`

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

## Finish — in this order, every run

1. Write/overwrite `structure.md`.
2. **Append** your round to `agent-review.md` (append-only, like every round):

```markdown
## Review <round> — structure · <depth> · <YYYY-MM-DD HH:MM>

**Verdict:** <one line: what the change does — a summary, not a judgement>

### Published
- Nothing — wrote structure.md (rendered in the Structure tab).
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
