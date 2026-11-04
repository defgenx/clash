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
    /// The current diff, one fenced block per file.
    pub diff: bool,
    /// The plan explanation's write-up (`explain-plan.md`).
    pub explain_plan: bool,
    /// The change explanation's write-up (`explain-diff.md`).
    pub explain_diff: bool,
    /// The plan-vs-changes report (`drift.md`).
    pub drift: bool,
    /// The executor's latest hand-off note (`handoff.md`): where it stands.
    pub handoff: bool,
    /// A linked contents list, back-links and comment → diff links. Off for
    /// a chat post, where anchors are dead text.
    pub toc: bool,
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
    pub explain_plan: &'a str,
    pub explain_diff: &'a str,
    pub drift: &'a str,
    pub handoff: &'a str,
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

/// A heading of the finished document: level, text and anchor slug.
type Anchored = (usize, String, String);

/// One `##` section of the share document: its heading and its markdown.
struct Section {
    title: &'static str,
    body: String,
}

/// Build the share document. Sections render in a fixed order regardless of
/// how the caller toggled them, so two exports of the same item always read
/// the same way. An all-off `sections` yields the summary anyway — an empty
/// share is never what anyone meant.
///
/// With two sections or more the document opens on a **Contents** list and
/// is cross-linked: every section links back to it, the diff is one heading
/// per file, and each open comment links to its file's diff. The anchors are
/// GitHub's heading slugs ([`heading_slug`]), so they work on GitHub, in any
/// renderer that follows it, in the GUI preview and in the `.html` export,
/// which assign ids with the same rule (`shareHeadingSlugs` in wf-share.js).
pub fn build_share_markdown(input: &ShareInput, sections: &ShareSections) -> String {
    let meta = input.meta;
    let title = if meta.title.trim().is_empty() {
        input.slug
    } else {
        meta.title.trim()
    };
    let mut head = format!("# {}\n\n", title);

    let any = sections.plan
        || sections.timeline
        || sections.reviews
        || sections.annotations
        || sections.diff
        || sections.explain_plan
        || sections.explain_diff
        || sections.drift
        || sections.handoff;
    if sections.summary || !any {
        head.push_str(&format!(
            "**Status:** {} · **Mode:** {} · iteration {}",
            meta.status,
            meta.mode,
            meta.iteration.max(1)
        ));
        if meta.review_round > 0 {
            head.push_str(&format!(
                " · {} agent review round{}",
                meta.review_round,
                plural(meta.review_round as u64)
            ));
        }
        head.push('\n');
        head.push_str(&format!("**Project:** {}", input.project));
        if !meta.branch.trim().is_empty() {
            head.push_str(&format!(" · **Branch:** `{}`", meta.branch.trim()));
            if !meta.base.trim().is_empty() {
                head.push_str(&format!(" (base `{}`)", meta.base.trim()));
            }
        }
        head.push('\n');

        let mut prs = Vec::new();
        if let Some(pr) = meta.pr.as_ref().filter(|p| !p.url.is_empty()) {
            prs.push(pr_line(pr, true));
        }
        for pr in meta.linked_prs.iter().filter(|p| !p.url.is_empty()) {
            prs.push(pr_line(pr, false));
        }
        if !prs.is_empty() {
            head.push_str("\n**Pull requests:**\n");
            head.push_str(&prs.join("\n"));
            head.push('\n');
        }
        head.push('\n');
    }

    let mut body: Vec<Section> = Vec::new();
    push_doc(
        &mut body,
        sections.handoff,
        "Where it stands",
        input.handoff,
    );

    if sections.plan {
        let text = if input.plan.trim().is_empty() {
            "_No plan — this item has no plan phase or none was written yet._".to_string()
        } else {
            // The plan's own headings sit under this section, not beside it.
            demote_headings(input.plan.trim(), 2)
        };
        body.push(Section {
            title: "Plan",
            body: text,
        });
    }

    push_doc(
        &mut body,
        sections.explain_plan,
        "Plan explained",
        input.explain_plan,
    );
    push_doc(
        &mut body,
        sections.explain_diff,
        "Changes explained",
        input.explain_diff,
    );
    push_doc(&mut body, sections.drift, "Plan vs changes", input.drift);

    if sections.timeline {
        let iterations = super::workflow::parse_review_iterations(input.review_md);
        if !iterations.is_empty() {
            let mut text = String::new();
            for it in &iterations {
                text.push_str(&format!("### Iteration {}", it.iteration));
                if !it.heading.is_empty() {
                    text.push_str(&format!(" — {}", it.heading));
                }
                text.push_str("\n\n");
                if !it.note.is_empty() {
                    text.push_str(&demote_headings(&it.note, 2));
                    text.push_str("\n\n");
                }
                if !it.annotations.is_empty() {
                    text.push_str(&format!(
                        "_{} diff comment{} attached to this round._\n\n",
                        it.annotations.len(),
                        plural(it.annotations.len() as u64)
                    ));
                }
            }
            body.push(Section {
                title: "Change rounds",
                body: text,
            });
        }
    }

    if sections.reviews {
        let rounds = super::workflow::all_agent_reviews(input.agent_review_md);
        if !rounds.is_empty() {
            let mut text = String::new();
            for r in &rounds {
                text.push_str(&format!("### Round {}", r.round));
                if !r.heading.is_empty() {
                    text.push_str(&format!(" — {}", r.heading));
                }
                text.push_str("\n\n");
                if !r.verdict.is_empty() {
                    text.push_str(&format!("**Verdict:** {}\n\n", r.verdict));
                }
                if !r.published.is_empty() {
                    text.push_str(&format!("Published: {}\n\n", r.published.join(" · ")));
                }
            }
            body.push(Section {
                title: "Agent reviews",
                body: text,
            });
        }
    }

    let open: Vec<&Annotation> = input
        .annotations
        .iter()
        .filter(|a| a.status == AnnotationStatus::Open)
        .collect();
    // Filled once the anchors are known: its links point into the diff.
    let comments_at = (sections.annotations && !open.is_empty()).then(|| {
        body.push(Section {
            title: "Open comments",
            body: String::new(),
        });
        body.len() - 1
    });

    let mut diff_files: Vec<String> = Vec::new();
    if sections.diff {
        let text = if input.diff.trim().is_empty() {
            "_No diff — nothing changed yet against the base._".to_string()
        } else {
            let files = split_diff_by_file(input.diff);
            let mut text = String::new();
            for (path, chunk) in &files {
                if let Some(path) = path {
                    text.push_str(&format!("### `{}`\n\n", path));
                    diff_files.push(path.clone());
                }
                // Four backticks so diffs containing ``` fences stay inside.
                text.push_str("````diff\n");
                text.push_str(chunk.trim_end());
                text.push_str("\n````\n\n");
            }
            text
        };
        body.push(Section {
            title: "Diff",
            body: text,
        });
    }

    let toc = sections.toc && body.len() >= 2;
    // Every heading of the finished document, in order — the slugs depend on
    // all of them, because a repeated heading gets `-1`, `-2`…
    let mut slugger = Slugger::default();
    for (_, t) in markdown_headings(&head) {
        slugger.slug(&t);
    }
    let contents_slug = toc.then(|| slugger.slug("Contents"));
    let mut section_slugs: Vec<(String, Vec<Anchored>)> = Vec::new();
    for sec in &body {
        let own = slugger.slug(sec.title);
        let subs = markdown_headings(&sec.body)
            .into_iter()
            .map(|(lvl, t)| {
                let s = slugger.slug(&t);
                (lvl, t, s)
            })
            .collect();
        section_slugs.push((own, subs));
    }

    if let Some(i) = comments_at {
        // Links only where the reader can follow them: a contents-less post
        // (a chat message) gets plain locations.
        let file_slug = |file: &str| -> Option<&String> {
            let (_, subs) = section_slugs.last()?;
            if !sections.diff || !toc {
                return None;
            }
            diff_files
                .iter()
                .position(|f| f == file)
                .and_then(|k| subs.get(k).map(|(_, _, s)| s))
        };
        let mut text = String::new();
        for a in &open {
            let loc = format!("`{}:{}`", a.file, a.line);
            let loc = match file_slug(&a.file) {
                Some(slug) => format!("[{}](#{})", loc, slug),
                None => loc,
            };
            text.push_str(&format!(
                "- {} — {}{}\n",
                loc,
                a.body.replace('\n', " "),
                if a.author == "agent" {
                    " _(agent)_"
                } else {
                    ""
                }
            ));
        }
        body[i].body = text;
    }

    let mut out = head;
    if let Some(contents) = &contents_slug {
        out.push_str("## Contents\n\n");
        for (sec, (slug, subs)) in body.iter().zip(&section_slugs) {
            out.push_str(&format!("- [{}](#{})\n", sec.title, slug));
            for (title, slug) in toc_children(subs) {
                out.push_str(&format!("  - [{}](#{})\n", toc_label(&title), slug));
            }
        }
        out.push('\n');
        for sec in &mut body {
            let trimmed = sec.body.trim_end().to_string();
            sec.body = format!("{}\n\n[↑ Contents](#{})", trimmed, contents);
        }
    }
    for sec in &body {
        out.push_str(&format!("## {}\n\n", sec.title));
        out.push_str(sec.body.trim_end());
        out.push_str("\n\n");
    }

    out.trim_end().to_string() + "\n"
}

