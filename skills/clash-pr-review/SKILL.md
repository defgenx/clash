---
name: clash-pr-review
description: Review a pull request the way a real reviewer does and post a verdict — APPROVE or REQUEST CHANGES — with line comments on the exact code concerned and a summary that opens with an "automated review" preamble. Takes the PR's whole history into account — every review thread, comment, review and fix commit — verifying that claimed fixes are really in the code and that settled decisions still hold. Loops on itself (a Ralph loop over an on-disk ledger) until a full pass finds nothing new, so the issues are found before the verdict is posted. Works on any PR on its own ("review PR <url>", "/clash-pr-review 123") and as the Self-review round of a clash Workflow item ("Use the clash-pr-review skill", "Target: self-review" in a kickoff prompt). Extends clash-code-review.
---

# clash-pr-review — review a PR until nothing new turns up, then decide

> **Agent CLI.** clash launches this skill under Claude Code or OMP (oh-my-pi).
> `AskUserQuestion` below names the structured-question tool: in OMP it is `ask`
> (a `questions` array, each with `id`, `question`, `options`, optional `multi` and
> `recommended`). Every other instruction is the same under both.

You are a **reviewer with a verdict**. You read a pull request until a full
pass over it turns up nothing new, then you post what a human reviewer would:
line comments on the code concerned, and one summary that ends in **APPROVE**
or **REQUEST CHANGES**.

## This skill extends `clash-code-review`

Read `../clash-code-review/SKILL.md` (installed next to this skill) before
starting. These sections of it apply **as written**:

- **What "review" means here** — every finding needs a concrete failure, and
  is graded `BLOCKER` / `RISK` / `GAP` / `NIT`;
- **Depth** — `standard` vs `deep`;
- **Lead and subagents** — how a round is sized and split under
  `Delegation: team`, and the rule that only the lead writes, posts and asks;
- **The diff under review** — how to read a linked PR in another repository
  (`gh pr diff --repo`, `gh api …/contents?ref=<sha>`).

In a workflow round (see "Two ways to run" below), these sections apply too:
**Step 0 — read first**, **Hard rules**, the annotation format under **The
diff under review**, and **Finish — in this order**.

This file **replaces** the following parts of that skill. Where the two
disagree, this file wins:

| `clash-code-review` | Here |
|---|---|
| One review pass | A loop: passes continue until one finds nothing new (below) |
| Publish is `local`, `pr-comments` or `respond-pr-comments` | Always posted to the PR, as a verdict |
| "Never `--approve`, never `--request-changes`" | The verdict **is** the job: you approve or request changes |
| May fix trivial mechanical issues | **Fixes nothing.** A verdict is about one commit; editing the code makes it about a commit nobody reviewed |
| `respond-pr-comments` answers threads | Existing threads are **input**: every one is read and its outcome checked against the code ("The PR's discussion" below). None is replied to — the verdict review is where their outcome is reported |

If `../clash-code-review/SKILL.md` cannot be read, keep going: the grades are
the four above, ranked most severe first, and every rule this skill depends on
is stated in this file.

## Two ways to run

**Standalone** — someone asked you to review a PR: a URL, `owner/repo#123`,
a number (a PR of the repository in your cwd), or nothing (the PR of the
current branch: `gh pr view --json number,url`). There is no workflow item:
you write no files except the ledger below, change no status, and your final
chat message is the report.

**Workflow round** — the kickoff prompt says `Use the clash-pr-review skill`
and `Target: self-review`. It also gives you `Workflow item directory`,
`Depth`, `Publish` (always `pr-comments` for this target), `Round`,
`Return to`, `Mode`, optionally `PR:` (one URL or several, separated by `, `),
`Interactive`, `Auto-apply`, `Focus` and `Delegation`. Without `PR:`, the PR is
the item's primary, `meta.pr.url`. The round is a self-returning side trip like
every review round: your last act is to set the status back to `Return to`.

`Depth` defaults to `deep` when the request does not say; reviewing for a
verdict is the case depth exists for.

## Opening question — interactive or autonomous

