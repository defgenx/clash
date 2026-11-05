---
name: clash-pr-review
description: Review a pull request the way a real reviewer does and post a verdict — APPROVE or REQUEST CHANGES — written the way a careful colleague writes one — short line comments on the exact code concerned, a summary shaped to the case, every finding carrying its grade and confidence, and a one-line automated-review disclosure. Takes the PR's whole history into account — every review thread, comment, review and fix commit — verifying that claimed fixes are really in the code and that settled decisions still hold. Loops on itself (a Ralph loop over an on-disk ledger) until a full pass finds nothing new, so the issues are found before the verdict is posted. Works on any PR on its own ("review PR <url>", "/clash-pr-review 123") and as the Self-review round of a clash Workflow item ("Use the clash-pr-review skill", "Target: self-review" in a kickoff prompt). Extends clash-code-review.
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

- **What "review" means here** — every finding needs a concrete failure, is
  graded `BLOCKER` / `RISK` / `GAP` / `NIT`, and carries a confidence
  (`high` / `medium` / `low`, by evidence) with its basis;
- **Depth** — `standard` vs `deep`;
- **Lead and subagents** — how a round is sized and split under
  `Delegation: team`, and the rule that only the lead writes, posts and asks;
- **The diff under review** — how to read a linked PR in another repository
  (`gh pr diff --repo`, `gh api …/contents?ref=<sha>`);
