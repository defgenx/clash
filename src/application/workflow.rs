//! Pure workflow logic — no IO, unit-tested directly.
//!
//! Home of everything the Workflows feature computes without touching the
//! filesystem: slugs, line hashing, annotation re-anchoring (over
//! `application::diff`'s structural parse), attention/notification
//! transition detection, and the agent kickoff prompt.

use std::collections::HashMap;

use crate::application::diff::{FileDiff, HunkLine};
use crate::domain::workflow::{
    AgentReviewSummary, Annotation, DiffSide, WorkflowItem, WorkflowMode, WorkflowStatus,
};

// ── Slugs ───────────────────────────────────────────────────────────────

/// Turn a human title into a filesystem-safe item slug: lowercase ASCII
/// alphanumerics, runs of anything else collapse to a single `-`, trimmed.
/// Never emits path separators or `..`, and never returns an empty string
/// (falls back to `"item"`).
///
/// Colons are excluded by construction — the GUI embeds `project/slug` in
/// `view:workflow:<project>/<slug>` tab keys, where a `:` would break the
/// tab-owner parsing.
pub fn slugify(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut last_dash = true; // suppress leading dashes
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "item".to_string()
    } else {
        out
    }
}

// ── Line hashing ────────────────────────────────────────────────────────

/// FNV-1a 64-bit over the *trimmed* line text, rendered as lowercase hex.
///
/// Trimming makes the hash robust to indentation changes and CRLF/LF
/// differences (`\r` is whitespace). Hand-rolled because clash has no hashing
/// crate and stability across versions matters more than speed here.
pub fn line_hash(line: &str) -> String {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    for b in line.trim().bytes() {
        hash ^= b as u64;
        hash = hash.wrapping_mul(PRIME);
    }
    format!("{:016x}", hash)
}

// ── Annotation anchoring ────────────────────────────────────────────────

/// An annotation resolved against the *current* diff.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnchoredAnnotation {
    pub annotation: Annotation,
    /// Line number (on the annotation's side) in the current diff, when the
    /// anchor still resolves. `None` when orphaned.
    pub current_line: Option<u32>,
    /// The file the annotation resolves to in the current diff (follows
    /// renames). Falls back to the stored file when orphaned.
    pub current_file: String,
    /// True when the anchor no longer exists in the current diff. The GUI
    /// renders these in a per-file "unanchored" tray — never dropped.
    pub orphaned: bool,
}

/// Line number of `rec` on the given diff side, if it exists there.
fn side_line_no(rec: &HunkLine, side: DiffSide) -> Option<u32> {
    match side {
        DiffSide::Old => rec.old_no,
        DiffSide::New => rec.new_no,
    }
}

/// Resolve every annotation against the current diff.
///
/// Match order per annotation:
/// 1. exact `file + side + line` whose trimmed-content hash still matches;
/// 2. content-hash search within the same file (renames followed via
///    `rename from` headers), nearest to the stored line wins;
/// 3. no match → `orphaned: true`.
///
/// Annotations with an empty hash (hand-written files) get one computed from
/// `line_content` when available; with neither, only an exact line-number
/// match can anchor them.
pub fn anchor_annotations(
    files: &[FileDiff],
    annotations: &[Annotation],
) -> Vec<AnchoredAnnotation> {
    annotations
        .iter()
        .map(|ann| anchor_one(files, ann))
        .collect()
}

fn anchor_one(files: &[FileDiff], ann: &Annotation) -> AnchoredAnnotation {
    let hash = if !ann.line_content_hash.is_empty() {
        ann.line_content_hash.clone()
    } else if !ann.line_content.is_empty() {
        line_hash(&ann.line_content)
    } else {
        String::new()
    };

    // The annotated file in the current diff: direct path match first, then
    // follow a rename (old annotated path == `rename from`).
    let file = files
        .iter()
        .find(|f| f.path() == ann.file || f.old_path == ann.file)
        .or_else(|| {
            files
                .iter()
                .find(|f| f.renamed_from.as_deref() == Some(ann.file.as_str()))
        });

    let Some(file) = file else {
        return AnchoredAnnotation {
            annotation: ann.clone(),
            current_line: None,
            current_file: ann.file.clone(),
            orphaned: true,
        };
    };

    let lines = file.hunks.iter().flat_map(|h| h.lines.iter());

    // Pass 1: exact position, content still matching (or no hash to check).
    for rec in lines.clone() {
        if side_line_no(rec, ann.side) == Some(ann.line) {
            let content_ok = hash.is_empty() || line_hash(&rec.text) == hash;
            if content_ok {
                return AnchoredAnnotation {
                    annotation: ann.clone(),
                    current_line: Some(ann.line),
                    current_file: file.path().to_string(),
                    orphaned: false,
                };
            }
            break;
        }
    }

    // Pass 2: content search, nearest to the original line.
    if !hash.is_empty() {
        let mut best: Option<u32> = None;
        for rec in lines {
            let Some(no) = side_line_no(rec, ann.side) else {
                continue;
            };
            if line_hash(&rec.text) != hash {
                continue;
            }
            let better = match best {
                None => true,
                Some(b) => no.abs_diff(ann.line) < b.abs_diff(ann.line),
            };
            if better {
                best = Some(no);
            }
        }
        if let Some(no) = best {
            return AnchoredAnnotation {
                annotation: ann.clone(),
                current_line: Some(no),
                current_file: file.path().to_string(),
                orphaned: false,
            };
        }
    }

    AnchoredAnnotation {
        annotation: ann.clone(),
        current_line: None,
        current_file: file.path().to_string(),
        orphaned: true,
    }
}

// ── Attention ledger (notification transition detection) ───────────────

/// A workflow item that just entered a decision-needed state through an
/// external write (i.e. the agent, not a clash button).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttentionEvent {
    pub project: String,
    pub slug: String,
    pub title: String,
    pub status: WorkflowStatus,
    /// The status the item left — `Reviewing` means a review round just
    /// handed back, which the GUI reports with the round's verdict.
    pub from: WorkflowStatus,
}

/// Pure transition detector behind workflow notifications.
///
/// The GUI holds one of these in a mutex: every reload calls [`observe`];
/// every clash-side mutation calls [`record_local_write`] right after
/// persisting, so the watcher-triggered reload that follows sees no
/// transition — the user is never notified about their own click. Only
/// external (agent) writes produce events.
///
/// [`observe`]: AttentionLedger::observe
/// [`record_local_write`]: AttentionLedger::record_local_write
#[derive(Debug, Default)]
pub struct AttentionLedger {
    prev: HashMap<(String, String), WorkflowStatus>,
}