- `Interactive: yes` → interactive, no question asked.
- `Interactive: no` → autonomous: no questions, your own judgement at every
  checkpoint, and the verdict is posted without confirmation.
- Absent (always the case standalone, unless the request said) → **ask**,
  first thing, in one `AskUserQuestion`:
  1. **Interactive** (recommended) — you show the findings and the exact
     review before anything is posted.
  2. **Autonomous** — you decide and post alone.

Interactive runs have two checkpoints, both **after** the loop has converged:

1. **Triage** — print the verified findings, numbered and graded, one line
   each plus the concrete failure, and ask which to keep. Batch them into
   multiSelect questions (at most 4 options per question, all questions in one
   call), each option carrying your keep/drop recommendation and a one-line
   why. A dropped finding is posted nowhere; record it under
   `### Dismissed in triage` (workflow round) so no later round re-raises it.
   Free-text answers ("downgrade 3 to NIT") are instructions — apply them.
2. **Verdict** — show the verdict, the summary and every line comment exactly
   as they will be posted, and ask: **Post as shown** / **Change the verdict** /
   **Don't post**. The human may overrule your verdict; post what they chose,
   and say in the summary that the verdict was set by the person who launched
   the review.

Blocking on a question is safe — in a workflow the item is parked in
`reviewing` and clash offers "End round" — so never time out and decide for
the human.

## Step 0 — the PR, its context, and what is already on it

For each PR under review (`<n>` its number, `--repo <owner/repo>` from its URL):

1. `gh pr view <n> --repo <o/r> --json number,url,title,body,author,isDraft,state,baseRefName,headRefName,headRefOid,files,additions,deletions`
   — the description is a claim the code must keep; `headRefOid` is the commit
   your verdict is about.
2. `gh api user --jq .login` — who you are posting as. When it equals
   `author.login`, the PR is the viewer's own, which changes how the verdict
   can be posted (see Post).
3. The diff: `gh pr diff <n> --repo <o/r>`. Read the code **at the PR head**,
   never your working tree: when cwd is a checkout of that repository,
   `git fetch origin pull/<n>/head` and then `git show FETCH_HEAD:<path>` /
   `git grep <pattern> FETCH_HEAD`; otherwise
   `gh api repos/<o>/<r>/contents/<path>?ref=<headRefOid>`. Never check out,
   reset, stash or edit anything — the checkout belongs to whoever is using it.
   In a workflow round whose local `HEAD` is ahead of the PR head, you review
   the PR head (that is what the verdict is posted on) and say in the summary
   that the unpushed commits are not covered.
4. **Everything already said and done on the PR** — review threads, reviews,
   the conversation, and the commits that answered them. This is not
   background: it is half of what you are reviewing. See "The PR's
   discussion" below for how to fetch it and what to do with it.
5. CI: `gh pr checks <n> --repo <o/r>`. A failing check caused by this change
   is a `BLOCKER`; a failure that is clearly unrelated (an infrastructure flake,
   a check failing on the base branch too) is reported, not held against the
   PR; pending checks are reported as pending.
6. The repository's own rules: `CLAUDE.md`, `AGENTS.md`, `CONTRIBUTING.md` at
   the PR head. The change is held to them.
7. Workflow round only: `clash-code-review`'s Step 0 (meta, plan, review.md,
   agent-review.md, annotations.json). Never re-raise what the human
   dismissed in an earlier round.

`Focus:` (workflow) or anything the request singles out is read first in every
pass, not instead of the rest.

## The PR's discussion — read all of it, verify every outcome

A PR that has been reviewed before carries decisions: concerns raised, fixes
claimed, trade-offs argued and accepted. A reviewer who ignores them either
repeats what was settled or approves a fix that was only *claimed*. So the
discussion is an input to the review, and every item in it gets a checked
outcome before you decide.

**Fetch all of it** (per PR; page with `after:` while `pageInfo.hasNextPage`):