- **A PR stack** — each layer measured against its parent, sibling layers
  checked before anything is called missing, findings per PR. "PR stacks"
  below adds how to find the stack when nobody gave it to you.

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
`Stack:` (the item's PR stack, base → tip, separated by ` > `),
`Interactive`, `Auto-apply`, `Focus` and `Delegation`. Without `PR:`, the PR is
the item's primary, `meta.pr.url`. The round is a self-returning side trip like
every review round: your last act is to set the status back to `Return to`.

`Depth` defaults to `deep` when the request does not say; reviewing for a
verdict is the case depth exists for.

## Size — how hard to look, and with which model

A ten-line fix and a 2,000-line feature do not deserve the same review. The
**size** of the change scales the loop and the reviewers' model; `Depth` still
sets the rigor of each pass.

- **Workflow round:** the kickoff says `Size: small|medium|large (<N> lines,
  <F> files)`. clash measured the change and already chose the models at
  launch (a small change gets a lighter lead, a large one stronger
  subagents), and it forces them, so you never pass a `model` there.
- **Standalone:** measure it yourself — `additions + deletions` and
  `changedFiles` from `gh pr view`, summed over the layers you review. Small
  is ≤150 lines **and** ≤5 files; large is ≥800 lines **or** ≥25 files;
  medium otherwise; more than one PR is never small. Here nothing pins the
  subagents, so pick their model per call by size (the table).

| Size | Who reads | Subagent model (standalone) | Loop |
|---|---|---|---|
| small | you alone — a subagent costs more than the reading | none | the depth's minimum passes |
| medium | `clash-code-review`'s sizing rules under `Delegation: team` | `sonnet` | the depth's minimum passes |
| large | one fresh reviewer per area **every** pass, split by lens under `deep` | `opus` | one pass above the depth's minimum, and every hunk read twice |

Say the size and what it decided in your final message.

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

1. **Triage** — print the verified findings, numbered, graded and with their
   confidence, one line each plus the concrete failure, and ask which to keep. Batch them into
   multiSelect questions (at most 4 options per question, all questions in one
   call), each option carrying your keep/drop recommendation and a one-line
   why. A dropped finding is posted nowhere; record it under
   `### Dismissed in triage` (workflow round) so no later round re-raises it.
   Free-text answers ("downgrade 3 to NIT") are instructions — apply them.
2. **Verdict** — show the verdict, the summary and every line comment exactly
   as they will be posted, and ask: **Post as shown** / **Change the verdict** /
   **Don't post**. The human may overrule your verdict, or rewrite any
   comment; post what they chose (the disclosure's "confirmed before posting"
   already says a person signed off).

Blocking on a question is safe — in a workflow the item is parked in
`reviewing` and clash offers "End round" — so never time out and decide for
the human.

## Linking tickets

Link every ticket you mention (`PS-12345`, `ENG-42`) wherever you write it — documents, reports, annotations, PR text. The kickoff's `Ticket links:` field is the URL to use: replace `{key}` with the key, as in `[PS-12345](https://yourorg.atlassian.net/browse/PS-12345)`. `Ticket links: none` means mention keys unlinked. Without the field, **ask the human for the ticket host** the first time you need one (interactive) and tell them it can be saved in Settings → Workflows → Ticket links; in an autonomous run leave the key unlinked and say so in your report. Never guess a host. The field is for ticket keys only: an external document (a Confluence page, a Google doc, any page you have a URL for) is linked with its own full URL.

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
   the PR head (that is what the verdict is posted on) and say in your final
   message that the unpushed commits are not covered.
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
8. **The surroundings** — the services, repositories and libraries the
   changed code calls or is called by. List them now; how far you read them
   is decided below ("The surroundings").

`Focus:` (workflow) or anything the request singles out is read first in every
pass, not instead of the rest.

## The surroundings — read the other side before you call it a bug

A change rarely stands alone. It calls another service, consumes an SDK
built from another repository, publishes an event someone else reads, or
relies on a library behaving a certain way. Most false positives come from
guessing what the other side does: "this field can be null", "that endpoint
retries", "the consumer ignores unknown values". Read the other side instead.

**What counts.** From the diff and the files it touches, list every
dependency the changed lines reach across a boundary:

- another service it calls or is called by: HTTP / gRPC clients and handlers,
  queue or topic producers and consumers, webhooks, the gateway in front of it;
- a shared contract or SDK: generated clients, protobuf / OpenAPI packages,
  an internal gem / module / npm package, and the repository it is built from;
- a third-party library or API whose behaviour the change depends on (a retry
  policy, an error type, a rate limit, a default);
- infrastructure configuration the code assumes (environment variables, feature
  flags, deployment manifests), when it lives in another repository.

**How to read it.**

- **An organisation repository:** find it (the import path, the gem or module
  source, the package's `repository` field, `gh search code` / `gh search repos
  --owner <org>`), then read the code **at the version this change runs
  against** — the ref pinned in the lockfile, `go.mod`, `Gemfile.lock` or
  `package.json`, not the default branch —
  `gh api repos/<o>/<r>/contents/<path>?ref=<pinned>`. When the default branch
  differs from the pinned version in a way that matters, that is a finding of
  its own (deploy order, a pending bump). Read only the code on the path the
  change uses: the handler it calls, the message it consumes, the field it
  reads.
- **A third-party library or API:** its documentation or source at the pinned
  version (the package registry's docs, the library's own repository at that
  tag), with the URL in the ledger. A changelog entry for a recent bump is the
  first thing to read.
- **Nothing to read** (a private SaaS with no public docs, a repository you
  cannot access): say so in the ledger, and every finding that rests on it stays
  at `medium` or below, with "the other side was not read" as its basis.

**The rule it enforces.** A finding whose concrete failure depends on what
another service, repository or library does is **at most `medium` until that
side has been read**, and it reaches `high` only from the code or the docs,
never from naming. Reading the other side also kills findings: when it shows
the case cannot happen, drop the candidate and record why.

**Ask first, unless autonomous.** Reading the surroundings costs time and
tokens, and only the human knows which ones are worth it. In an interactive run,
right after the opening question (in the same call as the stack's layer
question, when there is one), ask in one `AskUserQuestion` with
`multiSelect: true` which to read, one option per dependency you listed, each
with what you would read and why (`api-contracts @ v0.11.0 — the
CLASSIFICATION enum this PR switches on`), the ones the change's risky paths
depend on recommended, plus **None — this repository only**. A dependency the
human leaves unticked is not read, and findings that rest on it are capped at
`medium` as above. When you list nothing, say so and skip the question.
Autonomous: read every dependency on a path a candidate finding or a risky
change (see *Own the verdict*) depends on, and skip the rest.

Record each one in the ledger's `## Surroundings` table — what it is, where you
read it (repo and ref, or URL), and what it established — and every later pass
reads that table before guessing about a dependency. Under
`Delegation: team`, one read-only subagent per dependency worth more than a
couple of reads.

## PR stacks — find the stack, review every layer

A PR whose base branch is another PR's head is one layer of a **stack**.
Reviewed alone against the trunk it is wrong twice: its diff contains the
layers below it, and what the layers above it add reads as missing.

**Finding the stack.** A workflow round's `Stack:` field is the stack. With
none — always the case standalone — find it yourself, from the PR you were
given (`--repo <o/r>` on every call):

1. `gh repo view <o/r> --json defaultBranchRef --jq .defaultBranchRef.name` —
   the trunk. A PR whose `baseRefName` is the trunk is the bottom layer.
2. **Down:** while the current PR's `baseRefName` is not the trunk,
   `gh pr list --repo <o/r> --head <baseRefName> --state all --json number,url,state,baseRefName,headRefName`.
   One open (or merged) PR is the parent; none means the base is a plain
   branch and the walk stops; several is ambiguous — stop and say so.
3. **Up:** `gh pr list --repo <o/r> --base <headRefName> --state open --json number,url,baseRefName,headRefName`
   from the tip. One PR is the child; several is a fork — the stack is
   unclear above this point, so stop there and say which PRs fork.
4. Stop after 10 layers either way. Closed (unmerged) PRs are never layers;
   a merged lower layer is history and is not reviewed again.

A workflow round does not walk further than `Stack:` — it is what clash
ordered from the PRs the human linked — but if GitHub shows a parent or child
the item does not track, say so in the report so it can be linked.

**Which layers.** Review every layer of the stack unless `PR:` (workflow) or
the request (standalone) names some. Interactive: right after the opening
question, ask in one `AskUserQuestion` with `multiSelect: true` —
*Whole stack* (recommended), then one option per layer
(`#42 — layer 2/3, on #41`) — with the PR you were given, or the `PR:` set,
pre-recommended when it is not the whole stack. Autonomous: the whole stack.

**One loop, one ledger section and one verdict per layer.** Run the loop below
over all chosen layers together (a pass reads every layer, because a finding
on one is checked against the others), keep each layer's findings under its
own `# <owner>/<repo>#<n>` heading in the ledger, and post one review on each
PR, in stack order. A layer's verdict is about that layer: never request
changes on it for what a higher layer delivers, and do request them when it
cannot merge safely without that layer (see `clash-code-review` → "A PR
stack"). A summary may name the layer (`Layer 2/3 of #41 > #42 > #43.`) in its
verdict sentence; nothing more about the stack goes on the PR.

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
| **fixed** | the code at the PR head does what the fix claimed, and the concern no longer holds | nothing — recorded in the ledger |
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
- **Don't duplicate.** A finding the discussion already raised is posted once,
  as what to change — if its thread's lines are still in the diff, the line
  comment goes on the same line so the two sit together. Who raised it and
  what was said is recorded in the ledger, not repeated on the PR.
- **Respect decisions; don't relitigate taste.** Overturn a settled thread
  only with a concrete failure, never because you would have chosen otherwise.
- **Outstanding change requests.** A human reviewer whose latest review is
  `CHANGES_REQUESTED` has not been satisfied yet: their points that still hold
  are blocking items like any other. Your approval never claims to
  settle their review for them.
- **Earlier automated reviews** carry the marker `<!-- clash-pr-review -->`;
  their line comments carry `<!-- clash-pr-review F<n> -->` (older runs: a
  leading `🤖 **GRADE**`). Their findings are discussion items like any other:
  verify each, and never post the same line comment twice — one that still
  holds is a blocking item in the summary, linked to its existing comment,
  instead of a new one.

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
# <owner>/<repo>#<n> @ <headsha7> — <depth> · <size>

## Coverage
| file | hunks | read in passes |
|---|---|---|
| src/auth.rs | 3 | 1, 2 |

## Surroundings
| dependency | kind | read at | established |
|---|---|---|---|
| Hivebrite/api-contracts | SDK (gem) | v0.11.0 `proto/audit/v1/log.proto` | `classification` is a closed enum; unknown values decode as 0 |

## Discussion
| id | by | where | concern | state | claimed outcome | verified |
|---|---|---|---|---|---|---|
| T1 | @alice | src/auth.rs:40 | token compared with `==` | resolved | fixed in 3f2a1c9 | not fixed — `==` moved to verify.rs:12 → F1 |

## Findings
| id | grade | confidence | where | failure | status | pass |
|---|---|---|---|---|---|---|
| F1 | BLOCKER | high — traced both callers | src/auth.rs:42 | `==` on tokens leaks timing | verified | 1 |
| F2 | RISK | — | src/watch.rs:88 | … | dropped — debounced upstream (watch.rs:12) | 2 |

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
   - **the surroundings** — every candidate that rests on another service,
     repository or library, checked against the code or docs listed in
     `## Surroundings` (reading the ones it is missing, within what the human
     chose);
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
   reaches it. Keep it only if the refutation fails, and record its
   confidence: `high` once the refutation read the exact path. Regrade
   earlier findings — and raise their confidence — when this pass learned
   something that changes them.
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
converging: stop and request changes — a change that keeps yielding real
defects on every reading is not ready; its findings are the blocking
list, and the non-convergence goes in the ledger and your final message, not on
the PR. Reaching the cap with only new `NIT`s is convergence; continue.

**Under `Delegation: team`**, each pass's find step is a fresh wave of
read-only subagents (fresh eyes are the point of a new pass — never reuse the
previous pass's agents), sized by the change's **Size** (above) and
`clash-code-review`'s rules, each briefed
with the pass's angle, its area, and the ledger's findings verbatim under
"already found — do not report these again". Verification follows the same
rules as there. Only you write the ledger.

**Before posting**, re-read `headRefOid`. If the PR moved while you were
reviewing, run one more pass over `git diff <old>..<new>` (or the compare
API), re-verify every finding whose lines it touched, and post on the new
commit. Do this at most twice; if it is still moving, post on the latest
commit you fully read — `commit_id` pins the review to it.

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
- the change's core behaviour has no test that would fail if it were
  reverted;
- the loop hit its cap still finding real defects.

**APPROVE** when none of the above holds and you can answer every question
under *Own the verdict* below. `GAP`s, `NIT`s and minor `RISK`s are posted as non-blocking
comments alongside an approval, and the summary says they are non-blocking.

**COMMENT** — neither — when the review cannot honestly end in either:
- you could not review all of it (the diff could not be fetched, a file it
  depends on could not be read) — never approve what you did not read;
- what is open is a **question only the author can answer** (which of two
  behaviours is intended, whether a trade-off was deliberate), so you can
  neither vouch for the change nor name a defect to fix.

Say which of the two it is and exactly what is uncovered or unanswered. This
is a real third outcome, not a hedge: when the code gives you a concrete
failure, request changes; when it gives you none and you read it all,
approve.

Judge by severity, never by count: one timing-attack `BLOCKER` requests
changes; twenty `NIT`s do not.

Only a `high` or `medium` confidence finding drives REQUEST CHANGES. A `low`
one is either verified in another pass or posted as a non-blocking
`question:` with what would confirm it — never the sole reason for a verdict.

### Own the verdict

Both verdicts have consequences for a real person. An approval puts this
account's name on the change: if it breaks production, "the review approved
it" is part of the story. A change request blocks someone's work until they
answer it. Decide as if you will be the one asked to explain the verdict —
because the account holder will be.

**Before approving**, answer these from the ledger, not from memory:

1. **What is the worst this change can do once merged?** Name it: data lost or
   corrupted, a wrong permission granted, money or messages sent twice, an
   outage, a public API or schema broken for existing clients, a migration
   that cannot be rolled back.
2. **Did you read the code that prevents it?** For any change touching
   authentication or permissions, persisted data or migrations, payments,
   deletion, concurrency, or a contract other services consume, the paths that
   matter must be read at the PR head with `high` confidence. If they are not,
   you have not earned the approval: read them, or post `COMMENT` saying what
   you did not cover.
3. **Is every open `RISK` acceptable for a reason you can state?** "It only
   fires under X, and X is guarded by Y (link)" is a reason. "It's probably
   fine" is not; that `RISK` is still open.
4. **Does it do what the description says, and is it tested where it
   matters?** When no test would fail if the change's core behaviour were
   reverted, that `GAP` blocks: ask for the test. Other `GAP`s stay
   non-blocking.

An approval on a risky change says in its one line what you checked that
matters ("Went through the migration's rollback and both writers of
`balance`, all good."), so a reader knows what the approval covers. On a
trivial change, a short plain sentence is enough ("Looks good to me.").

**Before requesting changes**, check every blocking item:

- it is a defect, verified at `high` or `medium` confidence, with a concrete
  failure — not a preference, not a `NIT`, not a `low` hunch;
- it belongs to this PR: something the change introduces or makes worse. A
  pre-existing problem it merely sits next to is a non-blocking comment or a
  follow-up ticket, never a reason to hold this PR;
- the author can act on it: say what would make you approve ("once the retry
  is idempotent, this is good to go"). A change request with no way out is a
  veto, not a review;
- you are not blocking for the same point twice: when the author answered it
  in the discussion and the answer holds, it is settled.

Hold a few blocking points, not a long list of mixed ones: when everything
blocks, nothing reads as important. If there are many real blockers, say the
change needs another pass and lead with the ones that matter most.

**When in doubt between the two, don't pick the comfortable one.** Approving
to be nice and requesting changes to be safe are both ways of not deciding.
If the code gives you a concrete failure, request changes; if you read what
matters and it holds, approve; if you can do neither honestly, `COMMENT` and
say what is missing. In an interactive run, show the human your answers to
the questions above at the verdict checkpoint, so they sign off on the
reasoning and not just the word.

## Post — one review per PR, written like a colleague wrote it

The review goes out under a person's account and is read by people who work
with them. It must read like a careful colleague's review: short, specific,
in plain words. Whatever the loop found, a review that reads like a form is
skimmed, and a skimmed review is one nobody acts on.

### How to write

These rules come from how experienced reviewers actually write on PRs. They
apply to the summary, every line comment and every reply:

- **One comment, one point.** Most line comments are one to three
  sentences. If it needs a paragraph, the point is not clear yet.
- **Say the problem first, in the reader's terms**: what goes wrong, for whom,
  when. "This panics when `items` is empty — `load()` returns `[]` on a 404."
  Not a label, not a `Failure:` / `Fix:` scaffold, not a bold grade as the
  opening word.
- **First person, plain words.** "I think", "I'd", "not sure this…", "we"
  for the team's code. Contractions are fine. Write in the language the PR's
  description and discussion use.
- **Unsure? Ask.** A finding you hold at `low` confidence is a question,
  not a statement: "Can `user` be nil here? `find_by` returns nil when the
  invite was revoked." A question invites the author's knowledge; a confident
  wrong claim costs a round of argument.
- **Point to precedent.** "We already do this in `<permalink>`, could we
  reuse it?" persuades better than a rule. When the repository's own docs
  (`CLAUDE.md`, `CONTRIBUTING.md`) settle it, link the line.
- **Suggest the change, don't lecture.** Name the fix in one sentence, or a
  ` ```suggestion ` block when it is small and certainly right (it must apply
  cleanly to the commented lines), or a short sketch when it is neither.
- **Nothing that isn't for the author.** No praise padding, no "Overall…",
  no recap of what the PR does, no description of how the review ran, no
  pass counts. Those live in the ledger and the item's report.
- **Never "LGTM"**, in any case or form (`lgtm`, `LGTM!`, `LGTM 👍`), in the
  summary, a line comment or a reply. Say it in words.
- **No tells of generated text.** Avoid "It's worth noting", "Consider…",
  "This ensures", "robust", "comprehensive", "Additionally", lists of three
  for rhythm, em-dash chains, bold on every other phrase, headings inside a
  line comment. Use formatting only where it means something: code, links,
  a suggestion block.

### How to point at an issue

- **Anchor where the fix goes**, on the smallest range that shows it
  (`start_line` + `line`). If the symptom shows elsewhere, comment at the
  cause and link the symptom, not the other way round.
- **Make the failure concrete**: the input, the caller, the sequence. "A
  retried webhook runs this twice and creates two invoices", not "this is not
  idempotent".
- **Link code with permalinks pinned to the head commit** —
  `https://github.com/<o>/<r>/blob/<headRefOid>/<path>#L<a>-L<b>`. GitHub
  renders a permalink on its own line as a code snippet, so the reader sees the
  caller, the precedent or the other copy without leaving the thread. Never
  link a branch name: the lines move under it.
- **Every reference is a link.** A ticket, another PR or issue, a commit, a
  doc, an RFC, a library's changelog — whatever a comment or the summary
  names, the reader can click it:
  - Jira key → `<base>/browse/PS-1234`, the base taken from a ticket link
    already in the PR, its branch or description, or `workflows.jira_base_url`;
    Linear → the issue URL the PR or the Linear integration comment carries;
  - PRs, issues and commits → `owner/repo#123` and full shas autolink on
    GitHub; anything in another repo gets its full URL;
  - code → a permalink (above); docs and changelogs → the URL at the version
    the change pins.
  Never guess a URL. If you cannot find one, write the reference plainly and
  list it under *not linked* in the Finish report.
- **One mistake, one comment.** The same problem in several places is one
  comment on the first occurrence ("same thing at <permalink>, <permalink>"),
  never N copies.
- **Outside the diff** — a line GitHub cannot anchor to (unchanged code the
  change breaks, a missing file) goes in the summary as a permalink with the
  sentence that explains it. Never dropped for having no line to sit on.
- **Prefix only what changes the reader's next move**, the way human reviewers
  label comments ([Conventional Comments](https://conventionalcomments.org)):
  `blocking:` on what must change before merge, `nit:` on what doesn't
  matter, `question:` on what you are asking. Everything else needs no prefix.

### Every finding keeps its grade and its confidence

The prose is for people; the grade and the confidence are what clash and the
next run read, and they are **never** left out. Every line comment ends with
one small footer line:

```markdown
<sub>🤖 blocker · confidence high — traced both callers of `check_token`</sub> <!-- clash-pr-review F1 -->
```

- the grade in lower case (`blocker` / `risk` / `gap` / `nit`), the
  confidence (`high` / `medium` / `low`), and its basis — for `medium` and
  `low`, what would raise it;
- the hidden marker with the ledger id is how later runs recognise their own
  comments. (Runs before this format marked them with a leading `🤖 **GRADE**`;
  treat both as earlier automated comments.)

In the summary, every item that names a finding ends with its grade and
confidence in the same words: `— blocker, high confidence`.

### Pick the summary's shape from the case

There is no fixed template. Choose the shape that fits what this review has
to say, and leave out every part that would be empty:

| Case | What the summary says |
|---|---|
| Approve, nothing to change | One line. A short plain sentence on a trivial change ("Looks good to me."); on a risky one, what you checked that matters ("Went through the migration's rollback, all good."). Never "LGTM". Nothing else. |
| Approve, minor things inline | One line: approving, the inline comments are small and none blocks. Mention a point only when it is outside the diff. |
| Request changes, one blocker | Say the blocker in a sentence or two with its link and what would make you approve, then "the rest is inline and minor" if there is a rest. |
| Request changes, several | One sentence, then a short numbered list of what must change, each item one line with its link. Non-blocking items stay inline; add a second short list only for those outside the diff. |
| A question only the author can answer (`COMMENT`) | Ask it directly, and say what each answer would mean: "Is the fallback meant to swallow 5xx? If yes, a comment says so; if not, it should re-raise." |
| Couldn't review all of it (`COMMENT`) | What you covered, what you could not read and why. Never approve what you did not read. |
| Re-review after fixes | What you re-checked, briefly: "Checked the fixes — the timeout and the nil case are good. The retry still double-charges (link)." This is the one case where earlier discussion belongs on the PR. |
| Your own PR | Notes to yourself and your reviewers: what you would fix before asking for review, in the same voice. Posted as `COMMENT` (see below). |
| A stack layer | Start with where it sits: "Layer 2/3, reviewed against #41." Then the shape of whichever case above applies. |
| A draft | "Early pass on the draft:" then the case above. |
| Generated code, lockfiles, a pure dependency bump | Say what you checked (changelog, breaking notes, the call sites that matter) in one or two lines. |

Use headings (`**Blocking**`, `**Smaller things**`) only when there are
enough items that a reader needs them to find their way, roughly more than
five. Each review has one disclosure line, **last**, in small text:

```markdown
<!-- clash-pr-review -->
…the summary…

<sub>🤖 Automated review by clash (`clash-pr-review`), launched by @<login> — <confirmed before posting | posted without confirmation>.</sub>
```

The hidden marker stays the **first line** (later runs look for it). The
disclosure is not optional: the account is a person's, and a reader must be
able to tell this review was not written by them.

### Check before posting — every time

A review missing a confidence, or a finding the ledger holds but the post
dropped, is a defect in the review. Before sending, build `review.json` (below)
and run:

```bash
jq -e '
  ([.comments[].body | select(test("<!-- clash-pr-review F[0-9]+ -->") | not)] | length == 0)
  and ([.comments[].body | select(test("🤖 (blocker|risk|gap|nit) · confidence (high|medium|low) — .+") | not)] | length == 0)
  and (.body | startswith("<!-- clash-pr-review -->"))
  and (.body | test("<sub>🤖 Automated review by clash"))
  and ([.body, .comments[].body] | map(test("\\blgtm\\b"; "i")) | any | not)
' review.json
```

Then check the summary by eye: every finding it names ends with
`— <grade>, <level> confidence`, and every finding the ledger marks for
posting is either a line comment or a summary item, once. Fix and re-check
until both pass. Never post to get past the check.

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
  "body": "<!-- clash-pr-review -->\n<summary>\n\n<sub>🤖 Automated review by clash …</sub>",
  "comments": [
    { "path": "src/auth.rs", "start_line": 40, "line": 42, "side": "RIGHT",
      "body": "blocking: tokens are compared with `==`, which returns early on the first differing byte, so response time leaks how much of a guessed token is right. `subtle::ConstantTimeEq` is already a dependency (we use it in <permalink>).\n\n<sub>🤖 blocker · confidence high — traced both callers of `check_token`</sub> <!-- clash-pr-review F1 -->" }
  ]
}
```

`side` is `RIGHT` for added or kept lines, `LEFT` for a removed one; drop
`start_line` for a single line. A comment on a line outside the diff's hunks
makes GitHub reject the whole review (HTTP 422). When that happens, move the
offending comments into the summary as outside-the-diff items and send it
again — never drop a finding to make a post go through.

Three cases change the `event`, and each must be **said in the summary**
in a short sentence, or the review misstates what was decided:

- **Your own PR** (`author.login` = your login): GitHub refuses
  `APPROVE` and `REQUEST_CHANGES` from a PR's author — server-side, for every
  client and token of that account (HTTP 422), so there is nothing to retry and
  no flag to try; only a different account can leave a green check. Post with
  `"event": "COMMENT"` and say what the verdict would have been: "I'd request
  changes on this if I could — GitHub doesn't let an author do that on their
  own PR." This is the common case for a workflow item — the item's PR is
  usually yours.
- **A draft PR**: GitHub does not take a review on a draft. Post each line
  comment through `POST repos/<o>/<r>/pulls/<n>/comments` (`commit_id`,
  `path`, `line`, `side`, `body`), then the summary with
  `gh pr comment <n> --repo <o/r> --body-file <file>`, and say it is a draft
  pass to run again once the PR is ready. The pre-post check applies to each
  comment body all the same.
- **A COMMENT verdict**: `"event": "COMMENT"` — nothing to restate, the event
  is the verdict.

A later `APPROVE` from the same account supersedes an earlier
`REQUEST_CHANGES` on GitHub, so re-running after fixes needs no clean-up.

**Several PRs** (a multi-repo change): read them all before judging any — the
contract between them is the point of reviewing them together — then post one
review per PR, each with its own verdict. A finding that crosses repositories
goes on the PR where the fix belongs, and the other PR's summary links it.

If `gh` is missing or unauthenticated, post nothing and say so plainly; in a
standalone run, print the full review (summary, every line comment with its
`path:line`) in your final message so it can be posted by hand.

## Workflow round — what lands in the item

In addition to the post, as `clash-code-review` does it:

- **Annotations** — every posted finding on the **primary** PR's diff becomes
  an `annotations.json` entry with `"author": "agent"`, grade first in `body`
  and `confidence` / `confidenceBasis` set, so *Request changes* can turn the
  set into a fix round. The file is one
  object, `{"annotations": [ … ]}` — append inside that array, never write a
  bare array. A linked PR's
  findings stay on that PR only (they would anchor to nothing in this item).
- **`agent-review.md`** — appended in one shell append, never by rewriting the
  file:

```markdown
## Review <round> — self-review · <depth> · <YYYY-MM-DD HH:MM>

**Verdict:** REQUEST CHANGES | APPROVE | COMMENT — <one line>

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
   **Confidence:** high — <the evidence>

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
- the count per grade and per confidence, and how many passes the loop ran
  with the new verified findings per pass;
- the size, and the models and fan-out it decided;
- which surroundings were read (repo@ref or URL) and which were not, and the
  findings capped at `medium` because of it;
- the event actually used when it differs from the verdict (own PR, draft);
- what was not covered, if anything;
- *not linked*: any ticket or reference the review names without a link,
  because no URL could be found;
- under `Delegation: team`, how the passes were split and what verification
  dropped;
- the ledger's path.
