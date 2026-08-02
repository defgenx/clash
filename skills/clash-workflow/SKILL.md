---
name: clash-workflow
description: Execute one phase of a clash Workflow item (plan | revise | implement) in one of its entry modes (full | from-plan | review-only). Invoked by clash's GUI with a kickoff prompt naming the item directory, phase and mode. Reads meta.json/plan.md/review.md/annotations.json, does the work in the current worktree, addresses open diff annotations, and hands the item back to the human via a status transition. Triggers on "Use the clash-workflow skill", "Workflow item directory:", or when asked to run a clash workflow phase.
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
- **Phase** — `plan` | `revise` | `implement`
- **Mode** — `full` | `from-plan` | `review-only` (also in `meta.json.mode`;
  a missing value means `full`)

Your shell cwd is the item's git worktree. All code work happens there, on the
already-checked-out branch.

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

1. `meta.json` — mode, status, iteration, branch/base, PR info. Parse
   leniently.
2. `plan.md` — the current plan.
3. `review.md` — **top to bottom**: it is the accumulated decision history
   (`## Iteration N` sections with the human's notes + the open annotations
   at each round). Later sections override earlier ones.
4. `annotations.json` — every annotation with `"status": "open"` is work you
   MUST address this run (see phase `implement`).
5. The latest `history/<NNN>/diff.patch` if present — what the human last
   reviewed, useful context for what changed since.

## Hard rules (violating these corrupts the pipeline)

- **Never** touch `history/` and **never** change `iteration` in meta.json —
  clash owns both.
- Write `annotations.json` **only while** `meta.json.status` is
  `changes-requested` or `implementing`. During review phases the file
  belongs to the human's GUI.
- Every `meta.json` write is a **read-modify-write**: re-read the file, edit
  only your fields (`status`, `pr.url`), keep every field you don't
  understand. Never rewrite it from a template.
- `review.md` is append-only history — read it, never edit it.
- Statuses you may write, and only these transitions:
  `planning → plan-review`, `changes-requested → plan-review` (plan-revision
  round), `changes-requested → implementing`,
  `implementing → diff-review | pr-draft`.
- Git: commit your work on the current branch with clear conventional
  messages. **Never `--no-verify`** — if a hook fails, fix the cause or stop and
  explain in your final message. Pushing:
  - `full` / `from-plan`: **never push** unless creating the PR requires it
    (`gh pr create` pushes the branch).
  - `review-only`: **push after committing** — plain `git push` (add
    `-u origin <branch>` if it has no upstream), so the PR under review picks
    up the fixes. Never force-push, never rewrite published history; if the
    push is rejected, stop and report it instead of forcing.

## Phase: plan

Never runs in `review-only` mode. If you are somehow asked for it there, stop
and say so instead of writing a plan.

1. Explore the repo as needed to ground the plan in real code.
2. Write/overwrite `plan.md`: a concrete implementation plan — context, the
   approach, files to touch, ordered steps, testing strategy, risks. Plain
   markdown; the GUI renders it.
3. Finish: set `meta.json.status = "plan-review"`. Stop — the human reviews.

## Phase: revise

Read the **latest** `## Iteration` section of `review.md` first.

- In `review-only` mode: behave exactly like phase `implement`. Stop reading
  this section.
- If the requested changes concern the **plan**: update `plan.md`
  accordingly and finish with `status = "plan-review"`.
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
     addressed/wontfixed by earlier rounds.
4. Commit the work (small, reviewable commits are fine).
5. **`review-only`**: push the branch (see Git above), then finish at
   `"diff-review"` — you are done, skip steps 6–7.
6. Optional but preferred when the work is complete: create the draft PR —
   `gh pr create --draft --title "<item title>" --body "<summary>"` — then
   read-modify-write `meta.json` setting `pr.url` to the created URL (clash
   fills number/state on its next refresh).
7. Finish: set `status = "diff-review"` — or `"pr-draft"` if you created the
   PR in step 6. Stop — the human reviews the diff in clash and either
   approves or sends you a new round.

## Tone of artifacts

`plan.md` is for a human reviewer: lead with the goal and the shape of the
change, keep steps scannable (`review-only` items have no `plan.md`).
Annotation replies are one-liners — what you did, not an essay. Your final
chat message should summarize what changed and which annotations were
addressed vs wontfixed (with the one-line reasons).