impl AttentionLedger {
    /// Pre-seed the ledger with a status clash itself just wrote.
    pub fn record_local_write(&mut self, project: &str, slug: &str, status: WorkflowStatus) {
        self.prev
            .insert((project.to_string(), slug.to_string()), status);
    }

    /// Compare the freshly-loaded items against the previous observation and
    /// return the items that transitioned *into* a `needs_attention` state.
    /// The first observation of an item seeds the ledger silently (no
    /// notification replay on app start).
    pub fn observe(&mut self, items: &[WorkflowItem]) -> Vec<AttentionEvent> {
        let mut events = Vec::new();
        for item in items {
            let key = (item.project.clone(), item.slug.clone());
            let status = item.meta.status;
            match self.prev.insert(key, status) {
                Some(prev) if prev != status && status.needs_attention() => {
                    events.push(AttentionEvent {
                        project: item.project.clone(),
                        slug: item.slug.clone(),
                        title: item.meta.title.clone(),
                        status,
                        from: prev,
                    });
                }
                _ => {}
            }
        }
        events
    }
}

// ── Agent review report parsing ─────────────────────────────────────────

/// Pure: parse the **last** `## Review <n> …` round out of `agent-review.md`.
///
/// The section shape is contractual (the `clash-review` skill's Finish step):
/// a `## Review <n> — <heading>` heading, a `**Verdict:**` paragraph, and a
/// `### Published` list. Everything is parsed leniently — a missing piece
/// yields an empty field, never a parse failure, because the file is
/// agent-written prose.
pub fn latest_agent_review(md: &str) -> Option<AgentReviewSummary> {
    let lines: Vec<&str> = md.lines().collect();
    let (start, end, round) = last_round_bounds(&lines)?;
    Some(parse_round(&lines, start, end, round))
}

/// Pure: every `## Review <n>` round of `agent-review.md`, in file order —
/// the Timeline view renders one card per round, not just the latest.
pub fn all_agent_reviews(md: &str) -> Vec<AgentReviewSummary> {
    let lines: Vec<&str> = md.lines().collect();
    round_starts(&lines)
        .into_iter()
        .map(|(start, round)| {
            // A round's section ends at the next H2 of any kind — same rule as
            // `last_round_bounds`, so an interleaved "## Addendum" never leaks
            // into the preceding round's content.
            let end = next_h2(&lines, start).unwrap_or(lines.len());
            parse_round(&lines, start, end, round)
        })
        .collect()
}

/// Indices and round numbers of every `## Review <n>` heading.
fn round_starts(lines: &[&str]) -> Vec<(usize, u32)> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let round = l
                .strip_prefix("## Review ")?
                .split_whitespace()
                .next()?
                .parse::<u32>()
                .ok()?;
            Some((i, round))
        })
        .collect()
}

/// Index of the first `## ` heading after `start`, if any.
fn next_h2(lines: &[&str], start: usize) -> Option<usize> {
    lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("## "))
        .map(|i| start + 1 + i)
}

fn parse_round(lines: &[&str], start: usize, end: usize, round: u32) -> AgentReviewSummary {
    let rest = lines[start].strip_prefix("## Review ").unwrap_or_default();
    let round_tok = rest.split_whitespace().next().unwrap_or_default();
    let heading = rest[round_tok.len()..]
        .trim_start()
        .trim_start_matches(['—', '-'])
        .trim()
        .to_string();
    let section = &lines[start + 1..end];

    // Verdict: from the `**Verdict:**` marker to the first blank line,
    // collapsed to one line.
    let verdict = section
        .iter()
        .position(|l| l.trim_start().starts_with("**Verdict:**"))
        .map(|i| {
            let mut parts: Vec<&str> = Vec::new();
            let first = section[i].trim_start().trim_start_matches("**Verdict:**");
            parts.push(first.trim());
            for l in &section[i + 1..] {
                if l.trim().is_empty() || l.starts_with('#') {
                    break;
                }
                parts.push(l.trim());
            }
            parts.join(" ").trim().to_string()
        })
        .unwrap_or_default();

    // Published: the non-empty lines under `### Published`, bullets stripped.
    let published = section
        .iter()
        .position(|l| l.trim() == "### Published")
        .map(|i| {
            let mut out = Vec::new();
            let mut para: Vec<&str> = Vec::new();
            for l in &section[i + 1..] {
                if l.starts_with('#') {
                    break;
                }
                let t = l.trim();
                if t.is_empty() {
                    continue;
                }
                // A new bullet closes the previous one; continuation lines of
                // a wrapped bullet are joined onto it.
                if let Some(item) = t.strip_prefix("- ") {
                    if !para.is_empty() {
                        out.push(para.join(" "));
                    }
                    para = vec![item.trim()];
                } else if !para.is_empty() {
                    para.push(t);
                } else {
                    para = vec![t];
                }
            }
            if !para.is_empty() {
                out.push(para.join(" "));
            }
            out
        })
        .unwrap_or_default();

    AgentReviewSummary {
        round,
        heading,
        verdict,
        published,
    }
}

/// Line span (start inclusive, end exclusive) and round number of the last
/// `## Review <n> …` section. `end` is the next H2 or EOF.
fn last_round_bounds(lines: &[&str]) -> Option<(usize, usize, u32)> {
    let start = lines.iter().rposition(|l| {
        l.strip_prefix("## Review ")
            .and_then(|rest| rest.split_whitespace().next())
            .is_some_and(|tok| tok.parse::<u32>().is_ok())
    })?;
    let round: u32 = lines[start]
        .strip_prefix("## Review ")?
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    let end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("## "))
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());
    Some((start, end, round))
}

/// Pure: the full markdown of the last `## Review <n>` section, heading
/// included — what "Post round N to the PR" publishes as one PR comment.
pub fn latest_agent_review_section(md: &str) -> Option<(u32, String)> {
    let lines: Vec<&str> = md.lines().collect();
    let (start, end, round) = last_round_bounds(&lines)?;
    Some((round, lines[start..end].join("\n").trim_end().to_string()))
}

// ── review.md iteration parsing ─────────────────────────────────────────