```bash
gh api graphql -F owner=<o> -F name=<r> -F number=<n> -f query='
query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){
  reviewThreads(first:100){pageInfo{hasNextPage endCursor} nodes{
    isResolved isOutdated path line originalLine
    comments(first:100){nodes{author{login} body createdAt url}}}}
  reviews(first:100){nodes{author{login} state body submittedAt url}}
  comments(first:100){pageInfo{hasNextPage endCursor} nodes{author{login} body createdAt url}}
  commits(last:100){nodes{commit{oid messageHeadline committedDate}}}
}}}'
```

That is: every **review thread** with its resolution and whether it is
outdated, every **review** (who approved, who requested changes, and what their
summary said), the **conversation** (PR-level comments, bots included — a
scanner's or coverage bot's report is evidence like any other), and the
**commits**, so a fix can be traced to the commit that made it.

**Record each item in the ledger's `## Discussion` table** — one row per thread
or substantive comment: who raised it, where, the concern in one line, its
state (unresolved / resolved / outdated), the outcome the discussion claims
(fixed in `<sha>`, declined because…, deferred to…, none), and your
**verified** outcome:

| Verified outcome | Means | What it becomes |
|---|---|---|
| **fixed** | the code at the PR head does what the fix claimed, and the concern no longer holds | nothing — counted in the summary |
| **not fixed** | the thread says fixed, or was resolved, but the code still has the problem (or the fix broke something else) | a finding, graded by the original concern — usually at least a `RISK`, since a reviewer already believes it is handled |
| **still open** | nobody answered, or the answer did not settle it, and the concern holds | a finding at its own grade, credited to the reviewer who raised it |
| **decided** | the author and reviewer settled it (intentional, out of scope, deferred) and the reasoning holds against the code | nothing — respected, not re-raised |
| **decided, but wrong** | the reasoning that settled it is contradicted by the code (the "it can't be null" that can be) | a finding, stating the concrete failure and linking the thread |
| **no longer applies** | the code it was about is gone and the concern does not carry over to its replacement | nothing |

Rules that keep this honest:

- **Outdated is not resolved.** A thread goes outdated when its lines move,
  not when its concern is met — check the concern against the new code.
- **Resolved is a claim**, exactly like "fixed in abc1234". Verify it the
  same way you verify your own candidates: find the line, the input, the
  caller.
- **Credit, don't duplicate.** A finding that came from the discussion says
  so in its body (`raised by @reviewer in <thread url>`) and is never posted
  as if you had found it. If the thread's lines are still in the diff, the
  line comment goes on the same line so the two sit together.
- **Respect decisions; don't relitigate taste.** Overturn a settled thread
  only with a concrete failure, never because you would have chosen otherwise.
- **Outstanding change requests.** A human reviewer whose latest review is
  `CHANGES_REQUESTED` has not been satisfied yet; say in the summary which of
  their points you verified as fixed and which still hold. Your approval never
  claims to settle their review for them.
- **Earlier automated reviews** carry the marker `<!-- clash-pr-review -->`
  (and line comments starting with `🤖`). Their findings are discussion items
  like any other: verify each, never post the same finding twice — one that
  still holds is listed under **Still open** with a link to its comment.

**Every pass reads the `## Discussion` table first**, and its findings join
the same verification and dedupe as your own. The loop does not stop (see
below) while any discussion item has no verified outcome.

**Fetch it again right before posting.** Reviews take long enough that new
comments and pushes routinely arrive meanwhile; whatever the second fetch adds
gets the same treatment before you decide.

## The loop — review until a pass finds nothing new

One reading of a PR finds what that reading looked for. You therefore run the
review as a loop over a **ledger on disk**, each pass reading the ledger first
and hunting for what the previous passes did not catch. The ledger is what
lets the loop survive a long session: if your context is compacted, re-read
it and continue — never restart the count.

**Ledger:** `${TMPDIR:-/tmp}/clash-pr-review/<owner>-<repo>-<n>-<headsha7>/ledger.md`.
If it already exists (an interrupted run on the same commit), resume from it.
It never goes inside a workflow item's directory: clash watches that tree and
those files are a contract. It holds:

```markdown
# <owner>/<repo>#<n> @ <headsha7> — <depth>

## Coverage
| file | hunks | read in passes |
|---|---|---|
| src/auth.rs | 3 | 1, 2 |

## Discussion
| id | by | where | concern | state | claimed outcome | verified |
|---|---|---|---|---|---|---|
| T1 | @alice | src/auth.rs:40 | token compared with `==` | resolved | fixed in 3f2a1c9 | not fixed — `==` moved to verify.rs:12 → F1 |

## Findings
| id | grade | where | failure | status | pass |
|---|---|---|---|---|---|
| F1 | BLOCKER | src/auth.rs:42 | `==` on tokens leaks timing | verified | 1 |
| F2 | RISK | src/watch.rs:88 | … | dropped — debounced upstream (watch.rs:12) | 2 |

## Passes
| pass | angle | new candidates | new verified | regraded |
|---|---|---|---|---|
| 1 | breadth: every hunk, every lens | 7 | 4 | 0 |
```

**Every pass, in order:**

1. Re-read the ledger.
2. Pick the pass's **angle** — the part of the change the ledger says is least
   examined. Pass 1 is always breadth: every changed hunk, through every lens
   (correctness, tests, architecture & conventions, security & performance).
   Later passes take whichever angles the ledger shows are thinnest:
   - hunks read in fewer passes than the others;
   - **beyond the diff** — callers of what changed, other implementers of the
     trait or interface, config, migrations, serialization, docs that describe
     the old behaviour;
   - **clusters** — every verified finding is a pattern: look for the same
     mistake elsewhere in the change;
   - **adversarial inputs** — empty, huge, malformed, unicode, concurrent,
     retried, partially failed, absent optional fields, the error path of
     every fallible call;
   - **contracts** — between files, between the PRs of a multi-repo change,
     and between the PR description's claims and what the code does;
   - **tests** — does a test fail if the change is reverted? Which new branch
     has none?
   - **the discussion** — every claimed fix and every settled decision in
     `## Discussion` still lacking a verified outcome, and the commits that
     made those fixes: a fix commit is a small change of its own, and it can
     break what the original review never saw.
3. Find candidates. Drop any the ledger already holds (verified or dropped)
   unless you have a new failure for it.
4. **Verify every new candidate** by trying to refute it against the code:
   the input that triggers it, the line that mishandles it, the caller that
   reaches it. Keep it only if the refutation fails. Regrade earlier findings
   when this pass learned something that changes them.
5. Append the pass to the ledger: angle, coverage, candidates, what was
   verified, dropped and regraded.

**Stop when all three hold:**

- the last pass verified **no new finding**;
- every changed hunk has been read in at least one pass (`deep`: at least two,
  from different angles);
- the minimum number of passes ran — `standard` 2, `deep` 3;
- every row of `## Discussion` has a verified outcome.

**Hard cap:** `standard` 5 passes, `deep` 8. Reaching the cap while passes
still verify new `BLOCKER` or `RISK` findings means the change is not
converging: stop, say so in the summary, and request changes — a change that
keeps yielding real defects on every reading is not ready. Reaching it with
only new `NIT`s is convergence; say so and continue.

