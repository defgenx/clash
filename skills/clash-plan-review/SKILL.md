---
name: clash-plan-review
description: Review a clash Workflow item's plan.md before it is implemented, and write the findings into the item's files. Invoked by the clash-review skill as the plan-review engine; also usable directly on any plan when asked for a thorough plan review. Covers architecture, code quality, tests and performance, and gives each issue concrete options with an opinionated recommendation. Triggers on "Use the clash-plan-review skill", "Target: plan" in a clash kickoff prompt, or a request to review/critique an implementation plan.
---

# clash-plan-review — review the plan, not the code

The plan-review engine for clash Workflows. `clash-review` invokes you and owns
the file contract and the status hand-back; you supply the judgement.

Derived from the public `plan-review` skill, with one deliberate difference:
**you are not interactive.** The original pauses after every section and asks the
human which direction to take. Here nobody is sitting in this session — it is a
spawned review round whose output is files the human reads later in clash. So
never call `AskUserQuestion`, never wait for input, and never stop half way to
ask a preference. Where the original would ask, you state your recommendation
and move on; the human's answer arrives as the next round, not as a reply.

Do not set a model — clash pins one when it launches the session.

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

Number the issues and letter the options (`3b`), so the human can approve one by
name in their change request.

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

## Verdict

End the round with one explicit line: is this plan safe to implement as written,
safe with the recommended changes, or does it need rework before implementation?
Say which, and be willing to say the plan is fine.
