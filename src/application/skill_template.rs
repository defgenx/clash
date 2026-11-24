//! Per-harness rendering of clash's embedded skills. Pure.
//!
//! One `skills/<name>/SKILL.md` serves every agent CLI clash runs; what
//! differs between them is written as markers that this module resolves for
//! one [`AgentKind`]:
//!
//! - `{{token}}` — replaced inline (see [`TOKENS`]).
//! - a line `<!-- clash:if <kinds…> -->` … a line `<!-- clash:end -->` — kept
//!   only for the listed kinds (`claude`, `omp`, `hermes`), markers dropped.
//!   Blocks do not nest.
//!
//! Syntax, the token table and how to add a harness: `docs/workflows.md` →
//! *Skills per agent CLI*.

use crate::domain::entities::AgentKind;

/// Every harness a skill is rendered for, in display order.
pub const HARNESSES: [AgentKind; 3] = [AgentKind::Claude, AgentKind::Omp, AgentKind::Hermes];

/// Inline tokens: `{{name}}` → the value for each harness, in [`HARNESSES`] order.
pub const TOKENS: &[(&str, [&str; 3])] = &[
    // The structured-question tool.
    ("ask", ["AskUserQuestion", "ask", "clarify"]),
    // Turning on several answers in one question.
    (
        "multi",
        ["multiSelect: true", "multi: true", "multi_select: true"],
    ),
    // The subagent tool.
    ("subagent", ["Agent", "task", "delegate_task"]),
    // How the agent CLI is named in prose.
    ("harness", ["Claude Code", "OMP", "Hermes"]),
];

const IF_OPEN: &str = "<!-- clash:if ";
const MARK_CLOSE: &str = "-->";
const END: &str = "<!-- clash:end -->";

/// One rendered line, with whether its text depends on the harness — the
/// Skills tab marks those lines so a reader sees what the switch changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedLine {
    pub text: String,
    pub varies: bool,
}

fn index(kind: AgentKind) -> usize {
    HARNESSES
        .iter()
        .position(|k| *k == kind)
        .expect("every AgentKind is in HARNESSES")
}

fn parse_kinds(spec: &str, line_no: usize) -> Result<Vec<AgentKind>, String> {
    let mut kinds = Vec::new();
    for word in spec.split_whitespace() {
        // `AgentKind::parse` falls back to Claude on junk, so match exactly.
        let kind = HARNESSES
            .iter()
            .copied()
            .find(|k| k.as_str() == word)
            .ok_or_else(|| format!("line {line_no}: unknown harness `{word}` in clash:if"))?;
        kinds.push(kind);
    }
    if kinds.is_empty() {
        return Err(format!("line {line_no}: clash:if names no harness"));
    }
    Ok(kinds)
}

/// Replace every `{{token}}` on one line; returns the text and whether any
/// token was expanded (every token differs between harnesses).
fn expand(line: &str, kind: AgentKind, line_no: usize) -> Result<(String, bool), String> {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    let mut expanded = false;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .ok_or_else(|| format!("line {line_no}: unclosed `{{{{`"))?;
        let name = &after[..end];
        let (_, values) = TOKENS
            .iter()
            .find(|(t, _)| *t == name)
            .ok_or_else(|| format!("line {line_no}: unknown token `{{{{{name}}}}}`"))?;
        out.push_str(values[index(kind)]);
        expanded = true;
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    Ok((out, expanded))
}

