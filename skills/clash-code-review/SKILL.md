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
- **PR** — optional; the URL of the PR this round is about (also in
  `meta.review.prUrl`). Absent means "the item's whole change" — the local
  diff, with the primary `meta.pr.url` as the PR to talk to.

  A **linked** PR is in a *different repository* than your cwd, so a round
  naming one changes both what you read and where the findings go — see "The
  diff under review" below. Scope every `gh` call with `--repo <owner/repo>`
  parsed from the URL, read file context through `gh api`, and never edit
  anything locally for a linked PR: the checkout around you is the item's
  repo, not that one. The one thing that *is* an annotation on the item's own
  diff is work the linked PR implies **here** — "this repo's caller needs the
  new field too" is a finding about your own files.
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
   `respond-pr-comments`: per open thread, show the proposed **decision**
   (fixed / queued / declined / needs the human), the fix if there is one, and
   the reply text that will be published — then ask before posting. Every
   thread gets a reply; what the human decides is *which* reply.

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

Which diff depends on the kickoff's `PR:` field, and getting this wrong means
reviewing the wrong repository:

- **No `PR:`, or `PR:` naming the primary `meta.pr.url`** — the item's own
  change: `git diff <base>...HEAD` (base from `meta.base`, or the repo's
  default branch when empty). This is the normal round.
- **`PR:` naming a linked PR** — *that PR's* diff, which is in another
  repository and not in your checkout at all:
  `gh pr diff <n> --repo <owner/repo>`. The human scoped the round to one
  repository of a multi-repo change, so reviewing your local branch instead
  would be reviewing something they did not ask about. Read file context
  through `gh api` (`/repos/{owner}/{repo}/contents/<path>?ref=<head sha>`);
  never edit anything locally for a linked PR.

Review the change, not the whole file — but read enough of each file to judge
the change in context.

Code findings become **annotations**, one per finding, anchored to the line the
problem is on. That is what makes them actionable: the human triages them in
clash's diff view and one click turns them into the next change round.

**A linked PR's findings belong on that PR, not in `annotations.json`.** The
files are in another repository, so an annotation naming them anchors to
nothing in the item's diff view and lands in the orphan tray — visible, but
attached to nowhere and impossible to act on. Publish them as line comments on
that PR instead (`--repo`-scoped, exactly as `pr-comments` describes below);
clash pre-selects that publish mode when the round is scoped to a linked PR,
and if the round somehow arrives with `Publish: local` say so in the report
rather than writing annotations that cannot be triaged.

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

**One mode widens this: `respond-pr-comments`.** There, a reviewer has already
asked for the change and the human launched the round to act on it, so a
bounded, unambiguous fix inside the diff under review is in scope. See the
Publish section for exactly where that line falls.

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

  **A draft PR cannot take a review.** GitHub rejects `gh pr review` on a draft
  ("Draft pull requests cannot be reviewed"), so check first —
  `gh pr view <n> --json isDraft` — and when it is a draft, post the summary as
  an ordinary PR comment (`gh pr comment <n> --body-file <file>`) instead. Line
  comments through the pulls API are unaffected either way. Say which form you
  used in `### Published`. Reviewing a draft is the normal case, not an edge
  one: a draft is exactly what a change is in while it is being reviewed, and
  the local half of the round — findings as annotations — never depended on the
  PR's state at all.
