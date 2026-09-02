//! Pure share-document composition for workflow items — no IO.
//!
//! One markdown document assembled from an item's files, section by section.
//! Which sections are included is the caller's choice (the GUI's share dialog
//! previews exactly this output before anything leaves the machine); the
//! destinations — clipboard, file, webhook — only differ in transport, never
//! in content.

use crate::domain::workflow::{Annotation, AnnotationStatus, WorkflowMeta, WorkflowPr};

/// Which sections the document carries. Deserialized straight from the GUI's
/// share dialog; every flag defaults to off so a partial payload includes
/// exactly what it names.
#[derive(Debug, Clone, Copy, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShareSections {
    /// Title, status, project, branch, iteration/round counters, PR links.
    pub summary: bool,
    /// `plan.md` verbatim.
    pub plan: bool,
    /// The human's change rounds (`review.md`'s iteration notes).
    pub timeline: bool,
    /// Agent review rounds: verdicts and what each round published.
    pub reviews: bool,
    /// Open diff comments from `annotations.json`.
    pub annotations: bool,
    /// The current diff, fenced.
    pub diff: bool,
}

/// Everything the builder reads. Borrowed because the caller (a Tauri
/// command) already holds all of it; empty strings mean "the file is empty
/// or absent" and the section renders its honest placeholder or is skipped.
#[derive(Debug, Clone, Copy)]
pub struct ShareInput<'a> {
    pub meta: &'a WorkflowMeta,
    pub project: &'a str,
    pub slug: &'a str,
    pub plan: &'a str,
    pub review_md: &'a str,
    pub agent_review_md: &'a str,
    pub annotations: &'a [Annotation],
    pub diff: &'a str,
}

/// One PR line: URL plus whatever state is known. The primary is labeled —
/// it is the one that drives the item; linked PRs ride along.
fn pr_line(pr: &WorkflowPr, primary: bool) -> String {
    let mut bits = Vec::new();
    if primary {
        bits.push("primary".to_string());
    }
    if pr.draft {
        bits.push("draft".to_string());
    }
    match pr.state.as_str() {
        "MERGED" => bits.push("merged".to_string()),
        "CLOSED" => bits.push("closed".to_string()),
        _ => {}
    }
    if let Some(n) = pr.unanswered_comments.filter(|&n| n > 0) {
        bits.push(format!("{} unanswered comment{}", n, plural(n)));
    }
    if bits.is_empty() {
        format!("- {}", pr.url)
    } else {
        format!("- {} ({})", pr.url, bits.join(", "))
    }
}

fn plural(n: u64) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

