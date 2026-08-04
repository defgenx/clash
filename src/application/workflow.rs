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

    Some(AgentReviewSummary {
        round,
        heading,
        verdict,
        published,
    })
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

// ── Agent kickoff prompt ────────────────────────────────────────────────

/// Build the initial prompt for a workflow agent session. The skill owns the
/// actual behavior; the prompt only routes it to the item directory, the
/// requested phase (`plan` | `revise` | `implement`) and the item's entry mode.
///
/// The mode is also in `meta.json`, but stating it up front is what makes a
/// `review-only` run reliably skip the plan: the agent knows before it reads
/// anything that there is no plan to write and no `plan-review` to hand back to.
pub fn build_agent_prompt(item_dir: &str, phase: &str, mode: WorkflowMode) -> String {
    format!(
        "Use the clash-workflow skill. Workflow item directory: {}. Phase: {}. Mode: {}.",
        item_dir, phase, mode
    )
}

/// Pure: the skill or slash command that performs the actual reviewing for a
/// round. `clash-review` stays the harness — it owns the file contract — and
/// delegates the judgement to this. See `docs/workflows.md`.
///
/// Engines are either clash-owned embedded skills or Claude Code built-ins, so
/// a review round needs no third-party plugin installed to work.
pub fn review_engine_for(
    target: crate::domain::workflow::ReviewTarget,
    depth: crate::domain::workflow::ReviewDepth,
) -> &'static str {
    use crate::domain::workflow::{ReviewDepth, ReviewTarget};
    match (target, depth) {
        (ReviewTarget::Plan, _) => "clash-plan-review",
        // `/code-review` reads the working diff, which is what an item has in
        // its worktree; `/review` is the lighter pass.
        (_, ReviewDepth::Deep) => "/code-review",
        _ => "/review",
    }
}

/// Build the kickoff prompt for an agent **review** round — a different skill
/// (`clash-review`) than the executor, because reviewing and implementing are
/// different jobs and mixing them into one skill makes both vaguer.
///
/// Every parameter is also in `meta.json.review`, but stating them up front is
/// what lets the reviewer refuse impossible work immediately: a `plan` target
/// with no plan, or a publish mode needing a PR that does not exist.
/// `Return to:` is the repeatability contract — the round ends by putting the
/// item back exactly where the human launched it from, so the next round can
/// start from the same place.
pub fn build_review_prompt(
    item_dir: &str,
    review: &crate::domain::workflow::WorkflowReview,
    mode: WorkflowMode,
) -> String {
    let engine = review_engine_for(review.target, review.depth);
    let via = if engine.is_empty() {
        String::new()
    } else {
        format!(
            " Perform the review itself with {} — clash-review still owns the file \
             contract (annotations.json + agent-review.md) and the status hand-back.",
            engine
        )
    };
    format!(
        "Use the clash-review skill. Workflow item directory: {}. \
         Target: {}. Depth: {}. Publish: {}. Round: {}. Return to: {}. Mode: {}.{}",
        item_dir,
        review.target,
        review.depth,
        review.publish,
        review.round.max(1),
        review.return_status,
        mode,
        via
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
    fn review_engine_routes_by_target_then_depth() {
        use crate::domain::workflow::{ReviewDepth, ReviewTarget};
        // Depth never branches a plan review: it tunes how hard a diff is read,
        // and a plan has no hunks to read harder.
        for d in [
            ReviewDepth::Standard,
            ReviewDepth::Deep,
            ReviewDepth::Unknown,
        ] {
            assert_eq!(
                review_engine_for(ReviewTarget::Plan, d),
                "clash-plan-review"
            );
        }
        assert_eq!(
            review_engine_for(ReviewTarget::Diff, ReviewDepth::Deep),
            "/code-review"
        );
        assert_eq!(
            review_engine_for(ReviewTarget::Diff, ReviewDepth::Standard),
            "/review"
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

    /// A review engine naming a skill must name one clash actually installs —
    /// otherwise the round dies on an unresolvable skill after a full session
    /// spawn. Built-in `/slash` engines are Claude Code's and exempt.
    #[test]
    fn every_skill_engine_is_one_clash_installs() {
        use crate::domain::workflow::{ReviewDepth, ReviewTarget};
        let installed: Vec<&str> = crate::infrastructure::skills::SKILLS
            .iter()
            .map(|s| s.name)
            .collect();
        for t in [
            ReviewTarget::Plan,
            ReviewTarget::Diff,
            ReviewTarget::Unknown,
        ] {
            for d in [
                ReviewDepth::Standard,
                ReviewDepth::Deep,
                ReviewDepth::Unknown,
            ] {
                let e = review_engine_for(t, d);
                if e.is_empty() || e.starts_with('/') {
                    continue;
                }
                assert!(
                    installed.contains(&e),
                    "engine {:?} is not an embedded skill — add it to SKILLS or \
                     use a built-in",
                    e
                );
            }
        }
    }

    #[test]
    fn review_prompt_names_the_engine_and_keeps_the_harness() {
        use crate::domain::workflow::{ReviewDepth, ReviewTarget, WorkflowReview};
        let review = WorkflowReview {
            target: ReviewTarget::Diff,
            depth: ReviewDepth::Deep,
            ..Default::default()
        };
        let p = build_review_prompt("/items/x", &review, WorkflowMode::Full);
        assert!(p.contains("Use the clash-review skill"));
        assert!(p.contains("/code-review"));
        // The harness must stay named, or findings land nowhere clash reads.
        assert!(p.contains("annotations.json"));
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
        assert!(p.contains("clash-review skill"));
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
    fn prompt_routes_dir_and_phase() {
        let p = build_agent_prompt("/x/workflows/clash/auth", "revise", WorkflowMode::Full);
        assert!(p.contains("clash-workflow skill"));
        assert!(p.contains("/x/workflows/clash/auth"));
        assert!(p.contains("Phase: revise."));
        assert!(p.contains("Mode: full."));
    }

    #[test]
    fn prompt_carries_the_entry_mode() {
        // review-only must be visible before the agent reads any file — it is
        // what tells it there is no plan phase.
        let p = build_agent_prompt("/x/w/p/item", "revise", WorkflowMode::ReviewOnly);
        assert!(p.contains("Mode: review-only."));
        let p = build_agent_prompt("/x/w/p/item", "implement", WorkflowMode::FromPlan);
        assert!(p.contains("Mode: from-plan."));
    }
}