- **`respond-pr-comments`** — **close every open thread on the PR with a
  published decision, and fix what the comments ask for.** This is not a
  review round with a posting step at the end; the threads are the work.

  Read them from **the round's PR** (the kickoff's `PR:` field, else the
  primary `meta.pr.url`) via
  `gh api /repos/{owner}/{repo}/pulls/<n>/comments` and
  `gh pr view <n> --json reviews,comments`.

  **Which threads are open.** A thread is **settled** when *you* have replied
  in it and nobody has spoken after you. Everything else is open, and that
  covers two kinds of thread — both are your job:

  - **Somebody else's comment** with no answer from you, or with a follow-up
    after your answer. The obvious case.
  - **A finding this pipeline published itself** (a line comment from an
    earlier `pr-comments` round, posted under your own account) with no
    decision under it. A finding is a *question*; until the thread says what
    was done about it, the PR shows a pile of remarks and no outcomes. These
    are not "your own comments, so not your problem" — they are precisely the
    loop this mode closes.

  clash's own count (the "Answer N PR comments" button) uses that same rule.

  **Every open thread gets a published reply. No exceptions.** Right or wrong,
  the comment gets an answer that says what was decided and why. A thread you
  read and silently skipped is indistinguishable from a round that never ran —
  that is the actual failure this rule exists to prevent. Four honest
  decisions, and one of them must be in every reply:

  | Decision | Reply says | Also do |
  |---|---|---|
  | **Fixed** | what changed, and the commit sha | make the change, commit it |
  | **Queued** | that it needs a change round, and why it is not being done here | mirror it into `annotations.json` (`author: "agent"`) and name the annotation id |
  | **Declined** | why the comment does not hold — the code, the invariant, or the constraint that makes it wrong or unnecessary. Once, briefly, no argument | record it in your report |
  | **Needs the human** | what the open question is and who has to answer it | mirror it as an annotation for triage |

  **Fixing is in scope here** — wider than the "one trivial thing" allowance
  above, because a reviewer's comment on a PR *is* a change request and the
  human launched this round to act on them. Implement a comment when the
  change is bounded and unambiguous inside the diff under review. It stays a
  **Queued** decision (not a fix) when it changes the design, touches code
  outside the diff, needs a plan revision, or you would have to guess what
  "right" means. When in doubt, queue it and say so in the thread — a wrong
  fix costs more than a round.

  Every fix follows the fix discipline above: run the project's formatter,
  linter and tests; commit with a message naming the round; **push** when the
  branch is published (a local-only commit leaves the PR stale and your reply
  pointing at a sha nobody can see). Group the fixes into commits that make
  sense — one per thread is fine, one per theme is better.

  **Reply *in the thread*, with the replies endpoint:**

  ```
  gh api --method POST \
    /repos/{owner}/{repo}/pulls/<n>/comments/<root_comment_id>/replies \
    -f body="Fixed in abc1234 — <one line>."
  ```

  `<root_comment_id>` is the thread root's `id` (the `in_reply_to_id` of its
  replies, or the comment's own `id` when it is the root). `gh pr comment` is
  **not** a reply: it posts a detached issue comment on the PR, so the thread
  still shows no answer and still counts as open. That is the difference
  between "the round replied" and "the round appeared to do nothing".

  Never resolve a thread you did not fix. Never argue: state your reasoning
  once and, if you disagree with something material, record it as a finding
  for the human to arbitrate.

  Interactive rounds walk the human through the threads before anything is
  posted (checkpoint 3): per thread, the proposed decision, the fix if any,
  and the reply text. They can overrule any of it. Autonomous rounds decide
  by the table above and report every decision.

  **Fetch the comments twice: once at the start, and again right before you
  finish.** A round takes long enough that comments routinely arrive while you
  work — a single check at the start silently misses them (this happened: a
  round found "zero comments" on a PR that had two by the time it finished).
  Everything the second fetch surfaces gets the same treatment. If the final
  fetch finds every thread settled, say so explicitly in your report and final
  message — and point the human at clash's "Post round N to PR" button if what
  they wanted was this round's *findings* published (that is `pr-comments`, a
  different mode).

  `### Published` must list **one line per thread**: the thread (file:line or
  comment id), the decision, and what was posted. A count is not enough — the
  human has to be able to check the list against the PR.

If `gh` is missing or unauthenticated, do the local half of the work, then say
clearly in your final message that publishing was skipped and why. Never fail
the whole round over it.

## Decide whether this round should be applied

Every round ends with one more call: **should these findings become a fix round
now?** You are not fixing anything beyond the one-file allowance — clash
records a change round and launches an executor that addresses the open
annotations. You are saying whether that is worth doing now.

The kickoff's **`Auto-apply:`** field says what your answer does:

- `Auto-apply: yes` → a `yes` from you starts that fix round immediately, with
  no further human action. Say so when you ask.