/// An agent-written document as its own section, its headings pushed under
/// it — or nothing when the agent never wrote it: an empty heading says less
/// than no heading.
fn push_doc(body: &mut Vec<Section>, on: bool, title: &'static str, text: &str) {
    if on && !text.trim().is_empty() {
        body.push(Section {
            title,
            body: demote_headings(text.trim(), 2),
        });
    }
}

/// Most entries one section lists in the contents; a 300-file diff would
/// otherwise bury everything after it.
const TOC_CHILDREN_MAX: usize = 25;

/// The entries a section lists under it in the contents: its shallowest
/// headings, or the level below when there is only one of those (a plan
/// whose `# Title` sits above its real sections).
fn toc_children(subs: &[Anchored]) -> Vec<(String, String)> {
    let Some(min) = subs.iter().map(|(l, _, _)| *l).min() else {
        return Vec::new();
    };
    let at = |lvl: usize| subs.iter().filter(move |(l, _, _)| *l == lvl);
    let level = if at(min).count() == 1 && subs.iter().any(|(l, _, _)| *l == min + 1) {
        min + 1
    } else {
        min
    };
    let all: Vec<(String, String)> = at(level).map(|(_, t, s)| (t.clone(), s.clone())).collect();
    if all.len() <= TOC_CHILDREN_MAX {
        return all;
    }
    let more = all.len() - TOC_CHILDREN_MAX;
    let mut out: Vec<(String, String)> = all.into_iter().take(TOC_CHILDREN_MAX).collect();
    // Points at the section's last listed entry: the rest follow it.
    let last = out.last().map(|(_, s)| s.clone()).unwrap_or_default();
    out.push((format!("… {} more", more), last));
    out
}

