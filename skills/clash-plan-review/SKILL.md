---
name: clash-plan-review
description: Review a clash Workflow item's plan.md before it is implemented, walk the human through every issue with options and a recommendation, and write their decisions into the item's files. Invoked by the clash-review skill as the plan-review engine; also usable directly on any plan when asked for a thorough plan review. Covers architecture, code quality, tests and performance, gives each issue concrete options with an opinionated recommendation, and asks the human which direction to take before recording it. Triggers on "Use the clash-plan-review skill", "Target: plan" in a clash kickoff prompt, or a request to review/critique an implementation plan.
---

# clash-plan-review — review the plan, not the code

The plan-review engine for clash Workflows. `clash-review` invokes you and owns
the file contract and the status hand-back; you supply the judgement.

Derived from the public `plan-review` skill, and **interactive like it**: the
human launched this round from clash and is watching it in a session pane.
Your recommendations are input; their decisions are the deliverable. Present
each issue with options and an opinionated recommendation, ask with
`AskUserQuestion` which direction they want, and record the answer in the
round. Never silently settle something they would have decided differently.

Blocking on a question is safe: the item is parked in `reviewing` while you
run, and clash always offers "End round" if the human walks away. Wait for the
answer — never time out and pick for them. Run without questions **only** when
the kickoff prompt says `Interactive: no` or the human tells you to finish
without them; then state each recommendation and move on, and their answer
arrives as the next round instead.

Do not set a model — clash pins one when it launches the session.

## Opening question

Ask once, before reviewing, how to run the round:

1. **Section by section** (recommended) — review one section at a time
   (Architecture → Code quality → Tests → Performance), pausing after each to
   triage its issues while they are fresh.
2. **Batched** — do the full review first, then one triage pass over every
   finding at the end.
3. **Unattended** — no further questions; write the report with your
   recommendations, exactly as if the kickoff had said `Interactive: no`.

## Scope

Review `plan.md` against the **real codebase**. A plan review that only reads the
plan is a proofreading pass: open the files the plan names, check the APIs it
assumes exist, and confirm the approach fits how the code actually works today.

Do not change code. A trivial, obviously-correct fix is the reviewer's one
allowance (see `clash-review`); anything else is a finding, not an edit.

## Engineering preferences (use these to rank and recommend)

- DRY matters — flag repetition aggressively.
- Well-tested code is non-negotiable; too many tests beats too few.
- "Engineered enough": neither under-engineered (fragile, hacky) nor
  over-engineered (premature abstraction, needless complexity).
- Err toward handling more edge cases, not fewer. Thoughtfulness over speed.
- Bias toward explicit over clever.

## The four sections

Cover all four in one pass, at most **4 issues each**, strongest first. A section
with nothing wrong gets one line saying so — padding it dilutes the rest.

1. **Architecture** — component boundaries, coupling and the dependency
   direction, data flow, scaling characteristics, single points of failure,
   security boundaries (auth, data access, API surface).
2. **Code quality** — organization and module structure, DRY violations, error
   handling and missing edge cases, technical-debt hotspots, anything over- or
   under-engineered relative to the preferences above.
3. **Tests** — coverage gaps (unit/integration/e2e), assertion strength, missing
   edge cases, untested failure modes and error paths.
4. **Performance** — N+1 queries and access patterns, memory, caching
   opportunities, high-complexity paths.

## For each issue

1. State the problem concretely, with `file:line` references into the real code.
2. Give 2–3 options, including "do nothing" where that is defensible.
3. For each option: implementation effort, risk, blast radius on other code, and
   maintenance burden.
4. Name your recommended option and tie the reason to a preference above.
5. **Then ask.** After presenting a section's issues (or the whole batch, in
   batched mode), put them to the human with `AskUserQuestion` — one question
   per issue, at most 4 per call. Label every option with the issue number and
   option letter (`3b — extract the shared helper`), put your recommended
   option **first**, and always include a "Dismiss — not an issue" option. A
   free-text answer is an instruction: fold it into the record.

Number the issues and letter the options (`3b`), so the human can approve one by
name in their change request.

A decision here authorizes nothing in this round: you still change no code and
never edit `plan.md`. The point of asking is the **record** — each issue carries
the human's call, so their next *Request changes* can say `apply 1a and 3b`
instead of re-litigating the round.

## Where findings go

`clash-review` owns these files; follow its contract exactly.

- **Plan-level findings and the verdict** → appended as a new round in
  `agent-review.md`. This is the whole output for most plan reviews: a plan has
  no diff to annotate.
- **Findings that land on a specific existing line** → `annotations.json` with
  `"author": "agent"`, so they enter the human's triage loop and one *Request
  changes* turns them into work.
- Never write `plan.md` (you would be reviewing your own text next round) and
  never write `review.md` (clash's record of human decisions).

Record the human's decisions in the round itself: give each issue a
`**Decision:**` line — `accepted 3b`, `dismissed — <their words>`, or
`unreviewed (unattended round)`. Dismissed issues stay in the report (that is
what stops a future round from re-raising them) but never become annotations.
Close the round with an `### Accepted changes` list — one line per accepted
issue+option — written to be pasted into clash's *Request changes* composer:
that note is the next round's prompt, and this list is how the review turns
into applied work.

## Verdict

End the round with one explicit line: is this plan safe to implement as written,
safe with the accepted changes, or does it need rework before implementation?
When running interactively, confirm the verdict with the human as the final
question — it is theirs, not yours. Be willing to say the plan is fine.