/// Build the share document. Sections render in a fixed order regardless of
/// how the caller toggled them, so two exports of the same item always read
/// the same way. An all-off `sections` yields the summary anyway — an empty
/// share is never what anyone meant.
pub fn build_share_markdown(input: &ShareInput, sections: &ShareSections) -> String {
    let meta = input.meta;
    let title = if meta.title.trim().is_empty() {
        input.slug
    } else {
        meta.title.trim()
    };
    let mut out = format!("# {}\n\n", title);

    let any = sections.plan
        || sections.timeline
        || sections.reviews
        || sections.annotations
        || sections.diff;
    if sections.summary || !any {
        out.push_str(&format!(
            "**Status:** {} · **Mode:** {} · iteration {}",
            meta.status,
            meta.mode,
            meta.iteration.max(1)
        ));
        if meta.review_round > 0 {
            out.push_str(&format!(
                " · {} agent review round{}",
                meta.review_round,
                plural(meta.review_round as u64)
            ));
        }
        out.push('\n');
        out.push_str(&format!("**Project:** {}", input.project));
        if !meta.branch.trim().is_empty() {
            out.push_str(&format!(" · **Branch:** `{}`", meta.branch.trim()));
            if !meta.base.trim().is_empty() {
                out.push_str(&format!(" (base `{}`)", meta.base.trim()));
            }
        }
        out.push('\n');

        let mut prs = Vec::new();
        if let Some(pr) = meta.pr.as_ref().filter(|p| !p.url.is_empty()) {
            prs.push(pr_line(pr, true));
        }
        for pr in meta.linked_prs.iter().filter(|p| !p.url.is_empty()) {
            prs.push(pr_line(pr, false));
        }
        if !prs.is_empty() {
            out.push_str("\n**Pull requests:**\n");
            out.push_str(&prs.join("\n"));
            out.push('\n');
        }
        out.push('\n');
    }

    if sections.plan {
        out.push_str("## Plan\n\n");
        if input.plan.trim().is_empty() {
            out.push_str("_No plan — this item has no plan phase or none was written yet._\n\n");
        } else {
            out.push_str(input.plan.trim());
            out.push_str("\n\n");
        }
    }

    if sections.timeline {
        let iterations = super::workflow::parse_review_iterations(input.review_md);
        if !iterations.is_empty() {
            out.push_str("## Change rounds\n\n");
            for it in &iterations {
                out.push_str(&format!("### Iteration {}", it.iteration));
                if !it.heading.is_empty() {
                    out.push_str(&format!(" — {}", it.heading));
                }
                out.push_str("\n\n");
                if !it.note.is_empty() {
                    out.push_str(&it.note);
                    out.push_str("\n\n");
                }
                if !it.annotations.is_empty() {
                    out.push_str(&format!(
                        "_{} diff comment{} attached to this round._\n\n",
                        it.annotations.len(),
                        plural(it.annotations.len() as u64)
                    ));
                }
            }
        }
    }

    if sections.reviews {
        let rounds = super::workflow::all_agent_reviews(input.agent_review_md);
        if !rounds.is_empty() {
            out.push_str("## Agent reviews\n\n");
            for r in &rounds {
                out.push_str(&format!("### Round {}", r.round));
                if !r.heading.is_empty() {
                    out.push_str(&format!(" — {}", r.heading));
                }
                out.push_str("\n\n");
                if !r.verdict.is_empty() {
                    out.push_str(&format!("**Verdict:** {}\n\n", r.verdict));
                }
                if !r.published.is_empty() {
                    out.push_str(&format!("Published: {}\n\n", r.published.join(" · ")));
                }
            }
        }
    }

    if sections.annotations {
        let open: Vec<&Annotation> = input
            .annotations
            .iter()
            .filter(|a| a.status == AnnotationStatus::Open)
            .collect();
        if !open.is_empty() {
            out.push_str("## Open comments\n\n");
            for a in open {
                out.push_str(&format!(
                    "- `{}:{}` — {}{}\n",
                    a.file,
                    a.line,
                    a.body.replace('\n', " "),
                    if a.author == "agent" {
                        " _(agent)_"
                    } else {
                        ""
                    }
                ));
            }
            out.push('\n');
        }
    }

    if sections.diff {
        out.push_str("## Diff\n\n");
        if input.diff.trim().is_empty() {
            out.push_str("_No diff — nothing changed yet against the base._\n");
        } else {
            // Four backticks so diffs containing ``` fences stay inside.
            out.push_str("````diff\n");
            out.push_str(input.diff.trim_end());
            out.push_str("\n````\n");
        }
    }

    out.trim_end().to_string() + "\n"
}