/// A contents entry's text: a heading's code spans keep their backticks, but
/// a `]` would end the link early.
fn toc_label(title: &str) -> String {
    title.replace('[', "\\[").replace(']', "\\]")
}

/// Split a unified diff into one chunk per file, keyed by the file's path
/// (`b/` side, or `a/` for a deletion). Text before the first `diff --git`
/// line, or a diff without any, is one chunk with no path.
fn split_diff_by_file(diff: &str) -> Vec<(Option<String>, String)> {
    let mut out: Vec<(Option<String>, String)> = Vec::new();
    for line in diff.split_inclusive('\n') {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            out.push((diff_path(rest.trim_end()), String::new()));
        } else if out.is_empty() {
            out.push((None, String::new()));
        }
        out.last_mut().expect("pushed above").1.push_str(line);
    }
    out.retain(|(p, c)| p.is_some() || !c.trim().is_empty());
    out
}

/// `a/src/x.rs b/src/x.rs` → `src/x.rs`.
fn diff_path(spec: &str) -> Option<String> {
    let b = spec.rfind(" b/").map(|i| &spec[i + 3..]);
    let a = spec.strip_prefix("a/").and_then(|r| r.split(" b/").next());
    b.or(a).map(str::to_string).filter(|p| !p.is_empty())
}

