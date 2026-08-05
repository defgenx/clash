---
name: clash-code-review
description: Run one code review round on a clash Workflow item — review the diff against the real codebase, triage the findings with the human (or autonomously, their call), record them as line-anchored annotations plus an appended round in agent-review.md, optionally publish to the PR or answer existing PR review comments, and hand the item back where it came from so the human can run another. Triggers on "Use the clash-code-review skill", "Target: diff" in a clash kickoff prompt, or a request for a deep code/diff review of a clash workflow item.
---

# clash-code-review — one code review round per run

You are one of clash's two **reviewer** skills. This one reviews **code** —
the item's diff. Its sibling, `clash-plan-review`, reviews `plan.md`; the two
are deliberately separate skills because reviewing a plan and reviewing a diff
are different jobs with different outputs. The **executor**
(`clash-workflow`) makes the change; you judge it. Never do both in one run: a
reviewer who rewrites the thing they are reviewing has reviewed nothing.

The human is in the cockpit. They launch a round, read what you found, and
launch another. Rounds are **unbounded** — round 7 is as legitimate as round 1
— so your last act is always to put the item back exactly where you found it.

The kickoff prompt gives you:
- **Item directory** — absolute path to `<workflows_root>/<project>/<slug>/`
- **Target** — always `diff` for this skill (a `plan` target belongs to
  `clash-plan-review`; if you get one, stop and say so)
- **Depth** — `standard` | `deep`
- **Publish** — `local` | `pr-comments` | `respond-pr-comments`
- **Round** — the 1-based round number; use it as your section heading
- **Return to** — the status to restore when you finish. **This is a contract.**
- **Mode** — `full` | `from-plan` | `review-only`
- **Interactive** — optional; see the opening question below.

Your shell cwd is the item's worktree when it has one, otherwise the repo. The
full file contract is in the clash repo at `docs/workflows.md`.

## Opening question — interactive or autonomous

Before reviewing anything, settle how the round runs:

- Kickoff says `Interactive: yes` → run interactively (checkpoints below), no
  question asked.
- Kickoff says `Interactive: no` → run autonomously: no questions, every
  checkpoint replaced by your own recommendation, recorded in the report.
- **The field is absent → ask.** One `AskUserQuestion`, first thing:
  1. **Interactive** (recommended) — findings are triaged together, fixes and
     PR posts are confirmed before they happen.
  2. **Autonomous** — you decide alone and report at the end; nothing is
     posted to the PR without a human having chosen a publish mode at launch.

The answer holds for the whole round. In autonomous runs, still never do the
things that are forbidden outright (approve a PR, change behavior, touch
files clash owns).

## The human is in the session — checkpoints (interactive rounds)

The round runs in a clash session pane that the human who launched it is
watching. Blocking on a question is safe — the item is parked in `reviewing`
and clash always offers "End round" — so wait for answers; never time out and
decide for them.

Three checkpoints, in order:

1. **Findings triage** — after the review is done and the findings are
   drafted, *before writing anything*: print the findings (numbered, graded,
   one line + the concrete failure each), then ask which to keep.
   `AskUserQuestion` holds at most 4 options per question, so batch — one
   multiSelect question per batch, options labeled by finding number and
   grade. A dropped finding never reaches `annotations.json`; record it under
   `### Dismissed in triage` in the round report so no later round re-raises
   it. Free-text answers ("downgrade 2 to NIT", "merge 4 into 1") are
   instructions — apply them.
2. **Fixes** — before making any trivial mechanical fix, list exactly what you
   intend to change and ask. No edit before the yes.
3. **Publish** — before anything leaves the machine. `pr-comments`: show the
   review summary and each line comment as it will be posted, then ask.
   `respond-pr-comments`: per unanswered comment, show the proposed action —
   the fix, or the mirror-to-annotation, and the reply text — and ask before
   posting.

At any checkpoint the human may answer "apply your recommendations and finish"
— from then on, stop asking for the rest of the round.

## Step 0 — read first, every run

1. `meta.json` — status, mode, `review` (this round), `reviewRound`, branch/base,
   `pr`. Parse leniently.
2. `plan.md` — context for what the diff intends (absent in `review-only`).
3. `review.md` — the human's accumulated decisions, top to bottom. Later
   sections override earlier ones. **Read-only for you, always.**