/// The kickoff for a share handed to a Claude Code session.
///
/// Pure and tested because it is the whole instruction: clash is delegating an
/// outward-facing post to a session it will not supervise, so the prompt has
/// to name the destination and the payload, and say plainly that the document
/// is the message. "Summarize this for Slack" is how a share stops being the
/// thing the human previewed.
///
/// A skill is **optional**, and the fallback is the point: with one named the
/// prompt routes through it and still allows the session's own tooling if that
/// skill is not installed here; with none, the session is told to use whatever
/// it has connected — an MCP server for the destination, or the CLI it would
/// normally reach for. Requiring a skill would have made "I already have MCP
/// access to this" the one case clash could not serve.
pub fn share_prompt(
    skill: Option<&str>,
    destination: &str,
    payload_path: &str,
    title: &str,
    ticket: Option<&str>,
) -> String {
    let where_to = match destination {
        "jira" => match ticket {
            Some(key) => format!("Jira ticket {}", key),
            None => "Jira".to_string(),
        },
        other => other.to_string(),
    };
    let tools = format!(
        "use whatever tooling you have connected that can reach {destination} — an MCP \
         server for it, or the CLI you would normally use"
    );
    let how = match skill.map(str::trim).filter(|s| !s.is_empty()) {
        Some(skill) => format!(
            "Use the {skill} skill to post it. If that skill is not available in this \
             session, {tools}, and say which route you took."
        ),
        None => format!("To post it, {tools}."),
    };
    format!(
        "Post the document at {payload_path} to {where_to}. \
         It is the shared record of the clash workflow item \"{title}\". \
         {how} \
         The document IS the message: post it as written — do not summarize it, \
         re-order it, or add commentary of your own. Adapt only the formatting the \
         destination requires. If nothing here can reach {where_to}, say so and stop \
         rather than posting something else. When you are done, report exactly where \
         it landed (a URL if there is one)."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::workflow::{WorkflowMode, WorkflowStatus};

    fn meta() -> WorkflowMeta {
        WorkflowMeta {
            title: "Auth refactor".into(),
            status: WorkflowStatus::DiffReview,
            mode: WorkflowMode::Full,
            branch: "auth-refactor".into(),
            base: "main".into(),
            iteration: 3,
            review_round: 2,
            pr: Some(WorkflowPr {
                url: "https://github.com/o/r/pull/7".into(),
                number: 7,
                draft: true,
                ..Default::default()
            }),
            linked_prs: vec![WorkflowPr {
                url: "https://github.com/o/front/pull/12".into(),
                number: 12,
                state: "MERGED".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn input<'a>(m: &'a WorkflowMeta) -> ShareInput<'a> {
        ShareInput {
            meta: m,
            project: "clash",
            slug: "auth-refactor",
            plan: "Do the thing.\n\n- step one",
            review_md: "## Iteration 1 — 2026-08-01 10:00\n\nTighten the API.\n\n### Open annotations\n\n- `a.rs:1` — x\n",
            agent_review_md: "## Review 2 — diff · deep · 2026-08-04\n\n**Verdict:** ship it\n\n### Published\n\n- nothing\n",
            annotations: &[],
            diff: "diff --git a/a.rs b/a.rs\n+new line\n",
        }
    }

    #[test]
    fn summary_carries_identity_status_and_every_pr() {
        let m = meta();
        let md = build_share_markdown(
            &input(&m),
            &ShareSections {
                summary: true,
                ..Default::default()
            },
        );
        assert!(md.starts_with("# Auth refactor\n"));
        assert!(md.contains("**Status:** diff-review"));
        assert!(md.contains("iteration 3"));
        assert!(md.contains("2 agent review rounds"));
        assert!(md.contains("`auth-refactor` (base `main`)"));
        // Both PRs, with their roles and states.
        assert!(md.contains("- https://github.com/o/r/pull/7 (primary, draft)"));
        assert!(md.contains("- https://github.com/o/front/pull/12 (merged)"));
        // Nothing else leaked in.
        assert!(!md.contains("## Plan"));
        assert!(!md.contains("## Diff"));
    }

    #[test]
    fn sections_render_in_fixed_order_when_all_on() {
        let m = meta();
        let all = ShareSections {
            summary: true,
            plan: true,
            timeline: true,
            reviews: true,
            annotations: true,
            diff: true,
        };
        let mut inp = input(&m);
        let anns = [Annotation {
            file: "src/a.rs".into(),
            line: 12,
            body: "rename\nthis".into(),
            author: "agent".into(),
            ..Default::default()
        }];
        inp.annotations = &anns;
        let md = build_share_markdown(&inp, &all);
        let order: Vec<usize> = [
            "## Plan",
            "## Change rounds",
            "## Agent reviews",
            "## Open comments",
            "## Diff",
        ]
        .iter()
        .map(|h| md.find(h).unwrap_or_else(|| panic!("{} missing", h)))
        .collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "sections out of order"
        );
        // Newlines in a comment body collapse — one bullet per comment.
        assert!(md.contains("- `src/a.rs:12` — rename this _(agent)_"));
        // The diff is fenced with four backticks so inner fences survive.
        assert!(md.contains("````diff\n"));
        assert!(md.contains("**Verdict:** ship it"));
        assert!(md.contains("### Iteration 1 — 2026-08-01 10:00"));
        assert!(md.contains("_1 diff comment attached to this round._"));
    }

    #[test]
    fn an_all_off_selection_still_yields_the_summary() {
        let m = meta();
        let md = build_share_markdown(&input(&m), &ShareSections::default());
        assert!(md.contains("**Status:** diff-review"));
    }

    #[test]
    fn empty_files_render_placeholders_not_lies() {
        let m = WorkflowMeta {
            status: WorkflowStatus::PlanReview,
            ..Default::default()
        };
        let inp = ShareInput {
            meta: &m,
            project: "p",
            slug: "untitled-item",
            plan: "  ",
            review_md: "",
            agent_review_md: "",
            annotations: &[],
            diff: "",
        };
        let md = build_share_markdown(
            &inp,
            &ShareSections {
                summary: true,
                plan: true,
                timeline: true,
                reviews: true,
                annotations: true,
                diff: true,
            },
        );
        // Untitled falls back to the slug — the identity, like the GUI does.
        assert!(md.starts_with("# untitled-item\n"));
        assert!(md.contains("_No plan —"));
        assert!(md.contains("_No diff —"));
        // Empty change-rounds/reviews/comments sections are skipped outright.
        assert!(!md.contains("## Change rounds"));
        assert!(!md.contains("## Agent reviews"));
        assert!(!md.contains("## Open comments"));
    }

    #[test]
    fn parked_and_resolved_annotations_stay_out() {
        let m = meta();
        let anns = [
            Annotation {
                file: "a.rs".into(),
                line: 1,
                body: "open one".into(),
                status: AnnotationStatus::Open,
                ..Default::default()
            },
            Annotation {
                file: "a.rs".into(),
                line: 2,
                body: "parked one".into(),
                status: AnnotationStatus::Parked,
                ..Default::default()
            },
            Annotation {
                file: "a.rs".into(),
                line: 3,
                body: "done one".into(),
                status: AnnotationStatus::Addressed,
                ..Default::default()
            },
        ];
        let mut inp = input(&m);
        inp.annotations = &anns;
        let md = build_share_markdown(
            &inp,
            &ShareSections {
                annotations: true,
                ..Default::default()
            },
        );
        assert!(md.contains("open one"));
        assert!(!md.contains("parked one"));
        assert!(!md.contains("done one"));
    }

    // ── share_prompt ─────────────────────────────────────────────────

    #[test]
    fn the_prompt_names_the_destination_and_the_payload() {
        let p = share_prompt(
            Some("myorg:jira-post"),
            "jira",
            "/data/share/p-item-1.md",
            "Auth refactor",
            Some("PROJ-12"),
        );
        assert!(p.contains("/data/share/p-item-1.md"));
        assert!(p.contains("Jira ticket PROJ-12"));
        assert!(p.contains("Auth refactor"));
        assert!(p.contains("Use the myorg:jira-post skill"));
        // The instruction that keeps a share the thing the human previewed.
        assert!(p.contains("do not summarize"));
    }

    #[test]
    fn without_a_skill_the_session_uses_what_it_has_connected() {
        // The case clash could not serve while a skill was mandatory: MCP
        // access already in the session and no skill wrapping it.
        let p = share_prompt(None, "slack", "/tmp/x.md", "Item", None);
        assert!(!p.contains("skill"));
        assert!(p.contains("To post it, use whatever tooling you have connected"));
        assert!(p.contains("an MCP server for it"));
        assert!(p.contains("to slack."));
        // An empty skill name is the same as none, not a skill called "".
        assert_eq!(
            p,
            share_prompt(Some("  "), "slack", "/tmp/x.md", "Item", None)
        );
    }

    #[test]
    fn a_named_skill_still_allows_the_sessions_own_tooling() {
        // A skill named in Settings but not installed in this session must not
        // dead-end the share — same fallback the PR skill has.
        let p = share_prompt(Some("myorg:chat"), "discord", "/tmp/x.md", "Item", None);
        assert!(p.contains("If that skill is not available in this session"));
        assert!(p.contains("say which route you took"));
    }

    #[test]
    fn a_chat_destination_needs_no_ticket() {
        let p = share_prompt(Some("s"), "slack", "/tmp/x.md", "Item", None);
        assert!(!p.contains("ticket"));
        // Jira without a key still says Jira rather than inventing one.
        let j = share_prompt(None, "jira", "/tmp/x.md", "Item", None);
        assert!(j.contains("to Jira."));
    }
}