/// Every ATX heading outside fenced code: (level, text).
fn markdown_headings(md: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in md.lines() {
        let t = line.trim_start();
        if let Some(f) = fence_marker(t) {
            match fence {
                None => fence = Some(f),
                Some((c, n)) if f.0 == c && f.1 >= n && t.trim_end().chars().all(|x| x == c) => {
                    fence = None
                }
                _ => {}
            }
            continue;
        }
        if fence.is_some() || line.len() - t.len() > 3 {
            continue;
        }
        if let Some((level, text)) = atx_heading(t) {
            out.push((level, text));
        }
    }
    out
}

/// A fence opener/closer: three or more backticks or tildes.
fn fence_marker(t: &str) -> Option<(char, usize)> {
    let c = t.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let n = t.chars().take_while(|x| *x == c).count();
    (n >= 3).then_some((c, n))
}

fn atx_heading(t: &str) -> Option<(usize, String)> {
    let level = t.chars().take_while(|c| *c == '#').count();
    if level == 0 || level > 6 {
        return None;
    }
    let rest = &t[level..];
    if !(rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')) {
        return None;
    }
    let text = rest.trim().trim_end_matches('#').trim_end().to_string();
    Some((level, text))
}

/// Push every ATX heading outside fences `by` levels deeper (capped at 6), so
/// a document pasted under a `##` section stays inside it.
fn demote_headings(md: &str, by: usize) -> String {
    let mut out = String::with_capacity(md.len() + 16);
    let mut fence: Option<(char, usize)> = None;
    for line in md.split_inclusive('\n') {
        let t = line.trim_start();
        if let Some(f) = fence_marker(t) {
            match fence {
                None => fence = Some(f),
                Some((c, n)) if f.0 == c && f.1 >= n && t.trim_end().chars().all(|x| x == c) => {
                    fence = None
                }
                _ => {}
            }
        } else if fence.is_none() && line.len() - t.len() <= 3 {
            if let Some((level, _)) = atx_heading(t.trim_end()) {
                let to = (level + by).min(6);
                out.push_str(&"#".repeat(to));
                out.push_str(&t[level..]);
                continue;
            }
        }
        out.push_str(line);
    }
    out
}

/// GitHub's heading anchor for `text`: the rendered text lower-cased, every
/// character that is not a letter, digit, `-`, `_` or space dropped, spaces
/// turned into `-`. Inline markup is rendered away first (code backticks,
/// emphasis stars, link syntax). Mirrored by `shareHeadingSlug` in
/// `gui/dist/wf-share.js`; the two must agree or the links go nowhere.
pub fn heading_slug(text: &str) -> String {
    rendered_text(text)
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// `[text](url)` → `text`, and backticks and stars dropped.
fn rendered_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        match after
            .find("](")
            .and_then(|m| after[m + 2..].find(')').map(|c| (m, c)))
        {
            Some((m, c)) => {
                out.push_str(&rest[..open]);
                out.push_str(&after[..m]);
                rest = &after[m + 2 + c + 1..];
            }
            None => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.replace(['`', '*'], "")
}

/// Deduplicates slugs the way GitHub does: the second `Plan` is `plan-1`.
#[derive(Default)]
struct Slugger {
    seen: std::collections::HashMap<String, usize>,
}

impl Slugger {
    fn slug(&mut self, text: &str) -> String {
        let base = heading_slug(text);
        let mut slug = base.clone();
        while self.seen.contains_key(&slug) {
            let n = self.seen.get_mut(&base).expect("base was seen first");
            *n += 1;
            slug = format!("{}-{}", base, n);
        }
        self.seen.insert(slug.clone(), 0);
        slug
    }
}

/// The export's file name without extension: `clash-<project>-<slug>`, ASCII
/// only, so nothing in an item's name can become a path separator.
pub fn share_file_stem(project: &str, slug: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    };
    format!("clash-{}-{}", clean(project), clean(slug))
}