/// Pure: every `## Iteration <n>` section of `review.md`, in file order.
///
/// A section runs to the next `## Iteration` heading — **not** to any H2,
/// because the human's note is markdown and legitimately contains its own H2s
/// (the change-request composer's template starts with `## What to change`).
/// The `### Open annotations` digest clash appends is split out so the
/// Timeline can render the note and the annotation count separately.
pub fn parse_review_iterations(md: &str) -> Vec<crate::domain::workflow::ReviewIterationNote> {
    let lines: Vec<&str> = md.lines().collect();
    let starts: Vec<(usize, u32)> = lines
        .iter()
        .enumerate()
        .filter_map(|(i, l)| {
            let n = l
                .strip_prefix("## Iteration ")?
                .split_whitespace()
                .next()?
                .parse::<u32>()
                .ok()?;
            Some((i, n))
        })
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(idx, &(start, iteration))| {
            let end = starts.get(idx + 1).map(|&(s, _)| s).unwrap_or(lines.len());
            let rest = lines[start]
                .strip_prefix("## Iteration ")
                .unwrap_or_default();
            let tok = rest.split_whitespace().next().unwrap_or_default();
            let heading = rest[tok.len()..]
                .trim_start()
                .trim_start_matches(['—', '-'])
                .trim()
                .to_string();
            let section = &lines[start + 1..end];
            let ann_pos = section
                .iter()
                .position(|l| l.trim() == "### Open annotations");
            let note = section[..ann_pos.unwrap_or(section.len())]
                .join("\n")
                .trim()
                .to_string();
            let annotations = ann_pos
                .map(|i| {
                    section[i + 1..]
                        .iter()
                        .filter_map(|l| l.trim().strip_prefix("- ").map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            crate::domain::workflow::ReviewIterationNote {
                iteration,
                heading,
                note,
                annotations,
            }
        })
        .collect()
}

// ── Model selection ─────────────────────────────────────────────────────

/// The model a *thinking* phase runs on — planning and reviewing.
pub const MODEL_PLAN_REVIEW: &str = "claude-fable-5";

/// The model an *implementing* phase runs on.
pub const MODEL_IMPLEMENT: &str = "claude-opus-5";

/// Pure: pin the model for a workflow phase. See `docs/workflows.md`.
///
/// `revise` is a *planning* phase — it rewrites the plan, not the code — and
/// `pr` writes prose about a finished diff without touching it. An unrecognized
/// phase gets the implementation model, since under-powering real work is the
/// worse failure.
pub fn model_for_phase(phase: &str) -> &'static str {
    match phase {
        "plan" | "revise" | "review" | "pr" => MODEL_PLAN_REVIEW,
        _ => MODEL_IMPLEMENT,
    }
}

/// Phases that do not move the item into a working status when launched.
///
/// `pr` runs *on* an item parked at a human decision and its only output is a
/// PR; flipping it to `implementing` would advertise work that isn't happening
/// and, worse, let it re-enter the implement loop. The agent's own last act
/// sets `pr-draft`.
pub fn phase_keeps_status(phase: &str) -> bool {
    phase == "pr"
}

// ── PR body ─────────────────────────────────────────────────────────────

/// Pure: compose a draft PR body from the item's own `plan.md`. A transcription,
/// not a summary — no model runs. See `docs/workflows.md`.
///
/// `None` when there is no plan, so the caller leaves the body empty rather than
/// opening a PR described by a bare heading.
pub fn pr_body_from_plan(plan: &str, iteration: u32, review_rounds: u32) -> Option<String> {
    let plan = plan.trim();
    if plan.is_empty() {
        return None;
    }
    let mut body = String::from("## Plan\n\n");
    body.push_str(plan);
    body.push_str("\n\n---\n");
    let mut trail = Vec::new();
    if iteration > 0 {
        trail.push(format!(
            "{} change round{}",
            iteration,
            if iteration > 1 { "s" } else { "" }
        ));
    }
    if review_rounds > 0 {
        trail.push(format!(
            "{} agent review round{}",
            review_rounds,
            if review_rounds > 1 { "s" } else { "" }
        ));
    }
    if trail.is_empty() {
        body.push_str("Drafted from a clash workflow item.\n");
    } else {
        body.push_str(&format!(
            "Drafted from a clash workflow item after {}.\n",
            trail.join(" and ")
        ));
    }
    Some(body)
}

// ── Session naming ──────────────────────────────────────────────────────

/// The job half of a review-shaped round's session name: what the agent is
/// *doing* (plan review / code review / explain / answer PR comments), with
/// the round number where rounds accumulate.
pub fn review_job(review: &crate::domain::workflow::WorkflowReview) -> String {
    use crate::domain::workflow::{ReviewPublish, ReviewTarget};
    if review.publish == ReviewPublish::RespondPrComments {
        return "answer PR comments".to_string();
    }
    match review.target {
        ReviewTarget::Structure => "explain".to_string(),
        ReviewTarget::Plan => format!("plan review r{}", review.round.max(1)),
        ReviewTarget::Diff | ReviewTarget::Unknown => {
            format!("code review r{}", review.round.max(1))
        }
    }
}

/// Compose an item session's display name: the item title (shortened; falls
/// back to `wf-<slug>` when empty) followed by the job — `Auth refactor ·
/// implement` — so the sessions sidebar says what each agent is doing and for
/// which item. An item with `bareSessionNames: true` (the item Settings tab's
/// toggle) gets the bare job instead.
pub fn workflow_session_name(
    meta: &crate::domain::workflow::WorkflowMeta,
    slug: &str,
    job: &str,
) -> String {
    if meta.bare_session_names {
        return job.to_string();
    }
    let title = meta.title.trim();
    let prefix = if title.is_empty() {
        format!("wf-{}", slug)
    } else if title.chars().count() > 28 {
        let mut s: String = title.chars().take(27).collect();
        s.push('…');
        s
    } else {
        title.to_string()
    };
    format!("{} · {}", prefix, job)
}

// ── Agent kickoff prompt ────────────────────────────────────────────────

/// Everything that shapes an executor kickoff beyond the item directory.
/// A struct because the launch surfaces keep growing options (the composer's
/// "launch now" carries interaction mode and a skill override) and a
/// five-positional-argument prompt builder is how fields get swapped.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExecutorKickoff<'a> {
    /// `plan` | `revise` | `implement` | `pr`.
    pub phase: &'a str,
    pub mode: WorkflowMode,
    /// PR-creation skill from config; the agent must open PRs through it.
    pub pr_skill: Option<&'a str>,
    /// Executor skill override. `None`/blank means `clash-workflow`; a value
    /// routes the round through a custom skill that honors the same file
    /// contract (`docs/workflows.md`) — the power-user escape hatch.
    pub skill: Option<&'a str>,
    /// Launch-time interactivity choice; `None` means the skill's opening
    /// question asks in-session.
    pub interactive: Option<bool>,
}