- `Auto-apply: no` → your answer is a recommendation on a button the human
  presses. Never tell them it will happen by itself.

**Interactive rounds: ask**, once, after triage, carrying your recommendation
and its reason: **Start the fix round now** or **Not yet**.

**Autonomous rounds: judge it yourself**, on what survived triage:

- **Apply** when a blocker or a real risk is open — anything that is wrong,
  unsafe, or breaks under an input the code will actually see. Those are worth
  a round on their own.
- **Do not apply** when everything open is a nit, or when the diff is clean.
  A fix round costs tokens, rewrites the branch and asks for another review;
  spending that on formatting is a bad trade the human did not ask for.
- **Do not apply** when a finding needs a decision you could not make — two
  valid designs, an intentional trade-off you cannot confirm, a fix that would
  change behaviour the human may want. An executor cannot ask; it will pick.
- **Do not apply** when the item is at `pr-ready`: the branch is published and
  under human review, so pushing new commits mid-review is the human's call.
  Recommend it in the report instead.
- When nothing is open, the answer is **no**.

Judge by *severity*, never by count: one timing-attack blocker is worth a
round, nine nits are not.

## Finish — in this order, every run

1. **Append** your round to `agent-review.md`. Never rewrite earlier rounds;
   append-only, same discipline as `review.md`. Shape:

```markdown
## Review <round> — diff · <depth> · <YYYY-MM-DD HH:MM>

**Verdict:** <one line — ship it / N blockers / needs a decision on X>

**Apply:** yes|no — <one line: why this is or is not worth a fix round>

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

   The heading's shape is contractual: clash reads the round number and,
   from the first word of the tail, its target. Numbers restart per target
   — `diff` rounds are numbered among themselves — so `<round>` is the
   number the kickoff gave you (`Round:`), and dropping the target would
   make two different rounds indistinguishable.

`### Dismissed in triage` records the human's calls from checkpoint 1 — it is
what stops a later round from re-raising them. Omit the section only when
nothing was dismissed (autonomous rounds usually omit it).

`### Published` is **mandatory in every round**, whatever the publish mode —
clash parses it to show the outcome next to the item, and a missing section
reads as "silently did nothing". State exactly what left the machine, or that
nothing did and why:

- `local` round → `- Nothing — local round by request.`
- a respond round → one line per thread, e.g.
  ```
  - PR #41, 4 threads, all answered:
    - `src/auth.rs:42` (c#1902) — FIXED in abc1234 (constant-time compare); replied.
    - `src/auth.rs:88` (c#1903) — QUEUED as r5-2 (needs a plan revision); replied.
    - `src/watch.rs:12` (c#1904) — DECLINED (the debounce is intentional, see
      docs/watching.md); replied.
    - `src/lib.rs:3` (c#1905) — NEEDS THE HUMAN (which error type is canonical);
      queued as r5-3; replied.
  ```
- a respond round with nothing open → say so, when you checked, and why, e.g.
  `- Publish was respond-pr-comments; at 17:25 every thread on PR #41 was
  settled (each of its 7 findings carries our decision reply). Nothing
  posted.`
- `gh` failed → `- Publishing skipped: gh not authenticated.`
- a draft PR → `- Posted 3 line comments and a summary comment to PR #41 (a
  draft, so not as a review).`

**`**Apply:**` is mandatory too**, and must be exactly `yes` or `no` followed
by the reason — it is the decision from the section above, and clash reads it
to know whether to start the fix round (or, when auto-apply is off, to mark the
action as recommended). Anything it cannot read as yes/no leaves the call to
the human, wasting the judgement you just made. Do not hedge it; the reason
line is where nuance goes.

2. Read-modify-write `meta.json`: set `status` to the prompt's **`Return to:`**
   value. Change nothing else.
3. Final chat message: the verdict, the count per grade, what you fixed, and
   what you published. Two or three sentences — the report is the artifact.

Leaving the item in `reviewing` is the one failure the human cannot work around
from the keyboard, so do step 2 even when the round went badly. If you must
stop early, still return the status and say what you did not finish.
