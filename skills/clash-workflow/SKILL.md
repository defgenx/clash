---
name: clash-workflow
description: Execute one phase of a clash Workflow item (plan | revise | implement | pr) in one of its entry modes (full | from-plan | review-only). Invoked by clash's GUI with a kickoff prompt naming the item directory, phase and mode. Reads meta.json/plan.md/review.md/annotations.json, does the work in the current worktree, addresses open diff annotations, and hands the item back to the human via a status transition. Triggers on "Use the clash-workflow skill", "Workflow item directory:", or when asked to run a clash workflow phase.
---

# clash-workflow — one pipeline phase per run

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
- **Interactive** — optional; see the opening question below

Your shell cwd is the item's git worktree. All code work happens there, on the
already-checked-out branch.

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
  task, ask about everything unclear, proceed only on the human's
  confirmation), then — before writing `plan.md` — present the 2–3 viable
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
2. `plan.md` — the current plan.
3. `review.md` — **top to bottom**: it is the accumulated decision history
   (`## Iteration N` sections with the human's notes + the open annotations
   at each round). Later sections override earlier ones.
4. `annotations.json` — every annotation with `"status": "open"` is work you
   MUST address this run (see phase `implement`). Ones with
   `"author": "agent"` came from a review round; treat them exactly like the
   human's.
5. `agent-review.md` — the review rounds run on this item, if any. Read the
   latest; its open findings are work.
6. The latest `history/<NNN>/diff.patch` if present — what the human last
   reviewed, useful context for what changed since.

## You are the executor, not the reviewer

Two separate skills run review rounds on the same item — `clash-plan-review`
(judges `plan.md`) and `clash-code-review` (judges the diff). They write
findings into `agent-review.md` and `annotations.json` and never touch
`plan.md` or the code beyond trivial fixes. Two consequences for you:

- `agent-review.md` is **input** for you — read the latest round before
  implementing or revising; its findings are the same kind of work as the human's
  annotations. Never write to it.
- `reviewing` is not a status you may write or work in. If you ever find the item
  in it, stop and say so — a reviewer is mid-round.

## Hard rules (violating these corrupts the pipeline)

- **Never** touch `history/`, `plan-history/`, or `iteration`, `reviewRound`,
  `appliedReviewKey` and `blueprint` in meta.json — clash owns all six.
  `blueprint.md` and `structure.md` belong to the explainer; read them freely
  (an accepted blueprint is the agreed shape of the work), write neither.
- Write `annotations.json` **only while** `meta.json.status` is
  `changes-requested` or `implementing`. During review phases the file
  belongs to the human's GUI.
- Every `meta.json` write is a **read-modify-write**: re-read the file, edit
  only your fields (`status`, `pr.url`, `linkedPrs` appends), keep every
  field you don't understand. Never rewrite it from a template.
- If your work opened PRs in **other repositories** (multi-repo work), record
  each one by appending `{"url": "<pr url>"}` to `meta.json.linkedPrs` (create
  the array if absent) — clash fills state/draft on its next refresh and shows
  them on the item. Never remove or reorder existing entries; unlinking is the
  human's action. Linked PRs never change this item's status — only `pr.url`
  (the primary) does.
- `review.md` is append-only history — read it, never edit it.
- Statuses you may write, and only these transitions:
  `planning → plan-review`, `changes-requested → plan-review` (plan-revision
  round), `changes-requested → implementing`,
  `implementing → diff-review | pr-draft`, and in phase `pr` only,
  `diff-review → pr-draft`.
- Git: commit your work on the current branch with clear conventional
  messages. **Never `--no-verify`** — if a hook fails, fix the cause or stop and
  explain in your final message. Pushing:
  - `full` / `from-plan` **without a PR**: never push — the branch is
    unpublished and publishing it is the human's call (creating the PR is
    what pushes it).
  - **Any mode with a PR** (`meta.pr.url` is set) or `review-only`: **push
    after committing** — plain `git push` (add `-u origin <branch>` if it has
    no upstream). The branch is published and the PR must reflect the fixes;
    a fix round that only commits locally leaves the PR silently stale.
    Never force-push, never rewrite published history; if the push is
    rejected, stop and report it instead of forcing.

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
   - Ask about everything you are not sure of (`AskUserQuestion`, as many
     rounds as it takes): expected behavior, scope boundaries, edge cases,
     constraints, what "done" looks like. If the title alone doesn't tell you
     what the feature *is*, say so and ask the human to describe it before
     anything else.
   - Do not explore the repo and do not write a line of `plan.md` until the
     human confirms your restated understanding — their answer is the
     confirmation, not your own confidence.
   - Open `plan.md` with the agreed understanding (goal, confirmed decisions,
     explicit non-goals) so reviewers and later rounds see what was agreed,
     not just what was planned.
   In autonomous runs nobody can answer: take the most conservative reading
   and open `plan.md` with an **Assumptions** section listing every call you
   made in place of a question.