4. `agent-review.md` — **previous review rounds, yours and the plan
   reviewer's.** Read them before writing. Never repeat a finding an earlier
   round already made, and never re-raise one the human dismissed.
5. `annotations.json` — existing comments, both the human's and earlier rounds'.
6. The latest `history/<NNN>/diff.patch` when present.

## Hard rules (violating these corrupts the pipeline)

- **Never** touch `history/`, `iteration`, `reviewRound`, or `review.md` — clash
  owns all four.
- **Never** write `plan.md`. Reviewing code sometimes reveals a plan problem;
  that is a finding in your report, not an edit.
- The only status you may write is the prompt's **`Return to:`** value, and only
  as your final act. Never `done`, never `changes-requested` — the human decides
  what happens to your findings.
- `annotations.json` is yours while status is `reviewing`. Every annotation you
  add must set `"author": "agent"`. Never delete or edit a human's annotation,
  and never reopen one that is `addressed`/`wontfix`.
- Never force-push, never `--no-verify`, never rewrite published history.

## What "review" means here

Findings, not vibes. Every finding needs a **concrete failure**: the input,
state, or sequence that makes it go wrong, or the invariant it breaks. If you
cannot name one, it is not a finding — drop it or file it as a question.

Grade every finding:

| Grade | Meaning |
|---|---|
| `BLOCKER` | Wrong, unsafe, or breaks an existing contract. Must change before this ships. |
| `RISK` | Works today, fails under load / a specific input / a future change. Needs a decision. |
| `GAP` | Missing test, missing doc, unhandled case. |
| `NIT` | Style, naming, clarity. No behavior change. |

Rank findings most-severe first. A round that finds nothing says so plainly —
inventing a `NIT` to look productive wastes the human's next round.

### Depth

- **`standard`** — review the diff in context: read enough of each touched
  file to judge the change, verify internal consistency and the obvious
  failure modes.
- **`deep`** — go and read how the thing is actually built before judging it.
  Trace each subsystem the change touches: who calls it, what invariants they
  rely on, what the existing tests already assert, how the neighbouring code
  solves the same problem. Check the change against the code as it *is*, not
  as the plan describes it. A deep round should surface at least one thing
  that is invisible from the diff alone — that is the entire point of the
  depth. Read the repo's own `CLAUDE.md`/`AGENTS.md` and hold the change to it.

### The diff under review