/// Build the initial prompt for a workflow agent session. The skill owns the
/// actual behavior; the prompt only routes it to the item directory, the
/// requested phase and the item's entry mode.
///
/// The mode is also in `meta.json`, but stating it up front is what makes a
/// `review-only` run reliably skip the plan: the agent knows before it reads
/// anything that there is no plan to write and no `plan-review` to hand back to.
pub fn build_agent_prompt(item_dir: &str, kickoff: &ExecutorKickoff) -> String {
    let skill = kickoff
        .skill
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("clash-workflow");
    let mut prompt = format!(
        "Use the {} skill. Workflow item directory: {}. Phase: {}. Mode: {}.",
        skill, item_dir, kickoff.phase, kickoff.mode
    );
    if let Some(s) = kickoff.pr_skill.map(str::trim).filter(|s| !s.is_empty()) {
        prompt.push_str(&format!(" PR skill: {}.", s));
    }
    match kickoff.interactive {
        Some(true) => prompt.push_str(" Interactive: yes."),
        Some(false) => prompt.push_str(" Interactive: no."),
        None => {}
    }
    prompt
}

/// Pure: the review skill for a round's target. Plan reviews and code reviews
/// are deliberately **separate skills** — a reviewer that may rewrite what it
/// reviews has reviewed nothing, and a skill that does both jobs describes
/// neither well. Each skill is self-contained: it owns its file contract
/// (`agent-review.md` + `annotations.json`) and the status hand-back.
///
/// Engines are always clash-owned embedded skills, so a review round needs no
/// third-party plugin installed to work.
pub fn review_engine_for(target: crate::domain::workflow::ReviewTarget) -> &'static str {
    use crate::domain::workflow::ReviewTarget;
    match target {
        ReviewTarget::Plan => "clash-plan-review",
        // The explainer: writes structure.md instead of findings.
        ReviewTarget::Structure => "clash-explain",
        // Unknown degrades to the code reviewer: every mode has a diff to
        // read, while a plan may not exist at all.
        ReviewTarget::Diff | ReviewTarget::Unknown => "clash-code-review",
    }
}