/// Render a template for one harness, line by line, keeping which lines vary.
pub fn render_lines(template: &str, kind: AgentKind) -> Result<Vec<RenderedLine>, String> {
    let mut out = Vec::new();
    // `Some(kept)` while inside a block: whether this harness keeps it.
    let mut block: Option<bool> = None;
    for (i, line) in template.split('\n').enumerate() {
        let n = i + 1;
        let trimmed = line.trim();
        if let Some(spec) = trimmed.strip_prefix(IF_OPEN) {
            if block.is_some() {
                return Err(format!("line {n}: clash:if blocks do not nest"));
            }
            let spec = spec
                .strip_suffix(MARK_CLOSE)
                .ok_or_else(|| format!("line {n}: clash:if must end with `-->`"))?;
            block = Some(parse_kinds(spec, n)?.contains(&kind));
            continue;
        }
        if trimmed == END {
            if block.take().is_none() {
                return Err(format!("line {n}: clash:end without clash:if"));
            }
            continue;
        }
        if trimmed.contains("clash:if") || trimmed.contains("clash:end") {
            return Err(format!(
                "line {n}: a clash marker must be alone on its line"
            ));
        }
        if block == Some(false) {
            continue;
        }
        let (text, expanded) = expand(line, kind, n)?;
        out.push(RenderedLine {
            text,
            varies: block.is_some() || expanded,
        });
    }
    if block.is_some() {
        return Err("clash:if is never closed".to_string());
    }
    Ok(out)
}

/// Render a template for one harness.
pub fn render(template: &str, kind: AgentKind) -> Result<String, String> {
    Ok(render_lines(template, kind)?
        .into_iter()
        .map(|l| l.text)
        .collect::<Vec<_>>()
        .join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: &str = "---\nname: x\n---\nAsk with `{{ask}}`.\n<!-- clash:if hermes -->\nHermes only.\n<!-- clash:end -->\n<!-- clash:if claude omp -->\nNot Hermes.\n<!-- clash:end -->\nShared.\n";

    #[test]
    fn tokens_and_blocks_resolve_per_harness() {
        assert_eq!(
            render(T, AgentKind::Claude).unwrap(),
            "---\nname: x\n---\nAsk with `AskUserQuestion`.\nNot Hermes.\nShared.\n"
        );
        assert_eq!(
            render(T, AgentKind::Omp).unwrap(),
            "---\nname: x\n---\nAsk with `ask`.\nNot Hermes.\nShared.\n"
        );
        assert_eq!(
            render(T, AgentKind::Hermes).unwrap(),
            "---\nname: x\n---\nAsk with `clarify`.\nHermes only.\nShared.\n"
        );
    }

    #[test]
    fn a_template_without_markers_renders_unchanged() {
        let plain = "---\nname: x\n---\nbody\n";
        for kind in HARNESSES {
            assert_eq!(render(plain, kind).unwrap(), plain);
        }
    }

    #[test]
    fn varying_lines_are_the_tokens_and_the_blocks() {
        let lines = render_lines(T, AgentKind::Hermes).unwrap();
        let varies: Vec<(&str, bool)> = lines.iter().map(|l| (l.text.as_str(), l.varies)).collect();
        assert_eq!(
            varies,
            vec![
                ("---", false),
                ("name: x", false),
                ("---", false),
                ("Ask with `clarify`.", true),
                ("Hermes only.", true),
                ("Shared.", false),
                ("", false),
            ]
        );
    }

    #[test]
    fn malformed_templates_are_refused_not_guessed() {
        for (bad, why) in [
            ("{{nope}}", "unknown token"),
            ("{{ask", "unclosed"),
            ("<!-- clash:if claude -->\nx", "never closed"),
            ("<!-- clash:end -->", "without clash:if"),
            ("<!-- clash:if gpt -->\n<!-- clash:end -->", "unknown harness"),
            ("<!-- clash:if -->\n<!-- clash:end -->", "names no harness"),
            (
                "<!-- clash:if omp -->\n<!-- clash:if hermes -->\n<!-- clash:end -->\n<!-- clash:end -->",
                "do not nest",
            ),
            ("text <!-- clash:end -->", "alone on its line"),
        ] {
            let err = render(bad, AgentKind::Claude).unwrap_err();
            assert!(err.contains(why), "{bad:?}: {err}");
        }
    }

    #[test]
    fn every_token_has_a_value_for_every_harness_and_they_differ() {
        // `varies` assumes an expanded token always differs between harnesses.
        for (name, values) in TOKENS {
            assert!(values.iter().all(|v| !v.is_empty()), "{name}");
            assert!(values[0] != values[1] || values[1] != values[2], "{name}");
        }
    }
}