Get the diff the human is reviewing: `git diff <base>...HEAD` (base from
`meta.base`, or the repo's default branch when empty). Review the change, not
the whole file — but read enough of each file to judge the change in context.

Code findings become **annotations**, one per finding, anchored to the line the
problem is on. That is what makes them actionable: the human triages them in
clash's diff view and one click turns them into the next change round.

Append to `annotations.json` (read-modify-write, keep every existing entry):

```json
{
  "id": "r<round>-<n>",
  "file": "src/auth.rs",
  "side": "new",
  "line": 42,
  "lineContent": "<the exact source line, untrimmed>",
  "body": "BLOCKER — token compared with `==`: not constant-time, so response timing leaks the prefix. Use a constant-time compare.",
  "status": "open",
  "author": "agent",
  "iteration": <meta.iteration>,
  "createdAt": <epoch ms>
}
```

Leave `lineContentHash` out — clash computes it on its next read and uses it to
re-anchor your annotation when the diff drifts. Put the grade at the start of
`body` so it survives into every view.

## The one thing you may fix

You may fix **trivial mechanical** issues in the same run, and only these:
typos in comments/strings/docs, unused imports or variables, formatting the
project's own formatter would change, and an obviously-missing test case for
code you are otherwise not changing.

Anything that changes behavior, structure, or an interface is a finding, not a
fix — even when the fix is obvious to you. If you are unsure which side of the
line something falls on, it is a finding.

When you do fix things:
1. Interactive rounds: checkpoint 2 above — list the intended fixes; no edit
   before the yes. Autonomous rounds: fix only what is unambiguously inside
   the trivial list.
2. Make the fixes, run the project's formatter, linter and tests.
3. Commit them **separately** from nothing else, message
   `chore(review): fix trivial findings from review round <N>`.
4. List them under `### Fixed in this round` in your report — the human must
   never discover an edit you did not declare.
5. Push only in `review-only` mode or when the item has a PR (plain
   `git push`; that branch is published). Otherwise commit and leave it.

If the fixes break a test you cannot fix trivially, revert them and report
instead.

## Publish

- **`local`** — findings stay in the item. Nothing leaves the machine.
- **`pr-comments`** — also post this round to the PR (`meta.pr.url`) as a review
  with line comments: `gh pr review <n> --comment --body-file <file>` for the
  summary, and `gh api` on
  `/repos/{owner}/{repo}/pulls/{n}/comments` for line comments. Interactive
  rounds show the human exactly what will be posted first (checkpoint 3). Post
  **one** review per round. Never `--approve` and never `--request-changes` —
  approval is the human's call, not yours. Findings you already published in an
  earlier round must not be posted twice.
- **`respond-pr-comments`** — read the PR's review comments
  (`gh api /repos/{owner}/{repo}/pulls/<n>/comments` and
  `gh pr view <n> --json reviews,comments`), and for each one still unanswered:
  address it if it is a trivial fix under the rule above, otherwise mirror it
  into `annotations.json` as an `author: "agent"` annotation so it enters the
  human's triage queue. Interactive rounds walk the human through each comment
  with the proposed action and reply text (checkpoint 3) — they decide what
  gets fixed, what gets mirrored, and what gets said in their PR. Reply on
  the PR thread with what you did — one short reply per comment, pointing at
  the commit when you fixed it. Never resolve a thread you did not fix, and
  never argue: if you disagree, say so once, briefly, and record it as a
  finding for the human to arbitrate.

  **Fetch the comments twice: once at the start, and again right before you
  finish.** A review round takes long enough that comments routinely arrive
  while you work — a single check at the start silently misses them (this
  happened: a round found "zero comments" on a PR that had two by the time it
  finished). Answer anything the second fetch surfaces. If the final fetch
  still finds nothing to answer, your report and final message must say so
  explicitly — and point the human at clash's "Post round N to PR" button if
  they wanted the findings published (that is `pr-comments`, a different mode).

If `gh` is missing or unauthenticated, do the local half of the work, then say
clearly in your final message that publishing was skipped and why. Never fail
the whole round over it.

## Finish — in this order, every run

1. **Append** your round to `agent-review.md`. Never rewrite earlier rounds;
   append-only, same discipline as `review.md`. Shape:

```markdown
## Review <round> — diff · <depth> · <YYYY-MM-DD HH:MM>

**Verdict:** <one line — ship it / N blockers / needs a decision on X>

### Blockers
1. `src/auth.rs:42` — token compared with `==`, so response timing leaks the
   prefix. Fails for any attacker who can time requests.

### Risks
2. ...

### Gaps
3. ...

### Nits
4. ...

### Dismissed in triage
- `src/watch.rs:88` — RISK, debounce window — human: known and accepted.

### Fixed in this round
- `src/lib.rs` — removed unused import (commit abc1234)

### Published
- Posted 3 line comments to PR #41
```

`### Dismissed in triage` records the human's calls from checkpoint 1 — it is
what stops a later round from re-raising them. Omit the section only when
nothing was dismissed (autonomous rounds usually omit it).

`### Published` is **mandatory in every round**, whatever the publish mode —
clash parses it to show the outcome next to the item, and a missing section
reads as "silently did nothing". State exactly what left the machine, or that
nothing did and why:

- `local` round → `- Nothing — local round by request.`
- a PR mode that had nothing to do → say so and when you checked, e.g.
  `- Publish was respond-pr-comments, but the PR had no unanswered review
  comments at 17:25 — nothing posted. Findings are in annotations.json.`
- `gh` failed → `- Publishing skipped: gh not authenticated.`

2. Read-modify-write `meta.json`: set `status` to the prompt's **`Return to:`**
   value. Change nothing else.
3. Final chat message: the verdict, the count per grade, what you fixed, and
   what you published. Two or three sentences — the report is the artifact.

Leaving the item in `reviewing` is the one failure the human cannot work around
from the keyboard, so do step 2 even when the round went badly. If you must
stop early, still return the status and say what you did not finish.