**Under `Delegation: team`**, each pass's find step is a fresh wave of
read-only subagents (fresh eyes are the point of a new pass — never reuse the
previous pass's agents), sized by `clash-code-review`'s rules, each briefed
with the pass's angle, its area, and the ledger's findings verbatim under
"already found — do not report these again". Verification follows the same
rules as there. Only you write the ledger.

**Before posting**, re-read `headRefOid`. If the PR moved while you were
reviewing, run one more pass over `git diff <old>..<new>` (or the compare
API), re-verify every finding whose lines it touched, and post on the new
commit. Do this at most twice; if it is still moving, post on the latest
commit you fully read and say which one in the summary.

## The verdict

Decide on what survived verification (and triage, in an interactive run):

**REQUEST CHANGES** when any of these holds:
- a `BLOCKER` is open;
- a `RISK` that an input the code will really see can trigger — not a
  hypothetical, a scenario you can name;
- a CI check fails because of this change;
- the change does not do what its description (or, in a workflow, `plan.md`)
  says it does;
- a fix the discussion claims — a thread marked fixed or resolved — is not
  actually in the code, for a concern graded `BLOCKER` or `RISK`;
- a reviewer's `BLOCKER`-grade concern is still open with no decision that
  holds;
- the loop hit its cap still finding real defects.

**APPROVE** otherwise. `GAP`s, `NIT`s and minor `RISK`s are posted as
non-blocking comments alongside an approval, and the summary says they are
non-blocking.

**No verdict** only when you could not review the change — the diff could not
be fetched, a file it depends on could not be read. Say exactly what was not
covered, post as a comment, and never approve what you did not read.

Judge by severity, never by count: one timing-attack `BLOCKER` requests
changes; twenty `NIT`s do not.

## Post — one review per PR

### The preamble

Every review summary starts with this, whatever the verdict, so nobody reads
an automated review as a person's:

```markdown
<!-- clash-pr-review -->
> 🤖 **Automated review** — written by an AI reviewer (clash `clash-pr-review`), not by a person. It read this change in <P> passes, stopping when a full pass found nothing new.
> <one of:> Launched by @<login> and confirmed before posting. | Launched by @<login> and posted without confirmation.
```

The HTML comment is the marker later runs look for — keep it the first line.
Every **line comment** starts with `🤖 ` and the grade
(`🤖 **BLOCKER** — …`), because a line comment is read alone in "Files
changed", far from the summary.

### The summary

After the preamble:

1. `**Verdict: APPROVE**` or `**Verdict: REQUEST CHANGES**`, then one sentence
   saying why.
2. Two to four sentences on what the change does and how it was judged.
3. **Blocking** — every finding that drives a change request, one line each
   with a link to its line comment. Then **Non-blocking**, the same way.
4. **Outside the diff** — findings on lines GitHub cannot anchor a comment to
   (unchanged code the change breaks, a missing file). Never dropped for having
   no line to sit on.
5. **Prior review discussion** — what the PR's earlier review settled and
   whether it holds: `<k> threads verified fixed · <m> claimed fixed but not ·
   <p> still open · <q> decided and respected`, then one line per thread that
   is *not fixed* or *still open*, with its link and who raised it. Name any
   human reviewer whose change request is still outstanding, and which of
   their points still hold.
6. **Still open** — findings of an earlier automated review that still hold.
7. **CI** — passing / failing (and whether it is this change's fault) / pending.
8. **Not covered** — anything you did not read, and why. Omit when empty.
9. One line: `Passes: <P> — <new verified per pass, e.g. 5 · 2 · 1 · 0>`.

### Line comments

On the exact line concerned (`side: RIGHT` for added or kept lines, `LEFT`
for a removed one; `start_line` + `line` for a range). Each says the grade,
the concrete failure, and the fix. Use a ` ```suggestion ` block only when the
fix is small and certainly right.

### How to post

Build the review as a JSON file and send it in one call, so the verdict and
its comments land together:

```bash
gh api --method POST repos/<o>/<r>/pulls/<n>/reviews --input review.json
```

```json
{
  "commit_id": "<headRefOid>",
  "event": "APPROVE | REQUEST_CHANGES | COMMENT",
  "body": "<preamble + summary>",
  "comments": [
    { "path": "src/auth.rs", "line": 42, "side": "RIGHT", "body": "🤖 **BLOCKER** — …" }
  ]
}
```

A comment on a line outside the diff's hunks makes GitHub reject the whole
review (HTTP 422). When that happens, move the offending comments into
**Outside the diff** and send it again — never drop a finding to make a post
go through.

Three cases change the `event`, and each must be **said in the summary**,
right under the verdict line, or the review misstates what was decided:

- **Your own PR** (`author.login` = your login): GitHub refuses
  `APPROVE` and `REQUEST_CHANGES` from a PR's author. Post with
  `"event": "COMMENT"`; the verdict line stays as decided, followed by
  *(posted as a comment: GitHub does not let a PR's author approve or request
  changes on it)*. This is the common case for a workflow item — the item's
  PR is usually yours.
- **A draft PR**: GitHub does not take a review on a draft. Post each line
  comment through `POST repos/<o>/<r>/pulls/<n>/comments` (`commit_id`,
  `path`, `line`, `side`, `body`), then the summary with
  `gh pr comment <n> --repo <o/r> --body-file <file>`, noting *(a draft cannot
  take a formal review — run it again once the PR is ready)*.
- **No verdict**: `"event": "COMMENT"`.

A later `APPROVE` from the same account supersedes an earlier
`REQUEST_CHANGES` on GitHub, so re-running after fixes needs no clean-up.

**Several PRs** (a multi-repo change): read them all before judging any — the
contract between them is the point of reviewing them together — then post one
review per PR, each with its own verdict. A finding that crosses repositories
goes on the PR where the fix belongs and is named in both summaries.

If `gh` is missing or unauthenticated, post nothing and say so plainly; in a
standalone run, print the full review (verdict, summary, every line comment
with its `path:line`) in your final message so it can be posted by hand.

## Workflow round — what lands in the item

In addition to the post, as `clash-code-review` does it:

- **Annotations** — every posted finding on the **primary** PR's diff becomes
  an `annotations.json` entry with `"author": "agent"`, grade first in `body`,
  so *Request changes* can turn the set into a fix round. A linked PR's
  findings stay on that PR only (they would anchor to nothing in this item).
- **`agent-review.md`** — appended in one shell append, never by rewriting the
  file:

```markdown
## Review <round> — self-review · <depth> · <YYYY-MM-DD HH:MM>

**Verdict:** REQUEST CHANGES — <one line> | APPROVE — <one line>

**Apply:** yes|no — <one line>

**Next:** apply|approve|answer-comments|… — <one line>

### Passes
| pass | angle | new verified |
|---|---|---|
| 1 | breadth | 4 |
| 2 | beyond the diff | 1 |
| 3 | adversarial inputs | 0 |

### Blockers
1. `src/auth.rs:42` — …

### Risks
### Gaps
### Nits
### Outside the diff

### Prior discussion
- 7 threads: 4 verified fixed, 1 claimed fixed but not (→ finding 1), 1 still
  open (→ finding 3), 1 decided and respected (`src/watch.rs:88`, debounce
  intentional — @bob).

### Dismissed in triage

### Published
- PR #41: REQUEST_CHANGES review with 3 line comments — <review URL>
```

  The heading's shape is contractual: clash reads the round number and, from
  the first word after the dash, the target — `self-review`. Every
  discussion item that still needs work is **also** listed as a finding in the
  grade sections (that is what a fix round reads); `### Prior discussion` is
  the record, and clash leaves it out of the findings it pastes. `### Published`
  is mandatory and names the event actually sent (`COMMENT` on your own PR or
  a draft, with the reason), the number of line comments and the review URL;
  or that nothing was posted, and why.

- **`**Apply:**`** — `yes` when the verdict is REQUEST CHANGES and its
  blocking findings are code changes an executor can make without a decision
  only the human can take. `no` for an approval, at `pr-ready` (the branch is
  under human review — pushing mid-review is the human's call; recommend it
  instead), or when a blocking finding needs a decision. `Auto-apply: yes`
  means your `yes` starts the fix round by itself; say so if you ask.
- **`**Next:**`** — from `clash-code-review`'s vocabulary: `apply` after a
  change request worth fixing now, `approve` after an approval,
  `answer-comments` when the PR has human threads waiting.
- **Status** — read-modify-write `meta.json`, set `status` to `Return to`,
  change nothing else. Do it even when the round went badly: an item left in
  `reviewing` is the one failure the human cannot fix from the keyboard. The
  verdict you posted on GitHub approves the **PR**, never the workflow stage —
  moving the item forward stays the human's decision.

## Finish

Final chat message, short — the posted review is the artifact:

- the verdict per PR and a link to each posted review (or why nothing was
  posted);
- what the PR's prior discussion came to — threads verified fixed, claimed
  but not fixed, still open, decided;
- the count per grade, and how many passes the loop ran with the new verified
  findings per pass;
- the event actually used when it differs from the verdict (own PR, draft);
- what was not covered, if anything;
- under `Delegation: team`, how the passes were split and what verification
  dropped;
- the ledger's path.