/// The path a save dialog returned, made safe to export to: absolute, no
/// `..`, a real file name, ending in `.<ext>` (appended when it does not).
/// Whether the folder exists is the writer's check.
pub fn validate_export_path(path: &str, ext: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(path.trim());
    if !p.is_absolute() {
        return Err(format!("Not an absolute path: {}", path));
    }
    if p.components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(format!("Refusing a path with '..': {}", path));
    }
    let name = p
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.trim().is_empty())
        .ok_or_else(|| format!("No file name in {}", path))?;
    if name
        .rsplit_once('.')
        .is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext))
    {
        Ok(p.to_path_buf())
    } else {
        Ok(p.with_file_name(format!("{}.{}", name, ext)))
    }
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
            explain_plan: "",
            explain_diff: "",
            drift: "",
            handoff: "",
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
            toc: true,
            ..Default::default()
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
        // Newlines in a comment body collapse — one bullet per comment. The
        // diff has no `src/a.rs`, so the location stays plain.
        assert!(md.contains("- `src/a.rs:12` — rename this _(agent)_"));
        // The diff is fenced with four backticks so inner fences survive,
        // one block per file under the file's own heading.
        assert!(md.contains("### `a.rs`\n\n````diff\n"));
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
            explain_plan: "",
            explain_diff: "",
            drift: "",
            handoff: "",
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
                ..Default::default()
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

    #[test]
    fn clash_bookkeeping_never_reaches_a_share() {
        // The session id lives in meta.json; nothing the document is built
        // from may carry it out, whatever is selected.
        let mut m = meta();
        m.session_id = Some("0199-secret-session".into());
        let all = ShareSections {
            summary: true,
            plan: true,
            timeline: true,
            reviews: true,
            annotations: true,
            diff: true,
            toc: true,
            ..Default::default()
        };
        let md = build_share_markdown(&input(&m), &all);
        assert!(!md.contains("0199-secret-session"));
    }

    #[test]
    fn export_names_and_paths() {
        assert_eq!(
            share_file_stem("Update Ruby SDK", "a/../b"),
            "clash-Update-Ruby-SDK-a----b"
        );
        let ok = |p: &str, ext: &str| validate_export_path(p, ext).unwrap();
        assert_eq!(
            ok("/tmp/out/item", "html"),
            std::path::PathBuf::from("/tmp/out/item.html")
        );
        assert_eq!(
            ok("/tmp/out/item.HTML", "html"),
            std::path::PathBuf::from("/tmp/out/item.HTML")
        );
        assert_eq!(
            ok("/tmp/x.md", "html"),
            std::path::PathBuf::from("/tmp/x.md.html")
        );
        assert!(validate_export_path("relative.html", "html").is_err());
        assert!(validate_export_path("/tmp/../etc/x.html", "html").is_err());
        assert!(validate_export_path("/", "html").is_err());
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

    #[test]
    fn slugs_follow_github() {
        assert_eq!(heading_slug("Change rounds"), "change-rounds");
        assert_eq!(
            heading_slug("Iteration 1 — 2026-08-01 10:00"),
            "iteration-1--2026-08-01-1000"
        );
        assert_eq!(heading_slug("`src/a.rs`"), "srcars");
        assert_eq!(
            heading_slug("Use [the API](https://x.y) **now**"),
            "use-the-api-now"
        );
        assert_eq!(heading_slug("Déjà vu_2"), "déjà-vu_2");
        let mut s = Slugger::default();
        assert_eq!(s.slug("Plan"), "plan");
        assert_eq!(s.slug("Plan"), "plan-1");
        assert_eq!(s.slug("Plan 1"), "plan-1-1");
        assert_eq!(s.slug("Plan"), "plan-2");
    }

    #[test]
    fn a_multi_section_share_opens_on_linked_contents() {
        let m = meta();
        let mut inp = input(&m);
        inp.plan =
            "# The plan\n\n## Step one\n\nx\n\n## Step two\n\n```md\n## not a heading\n```\n";
        inp.diff = "diff --git a/src/a.rs b/src/a.rs\n+x\ndiff --git a/b.rs b/b.rs\n-y\n";
        inp.explain_diff = "# What this change does\n\n## At a glance\n\nit works\n";
        inp.handoff = "Implemented; tests pass.\n";
        let anns = [Annotation {
            file: "src/a.rs".into(),
            line: 4,
            body: "check".into(),
            ..Default::default()
        }];
        inp.annotations = &anns;
        let md = build_share_markdown(
            &inp,
            &ShareSections {
                summary: true,
                plan: true,
                annotations: true,
                diff: true,
                explain_diff: true,
                handoff: true,
                toc: true,
                ..Default::default()
            },
        );
        let toc_at = md.find("## Contents").expect("contents");
        assert!(toc_at < md.find("## Where it stands").unwrap(), "{md}");
        // Every section is listed and linked, in order, with its parts.
        for line in [
            "- [Where it stands](#where-it-stands)",
            "- [Plan](#plan)",
            "  - [Step one](#step-one)",
            "  - [Step two](#step-two)",
            "- [Changes explained](#changes-explained)",
            "  - [At a glance](#at-a-glance)",
            "- [Open comments](#open-comments)",
            "- [Diff](#diff)",
            "  - [`src/a.rs`](#srcars)",
            "  - [`b.rs`](#brs)",
        ] {
            assert!(md.contains(line), "missing {line:?} in\n{md}");
        }
        // The plan's headings sit under its section; a fenced `##` is code.
        assert!(md.contains("### The plan\n"));
        assert!(md.contains("#### Step one"));
        assert!(md.contains("```md\n## not a heading\n```"));
        // A comment links to its file's diff; every section leads back.
        assert!(md.contains("- [`src/a.rs:4`](#srcars) — check"), "{md}");
        assert_eq!(md.matches("[↑ Contents](#contents)").count(), 5);
    }

    #[test]
    fn without_contents_nothing_is_linked() {
        let m = meta();
        let mut inp = input(&m);
        let anns = [Annotation {
            file: "a.rs".into(),
            line: 1,
            body: "check".into(),
            ..Default::default()
        }];
        inp.annotations = &anns;
        let sections = ShareSections {
            summary: true,
            annotations: true,
            diff: true,
            ..Default::default()
        };
        let md = build_share_markdown(&inp, &sections);
        assert!(!md.contains("## Contents"));
        assert!(!md.contains("](#"), "{md}");
        // One section is no reason for contents, even when asked.
        let md = build_share_markdown(
            &inp,
            &ShareSections {
                plan: true,
                toc: true,
                ..Default::default()
            },
        );
        assert!(!md.contains("## Contents"));
    }

    #[test]
    fn an_unwritten_document_adds_no_section() {
        let m = meta();
        let md = build_share_markdown(
            &input(&m),
            &ShareSections {
                explain_plan: true,
                drift: true,
                handoff: true,
                ..Default::default()
            },
        );
        assert!(!md.contains("## Plan explained"));
        assert!(!md.contains("## Plan vs changes"));
        assert!(!md.contains("## Where it stands"));
    }
}