/// Build the kickoff prompt for an agent **review** round — a different skill
/// per target (see [`review_engine_for`]), never the executor: reviewing and
/// implementing are different jobs and mixing them into one skill makes both
/// vaguer.
///
/// Every parameter is also in `meta.json.review`, but stating them up front is
/// what lets the reviewer refuse impossible work immediately: a `plan` target
/// with no plan, or a publish mode needing a PR that does not exist.
/// `Return to:` is the repeatability contract — the round ends by putting the
/// item back exactly where the human launched it from, so the next round can
/// start from the same place.
///
/// `Interactive:` carries the human's launch-time choice; when they made none
/// the field is omitted and the skill's own opening question asks in-session.
pub fn build_review_prompt(
    item_dir: &str,
    review: &crate::domain::workflow::WorkflowReview,
    mode: WorkflowMode,
) -> String {
    let engine = review_engine_for(review.target);
    let interactive = match review.interactive {
        Some(true) => " Interactive: yes.",
        Some(false) => " Interactive: no.",
        None => "",
    };
    format!(
        "Use the {} skill. Workflow item directory: {}. \
         Target: {}. Depth: {}. Publish: {}. Round: {}. Return to: {}. Mode: {}.{}",
        engine,
        item_dir,
        review.target,
        review.depth,
        review.publish,
        review.round.max(1),
        review.return_status,
        mode,
        interactive
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::diff::parse_file_diffs;
    use crate::domain::workflow::AnnotationStatus;

    #[test]
    fn thinking_phases_run_on_fable() {
        // `revise` rewrites the *plan*, so it belongs with planning, not with
        // implementing — the easy one to get wrong.
        for phase in ["plan", "revise", "review"] {
            assert_eq!(model_for_phase(phase), MODEL_PLAN_REVIEW, "phase {}", phase);
        }
    }

    #[test]
    fn implement_runs_on_opus() {
        assert_eq!(model_for_phase("implement"), MODEL_IMPLEMENT);
    }

    #[test]
    fn review_engine_routes_by_target() {
        use crate::domain::workflow::ReviewTarget;
        // Plan review and code review are separate skills — that split is a
        // correctness property, not packaging.
        assert_eq!(review_engine_for(ReviewTarget::Plan), "clash-plan-review");
        assert_eq!(review_engine_for(ReviewTarget::Diff), "clash-code-review");
        // The explainer writes structure.md instead of findings.
        assert_eq!(review_engine_for(ReviewTarget::Structure), "clash-explain");
        // Unknown degrades to the code reviewer: every mode has a diff.
        assert_eq!(
            review_engine_for(ReviewTarget::Unknown),
            "clash-code-review"
        );
    }

    /// Embedded skills are clash's own, so a name collision can't hijack a
    /// third party's skill (or be hijacked by one).
    #[test]
    fn only_clash_owned_skills_are_embedded() {
        for s in crate::infrastructure::skills::SKILLS {
            assert!(
                s.name.starts_with("clash-"),
                "embedded skill {:?} is not clash-owned",
                s.name
            );
        }
    }

    /// A review engine must name a skill clash actually installs — otherwise
    /// the round dies on an unresolvable skill after a full session spawn.
    #[test]
    fn every_skill_engine_is_one_clash_installs() {
        use crate::domain::workflow::ReviewTarget;
        let installed: Vec<&str> = crate::infrastructure::skills::SKILLS
            .iter()
            .map(|s| s.name)
            .collect();
        for t in [
            ReviewTarget::Plan,
            ReviewTarget::Diff,
            ReviewTarget::Structure,
            ReviewTarget::Unknown,
        ] {
            let e = review_engine_for(t);
            assert!(
                installed.contains(&e),
                "engine {:?} is not an embedded skill — add it to SKILLS",
                e
            );
        }
    }

    #[test]
    fn review_prompt_names_the_skill_for_the_target() {
        use crate::domain::workflow::{ReviewDepth, ReviewTarget, WorkflowReview};
        let diff = WorkflowReview {
            target: ReviewTarget::Diff,
            depth: ReviewDepth::Deep,
            ..Default::default()
        };
        let p = build_review_prompt("/items/x", &diff, WorkflowMode::Full);
        assert!(p.contains("Use the clash-code-review skill"));

        let plan = WorkflowReview {
            target: ReviewTarget::Plan,
            ..Default::default()
        };
        let p = build_review_prompt("/items/x", &plan, WorkflowMode::Full);
        assert!(p.contains("Use the clash-plan-review skill"));
        // The retired harness must never come back into a kickoff.
        assert!(!p.contains("clash-review skill"));
    }

    #[test]
    fn review_prompt_carries_the_interactivity_choice_or_omits_it() {
        use crate::domain::workflow::WorkflowReview;
        // No launch-time choice → no field: the skill asks in-session.
        let ask = WorkflowReview::default();
        let p = build_review_prompt("/x", &ask, WorkflowMode::Full);
        assert!(!p.contains("Interactive:"));

        let yes = WorkflowReview {
            interactive: Some(true),
            ..Default::default()
        };
        assert!(build_review_prompt("/x", &yes, WorkflowMode::Full).ends_with("Interactive: yes."));

        let no = WorkflowReview {
            interactive: Some(false),
            ..Default::default()
        };
        assert!(build_review_prompt("/x", &no, WorkflowMode::Full).ends_with("Interactive: no."));
    }

    #[test]
    fn pr_body_transcribes_the_plan_and_the_round_trail() {
        let b = pr_body_from_plan("Add a widget.\n\n- step one", 2, 1).unwrap();
        assert!(b.starts_with("## Plan\n\nAdd a widget."));
        assert!(b.contains("- step one"));
        assert!(b.contains("2 change rounds and 1 agent review round"));
    }

    #[test]
    fn pr_body_is_none_without_a_plan() {
        // Better an empty body than a PR described only by a bare heading.
        assert!(pr_body_from_plan("", 0, 0).is_none());
        assert!(pr_body_from_plan("   \n\t\n ", 3, 2).is_none());
    }

    #[test]
    fn pr_body_omits_a_trail_it_has_no_counts_for() {
        let b = pr_body_from_plan("Do the thing.", 0, 0).unwrap();
        assert!(b.contains("Drafted from a clash workflow item.\n"));
        assert!(!b.contains("after"));
    }

    #[test]
    fn unknown_phase_falls_back_to_the_implementation_model() {
        // Deliberate: an unrecognized phase is assumed to do work, and
        // under-powering real work is the worse of the two failures.
        assert_eq!(model_for_phase("something-new"), MODEL_IMPLEMENT);
        assert_eq!(model_for_phase(""), MODEL_IMPLEMENT);
    }

    fn ann(file: &str, side: DiffSide, line: u32, content: &str) -> Annotation {
        Annotation {
            id: format!("a-{}-{}", file, line),
            file: file.to_string(),
            side,
            line,
            line_content: content.to_string(),
            line_content_hash: line_hash(content),
            body: "comment".into(),
            status: AnnotationStatus::Open,
            ..Annotation::default()
        }
    }

    // ── slugify ─────────────────────────────────────────────────────

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Auth Refactor"), "auth-refactor");
        assert_eq!(slugify("  Fix: PTY / locale bug!  "), "fix-pty-locale-bug");
        assert_eq!(slugify("émoji 🚀 title"), "moji-title");
        assert_eq!(slugify("---"), "item");
        assert_eq!(slugify(""), "item");
        assert_eq!(slugify("../evil"), "evil");
        assert!(!slugify("a:b/c\\d").contains([':', '/', '\\']));
    }

    // ── line_hash ───────────────────────────────────────────────────

    #[test]
    fn line_hash_trims_and_is_stable() {
        // Indentation and CRLF do not change the hash.
        assert_eq!(line_hash("let x = 1;"), line_hash("    let x = 1;"));
        assert_eq!(line_hash("let x = 1;"), line_hash("let x = 1;\r"));
        assert_ne!(line_hash("let x = 1;"), line_hash("let x = 2;"));
        // Known FNV-1a vector: empty input → offset basis.
        assert_eq!(line_hash(""), "cbf29ce484222325");
    }

    const SIMPLE_DIFF: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
index 111..222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -10,4 +10,5 @@ fn ctx()
 keep one
-old line
+new line
+added line
 keep two
";

    // ── anchor_annotations ──────────────────────────────────────────

    #[test]
    fn anchor_exact_match() {
        let files = parse_file_diffs(SIMPLE_DIFF);
        let a = ann("src/lib.rs", DiffSide::New, 11, "new line");
        let out = anchor_annotations(&files, &[a]);
        assert!(!out[0].orphaned);
        assert_eq!(out[0].current_line, Some(11));
        assert_eq!(out[0].current_file, "src/lib.rs");
    }

    #[test]
    fn anchor_old_side() {
        let files = parse_file_diffs(SIMPLE_DIFF);
        let a = ann("src/lib.rs", DiffSide::Old, 11, "old line");
        let out = anchor_annotations(&files, &[a]);
        assert!(!out[0].orphaned);
        assert_eq!(out[0].current_line, Some(11));
    }

    #[test]
    fn anchor_reanchors_when_line_shifts() {
        // Same content, but the annotation was recorded at line 5 in an older
        // iteration; the line now lives at new 11.
        let files = parse_file_diffs(SIMPLE_DIFF);
        let a = ann("src/lib.rs", DiffSide::New, 5, "new line");
        let out = anchor_annotations(&files, &[a]);
        assert!(!out[0].orphaned);
        assert_eq!(out[0].current_line, Some(11));
    }

    #[test]
    fn anchor_orphans_when_content_gone() {
        let files = parse_file_diffs(SIMPLE_DIFF);
        let a = ann("src/lib.rs", DiffSide::New, 11, "content that vanished");
        let out = anchor_annotations(&files, &[a]);
        assert!(out[0].orphaned);
        assert_eq!(out[0].current_line, None);
        // File context is preserved for the orphan tray.
        assert_eq!(out[0].current_file, "src/lib.rs");
    }

    #[test]
    fn anchor_orphans_when_file_gone() {
        let files = parse_file_diffs(SIMPLE_DIFF);
        let a = ann("src/other.rs", DiffSide::New, 1, "whatever");
        let out = anchor_annotations(&files, &[a]);
        assert!(out[0].orphaned);
        assert_eq!(out[0].current_file, "src/other.rs");
    }

    #[test]
    fn anchor_follows_renames() {
        let text = "\
diff --git a/old_name.rs b/new_name.rs
rename from old_name.rs
rename to new_name.rs
--- a/old_name.rs
+++ b/new_name.rs
@@ -3 +3 @@
+let x = qux();
";
        let files = parse_file_diffs(text);
        let a = ann("old_name.rs", DiffSide::New, 3, "let x = qux();");
        let out = anchor_annotations(&files, &[a]);
        assert!(!out[0].orphaned);
        assert_eq!(out[0].current_file, "new_name.rs");
        assert_eq!(out[0].current_line, Some(3));
    }

    #[test]
    fn anchor_duplicate_lines_nearest_wins() {
        let text = "\
diff --git a/dup.rs b/dup.rs
--- a/dup.rs
+++ b/dup.rs
@@ -1,7 +1,7 @@
+let x = 1;
 a
 b
+let x = 1;
 c
 d
+let x = 1;
";
        let files = parse_file_diffs(text);
        // Duplicates live at new 1, 4, 7. Recorded at 5 → nearest is 4.
        let a = ann("dup.rs", DiffSide::New, 5, "let x = 1;");
        let out = anchor_annotations(&files, &[a]);
        assert_eq!(out[0].current_line, Some(4));
    }

    #[test]
    fn anchor_crlf_content_still_matches() {
        // The stored content came from a CRLF checkout; the diff has LF.
        let files = parse_file_diffs(SIMPLE_DIFF);
        let a = ann("src/lib.rs", DiffSide::New, 11, "new line\r");
        let out = anchor_annotations(&files, &[a]);
        assert!(!out[0].orphaned);
    }

    #[test]
    fn anchor_context_line_dropped_from_shrunken_hunk() {
        // The annotated context line fell out of the hunk in this iteration.
        let a = ann("src/lib.rs", DiffSide::New, 42, "some far away context");
        let out = anchor_annotations(&parse_file_diffs(SIMPLE_DIFF), &[a]);
        assert!(out[0].orphaned);
    }

    #[test]
    fn anchor_without_hash_uses_exact_line_only() {
        let files = parse_file_diffs(SIMPLE_DIFF);
        let mut a = ann("src/lib.rs", DiffSide::New, 11, "");
        a.line_content_hash = String::new();
        let out = anchor_annotations(&files, &[a]);
        // Line 11 exists on the new side → anchors by position alone.
        assert!(!out[0].orphaned);
        assert_eq!(out[0].current_line, Some(11));
    }

    // ── latest_agent_review ─────────────────────────────────────────

    const REVIEW_MD: &str = "\
# Agent review\n\n\
## Review 1 — plan · deep · 2026-08-03 11:52\n\n\
**Verdict:** ship it\n\n\
### Blockers\n\nnone\n\n\
## Review 4 — diff · deep · 2026-08-04 17:27\n\n\
**Verdict:** 2 blockers, both procedural fallout from round 3's decisions\n\
not landing in the branch: the commit message still ships a MAJOR.\n\n\
### Blockers\n\n1. `a.yml:99` — wrong\n\n\
### Published\n\n\
- Publish mode was `respond-pr-comments`, but PR #571 has **zero review\n\
  comments** — nothing to answer, so nothing was posted.\n\
- Findings live in `annotations.json` (r4-1 … r4-12).\n";

    #[test]
    fn latest_agent_review_parses_last_round() {
        let s = latest_agent_review(REVIEW_MD).expect("summary");
        assert_eq!(s.round, 4);
        assert_eq!(s.heading, "diff · deep · 2026-08-04 17:27");
        // Wrapped verdict collapses to one line and stops at the blank line.
        assert!(s.verdict.starts_with("2 blockers, both procedural"));
        assert!(s.verdict.ends_with("ships a MAJOR."));
        assert!(!s.verdict.contains('\n'));
        // Wrapped bullets are joined; both entries survive.
        assert_eq!(s.published.len(), 2);
        assert!(s.published[0].contains("nothing was posted"));
        assert!(s.published[1].starts_with("Findings live"));
    }

    #[test]
    fn latest_agent_review_without_published_or_verdict() {
        let s = latest_agent_review("## Review 2 — plan · standard · x\n\nprose only\n")
            .expect("summary");
        assert_eq!(s.round, 2);
        assert_eq!(s.verdict, "");
        assert!(s.published.is_empty());
    }

    #[test]
    fn latest_agent_review_ignores_non_round_h2s() {
        // Interleaved sections like "## Addendum 2 …" or decision logs must not
        // shadow the last real round (both occur in real files).
        let md = format!(
            "{}\n## Addendum 2 — refined against sprint\n\nmore\n",
            REVIEW_MD
        );
        let s = latest_agent_review(&md).expect("summary");
        assert_eq!(s.round, 4);
    }

    #[test]
    fn latest_agent_review_none_when_no_rounds() {
        assert!(latest_agent_review("").is_none());
        assert!(latest_agent_review("# notes\n\njust prose\n").is_none());
    }

    #[test]
    fn all_agent_reviews_returns_every_round_in_order() {
        let rounds = all_agent_reviews(REVIEW_MD);
        assert_eq!(rounds.len(), 2);
        assert_eq!(rounds[0].round, 1);
        assert_eq!(rounds[0].verdict, "ship it");
        assert_eq!(rounds[1].round, 4);
        assert_eq!(rounds[1].published.len(), 2);
        // Interleaved non-round H2s neither become rounds nor leak into one.
        let md = format!("{}\n## Addendum 2 — notes\n\nmore\n", REVIEW_MD);
        let rounds = all_agent_reviews(&md);
        assert_eq!(rounds.len(), 2);
        assert!(!rounds[1].published.iter().any(|p| p.contains("more")));
        assert!(all_agent_reviews("just prose\n").is_empty());
    }

    // ── parse_review_iterations ─────────────────────────────────────

    #[test]
    fn review_iterations_parse_notes_and_annotation_digests() {
        let md = "\
## Iteration 1 — 2026-08-01 10:00\n\n\
Tighten the API.\n\n\
### Open annotations\n\n\
- `src/a.rs:12` — rename this\n\
- `src/b.rs:3` — split it\n\n\
## Iteration 2 — 2026-08-02 11:30\n\n\
## What to change\n\nUse the helper instead.\n\n## Out of scope\n\nPerf.\n";
        let its = parse_review_iterations(md);
        assert_eq!(its.len(), 2);
        assert_eq!(its[0].iteration, 1);
        assert_eq!(its[0].heading, "2026-08-01 10:00");
        assert_eq!(its[0].note, "Tighten the API.");
        assert_eq!(its[0].annotations.len(), 2);
        assert!(its[0].annotations[0].contains("src/a.rs:12"));
        // The composer template's own H2s stay inside the note — a section
        // ends only at the next `## Iteration` heading.
        assert_eq!(its[1].iteration, 2);
        assert!(its[1].note.contains("## What to change"));
        assert!(its[1].note.contains("## Out of scope"));
        assert!(its[1].annotations.is_empty());
    }

    #[test]
    fn review_iterations_empty_and_noteless_sections() {
        assert!(parse_review_iterations("").is_empty());
        // A round sent with annotations only (no note) parses to an empty note.
        let md = "## Iteration 3 — x\n\n### Open annotations\n\n- `f:1` — fix\n";
        let its = parse_review_iterations(md);
        assert_eq!(its.len(), 1);
        assert_eq!(its[0].note, "");
        assert_eq!(its[0].annotations, vec!["`f:1` — fix".to_string()]);
    }

    #[test]
    fn latest_agent_review_section_returns_whole_round() {
        let (round, section) = latest_agent_review_section(REVIEW_MD).expect("section");
        assert_eq!(round, 4);
        assert!(section.starts_with("## Review 4 — diff · deep"));
        assert!(section.contains("### Published"));
        // Round 1's content stays out.
        assert!(!section.contains("ship it"));
    }

    // ── AttentionLedger ─────────────────────────────────────────────

    fn item(project: &str, slug: &str, status: WorkflowStatus) -> WorkflowItem {
        WorkflowItem {
            project: project.into(),
            slug: slug.into(),
            meta: crate::domain::workflow::WorkflowMeta {
                title: format!("{} title", slug),
                status,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn ledger_external_transition_notifies() {
        let mut ledger = AttentionLedger::default();
        // First sight seeds silently — even in an attention state.
        let first = ledger.observe(&[item("p", "a", WorkflowStatus::PlanReview)]);
        assert!(first.is_empty());

        // Agent moves it planning → plan-review externally.
        ledger.observe(&[item("p", "a", WorkflowStatus::Planning)]);
        let events = ledger.observe(&[item("p", "a", WorkflowStatus::PlanReview)]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].status, WorkflowStatus::PlanReview);
        assert_eq!(events[0].slug, "a");
        assert_eq!(events[0].from, WorkflowStatus::Planning);
    }

    #[test]
    fn ledger_review_handback_carries_from_reviewing() {
        let mut ledger = AttentionLedger::default();
        ledger.observe(&[item("p", "a", WorkflowStatus::Reviewing)]);
        let events = ledger.observe(&[item("p", "a", WorkflowStatus::PrDraft)]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].from, WorkflowStatus::Reviewing);
    }

    #[test]
    fn ledger_local_write_is_suppressed() {
        let mut ledger = AttentionLedger::default();
        ledger.observe(&[item("p", "a", WorkflowStatus::Implementing)]);
        // Clash itself moves the item into diff-review (e.g. a test flow),
        // pre-seeding the ledger — the reload must stay silent.
        ledger.record_local_write("p", "a", WorkflowStatus::DiffReview);
        let events = ledger.observe(&[item("p", "a", WorkflowStatus::DiffReview)]);
        assert!(events.is_empty());
    }

    #[test]
    fn ledger_silent_when_an_item_is_born_needing_attention() {
        // from-plan and review-only items are *created* in an attention state.
        // Creation seeds the ledger, so the very first reload must not notify
        // the user about a decision they just asked for.
        for born in [
            WorkflowMode::FromPlan.initial_status(),
            WorkflowMode::ReviewOnly.initial_status(),
        ] {
            assert!(born.needs_attention(), "{} should need a decision", born);
            let mut ledger = AttentionLedger::default();
            ledger.record_local_write("p", "a", born);
            assert!(
                ledger.observe(&[item("p", "a", born)]).is_empty(),
                "{}",
                born
            );
        }
    }

    #[test]
    fn ledger_does_not_renotify_same_status() {
        let mut ledger = AttentionLedger::default();
        ledger.observe(&[item("p", "a", WorkflowStatus::Planning)]);
        assert_eq!(
            ledger
                .observe(&[item("p", "a", WorkflowStatus::PlanReview)])
                .len(),
            1
        );
        // Subsequent reloads with the unchanged status stay quiet.
        assert!(ledger
            .observe(&[item("p", "a", WorkflowStatus::PlanReview)])
            .is_empty());
        assert!(ledger
            .observe(&[item("p", "a", WorkflowStatus::PlanReview)])
            .is_empty());
    }

    #[test]
    fn ledger_non_attention_transition_is_silent() {
        let mut ledger = AttentionLedger::default();
        ledger.observe(&[item("p", "a", WorkflowStatus::Draft)]);
        assert!(ledger
            .observe(&[item("p", "a", WorkflowStatus::Planning)])
            .is_empty());
    }

    // ── build_agent_prompt ──────────────────────────────────────────

    #[test]
    fn review_prompt_carries_the_whole_round() {
        use crate::domain::workflow::{ReviewDepth, ReviewPublish, ReviewTarget, WorkflowReview};
        let p = build_review_prompt(
            "/x/workflows/clash/auth",
            &WorkflowReview {
                target: ReviewTarget::Diff,
                depth: ReviewDepth::Deep,
                publish: ReviewPublish::RespondPrComments,
                return_status: WorkflowStatus::PrDraft,
                round: 4,
                ..WorkflowReview::default()
            },
            WorkflowMode::Full,
        );
        // A different skill than the executor — reviewing is not implementing.
        assert!(p.contains("clash-code-review skill"));
        assert!(!p.contains("clash-workflow skill"));
        assert!(p.contains("/x/workflows/clash/auth"));
        assert!(p.contains("Target: diff."));
        assert!(p.contains("Depth: deep."));
        assert!(p.contains("Publish: respond-pr-comments."));
        assert!(p.contains("Round: 4."));
        // The repeatability contract has to be in the prompt, not just on disk.
        assert!(p.contains("Return to: pr-draft."));
        assert!(p.contains("Mode: full."));
    }

    #[test]
    fn review_prompt_never_says_round_zero() {
        // A default/legacy review block would otherwise ask the agent to write
        // a "## Review 0" section.
        let p = build_review_prompt(
            "/x/i",
            &crate::domain::workflow::WorkflowReview::default(),
            WorkflowMode::Full,
        );
        assert!(p.contains("Round: 1."));
    }

    #[test]
    fn session_names_say_what_the_agent_does() {
        use crate::domain::workflow::{
            ReviewDepth, ReviewPublish, ReviewTarget, WorkflowMeta, WorkflowReview,
        };
        let mk = |target, publish, round| WorkflowReview {
            target,
            publish,
            round,
            depth: ReviewDepth::Deep,
            ..Default::default()
        };
        assert_eq!(
            review_job(&mk(ReviewTarget::Plan, ReviewPublish::Local, 2)),
            "plan review r2"
        );
        assert_eq!(
            review_job(&mk(ReviewTarget::Diff, ReviewPublish::PrComments, 4)),
            "code review r4"
        );
        // The job overrides the target for a respond round, and explain rounds
        // name the job, not a round number (the document is regenerated).
        assert_eq!(
            review_job(&mk(ReviewTarget::Diff, ReviewPublish::RespondPrComments, 3)),
            "answer PR comments"
        );
        assert_eq!(
            review_job(&mk(ReviewTarget::Structure, ReviewPublish::Local, 5)),
            "explain"
        );
        // A default/legacy round never says "r0".
        assert_eq!(
            review_job(&mk(ReviewTarget::Diff, ReviewPublish::Local, 0)),
            "code review r1"
        );

        // The prefix is the human title, shortened; wf-<slug> when untitled;
        // nothing at all when the item opted out in its Settings tab.
        let meta = WorkflowMeta {
            title: "Auth refactor".into(),
            ..Default::default()
        };
        assert_eq!(
            workflow_session_name(&meta, "auth-refactor", "implement"),
            "Auth refactor · implement"
        );
        let untitled = WorkflowMeta::default();
        assert_eq!(
            workflow_session_name(&untitled, "auth-refactor", "plan"),
            "wf-auth-refactor · plan"
        );
        let long = WorkflowMeta {
            title: "A very long workflow item title that keeps going".into(),
            ..Default::default()
        };
        let name = workflow_session_name(&long, "x", "explain");
        assert!(name.ends_with(" · explain"));
        assert!(name.starts_with("A very long workflow item"));
        assert!(name.contains('…'));
        let bare = WorkflowMeta {
            title: "Auth refactor".into(),
            bare_session_names: true,
            ..Default::default()
        };
        assert_eq!(
            workflow_session_name(&bare, "auth-refactor", "implement"),
            "implement"
        );
    }

    fn kickoff<'a>(phase: &'a str, mode: WorkflowMode) -> ExecutorKickoff<'a> {
        ExecutorKickoff {
            phase,
            mode,
            ..ExecutorKickoff::default()
        }
    }

    #[test]
    fn prompt_routes_dir_and_phase() {
        let p = build_agent_prompt(
            "/x/workflows/clash/auth",
            &kickoff("revise", WorkflowMode::Full),
        );
        assert!(p.contains("clash-workflow skill"));
        assert!(p.contains("/x/workflows/clash/auth"));
        assert!(p.contains("Phase: revise."));
        assert!(p.contains("Mode: full."));
    }

    #[test]
    fn prompt_carries_the_entry_mode() {
        // review-only must be visible before the agent reads any file — it is
        // what tells it there is no plan phase.
        let p = build_agent_prompt("/x/w/p/item", &kickoff("revise", WorkflowMode::ReviewOnly));
        assert!(p.contains("Mode: review-only."));
        let p = build_agent_prompt("/x/w/p/item", &kickoff("implement", WorkflowMode::FromPlan));
        assert!(p.contains("Mode: from-plan."));
    }

    #[test]
    fn prompt_names_the_pr_skill_only_when_configured() {
        let p = build_agent_prompt("/x/i", &kickoff("pr", WorkflowMode::Full));
        assert!(!p.contains("PR skill:"));
        // Blank config reads as "not configured".
        let p = build_agent_prompt(
            "/x/i",
            &ExecutorKickoff {
                pr_skill: Some("   "),
                ..kickoff("pr", WorkflowMode::Full)
            },
        );
        assert!(!p.contains("PR skill:"));
        let p = build_agent_prompt(
            "/x/i",
            &ExecutorKickoff {
                pr_skill: Some("hivebrite-engineering:github-pr"),
                ..kickoff("pr", WorkflowMode::Full)
            },
        );
        assert!(p.ends_with("PR skill: hivebrite-engineering:github-pr."));
    }

    #[test]
    fn prompt_honors_the_executor_skill_override() {
        // The composer's escape hatch: route a round through a custom skill
        // that honors the same file contract. Blank falls back to the default.
        let p = build_agent_prompt(
            "/x/i",
            &ExecutorKickoff {
                skill: Some("my-org-flow"),
                ..kickoff("revise", WorkflowMode::Full)
            },
        );
        assert!(p.starts_with("Use the my-org-flow skill."));
        assert!(!p.contains("clash-workflow"));
        let p = build_agent_prompt(
            "/x/i",
            &ExecutorKickoff {
                skill: Some("  "),
                ..kickoff("revise", WorkflowMode::Full)
            },
        );
        assert!(p.starts_with("Use the clash-workflow skill."));
    }

    #[test]
    fn prompt_carries_the_executor_interactivity_choice_or_omits_it() {
        // Absent → the skill's opening question asks in-session.
        let p = build_agent_prompt("/x/i", &kickoff("implement", WorkflowMode::Full));
        assert!(!p.contains("Interactive:"));
        let p = build_agent_prompt(
            "/x/i",
            &ExecutorKickoff {
                interactive: Some(true),
                ..kickoff("implement", WorkflowMode::Full)
            },
        );
        assert!(p.ends_with("Interactive: yes."));
        let p = build_agent_prompt(
            "/x/i",
            &ExecutorKickoff {
                interactive: Some(false),
                ..kickoff("implement", WorkflowMode::Full)
            },
        );
        assert!(p.ends_with("Interactive: no."));
    }
}