2. Explore the repo as needed to ground the plan in real code.
3. Write/overwrite `plan.md`: a concrete implementation plan — context, the
   approach, files to touch, ordered steps, testing strategy, risks. Plain
   markdown; the GUI renders it.
4. Finish: set `meta.json.status = "plan-review"`. Stop — the human reviews.

## Phase: revise

Read the **latest** `## Iteration` section of `review.md` first.

- In `review-only` mode: behave exactly like phase `implement`. Stop reading
  this section.
- If the requested changes concern the **plan**: update `plan.md`
  accordingly and finish with `status = "plan-review"`.
  - **Edit it in place.** clash froze the previous plan as a version before
    launching you and shows the human a diff of what you changed, so keep every
    section the round did not challenge — wording included. A wholesale rewrite
    is indistinguishable from "everything changed" and makes that diff useless.
  - When the note reads `Apply agent review round N`, round N of
    `agent-review.md` **is** the work: address each of its findings, and where
    you disagree with one, say so in your summary rather than dropping it in
    silence.
- If they concern the **code** (there are open annotations, or the note
  references the diff): behave exactly like phase `implement`.

## Phase: implement

1. Set `meta.json.status = "implementing"` before you start.
2. Implement the plan (or the requested changes) in the worktree. Follow the
   repo's own CLAUDE.md conventions. Run the project's tests/linters.
3. **Address every open annotation**, one by one. Each is anchored to
   `file` + `line` with the annotated source line in `lineContent`. For each:
   - Make the change (or decide, with good reason, not to).
   - Update the annotation in `annotations.json`: set `"status"` to
     `"addressed"` (or `"wontfix"`) and append a reply
     `{"author": "agent", "body": "<one-line resolution or justification>", "createdAt": <epoch ms>}`
     to its `replies`. Keep all other fields intact.
   - Never delete an annotation and never touch ones already
     addressed/wontfixed by earlier rounds — nor `parked` ones (comments the
     human deliberately held back from this round).
4. Commit the work (small, reviewable commits are fine). If this item
   already has a PR (`meta.pr.url` is set), push (see Git above) so the PR
   picks up the fixes.
5. **`review-only`**: push the branch (see Git above), then finish at
   `"diff-review"` — you are done, skip steps 6–7.
6. **Only if the repo clearly works through PRs** — an existing `pr.url` on this
   item, or a repo whose recent history is merge commits from PRs — create the
   draft PR. When the kickoff prompt names a **PR skill**, invoke that skill to
   open it (it encodes the org's house style; if it is not available in this
   session, fall back to the `gh` path below and say so); otherwise
   `gh pr create --draft --title "<item title>" --body "<summary>"`. Either
   way, read-modify-write `meta.json` setting `pr.url` to the created URL
   (clash fills number/state on its next refresh). Otherwise **skip this**: a
   PR is not required to finish, the human approves the diff either way, and an
   unwanted PR is a chore for them to close. When in doubt, skip it and say so.
7. Finish: set `status = "diff-review"` — or `"pr-draft"` if you created the
   PR in step 6. Stop — the human reviews the diff in clash and either
   approves (which may close the item outright) or sends you a new round.

## Phase: pr

Write and open the draft PR for a diff that is already finished. You are here
because the human chose "let Claude Code write it" instead of clash's own
deterministic PR body — so the description is the deliverable, and it must be
better than a transcription of `plan.md` (which is what they declined).

**Do not change code in this phase.** No fixes, no refactors, no "while I'm
here". If you find something wrong, say so in your final message and leave it —
the human decides whether that becomes a new round.

1. Read the actual diff against the base branch (`git diff <base>...HEAD`, base
   from `meta.json.base`, else the repo's default branch) and the commit
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
   `gh pr create --draft --title "<title>" --body "<body>"` (add `--base` when
   `meta.json.base` is set).
5. Read-modify-write `meta.json`: set `pr.url` to the created URL and
   `status = "pr-draft"`. Leave `iteration` and `reviewRound` alone — those are
   clash's.

If the repo clearly does not work through PRs, stop before step 4 and say so
rather than opening one nobody wants.

## Tone of artifacts

`plan.md` is for a human reviewer: lead with the goal and the shape of the
change, keep steps scannable (`review-only` items have no `plan.md`).
Annotation replies are one-liners — what you did, not an essay. Your final
chat message should summarize what changed and which annotations were
addressed vs wontfixed (with the one-line reasons).
