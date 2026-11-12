//! The decision queue — pure.
//!
//! The executor's `handoff.md` ends each phase with the questions only the
//! human can answer. The file is overwritten every phase, so clash keeps the
//! questions in `decisions.json` instead: parsed from each new hand-off,
//! answered in the GUI, handed to the next round through the kickoff's
//! `Decisions:` field and the change request's `### Decisions` section.
//! Contract: `docs/workflows.md` → *Decisions*.

use crate::domain::workflow::{DecisionOption, DecisionRecord, DecisionStatus, DecisionsFile};

/// One decision as a hand-off lists it. `id` is `None` for a bullet written
/// without one; [`merge_decisions`] gives it one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedDecision {
    pub id: Option<String>,
    pub text: String,
    pub options: Vec<DecisionOption>,
    pub recommended: String,
}

/// Pure: the decisions of a hand-off — the bullets under the heading that
/// names decisions (`## Unsure / needs your decision`). Lenient: the id may
/// be written `[D3]`, `**D3**`, `D3:` or `(D3)`; options inline
/// (`— options: a) … b) …`) or as nested `a) …` bullets; `recommended: a`
/// inline or nested. "nothing"/"none" bullets are skipped.
pub fn parse_decisions(handoff_md: &str) -> Vec<ParsedDecision> {
    let mut out: Vec<ParsedDecision> = Vec::new();
    let mut in_section = false;
    let mut current: Option<ParsedDecision> = None;
    let flush = |current: &mut Option<ParsedDecision>, out: &mut Vec<ParsedDecision>| {
        if let Some(d) = current.take() {
            if !d.text.is_empty() && !is_nothing(&d.text) {
                out.push(d);
            }
        }
    };
    for line in handoff_md.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            flush(&mut current, &mut out);
            let title = trimmed.trim_start_matches('#').trim().to_ascii_lowercase();
            in_section = title.contains("decision") || title.contains("unsure");
            continue;
        }
        if !in_section {
            continue;
        }
        if trimmed.starts_with("**Next:**") {
            flush(&mut current, &mut out);
            in_section = false;
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        let bullet = strip_bullet(trimmed);
        if indent < 2 {
            if let Some(rest) = bullet {
                flush(&mut current, &mut out);
                current = Some(parse_bullet(rest));
                continue;
            }
        }
        // A nested line belongs to the bullet above it.
        let Some(d) = current.as_mut() else {
            continue;
        };
        let rest = bullet.unwrap_or(trimmed);
        if let Some(r) = strip_label(rest, &["recommended:", "recommendation:"]) {
            d.recommended = recommended_value(r);
        } else if let Some(r) = strip_label(rest, &["options:"]) {
            d.options.extend(parse_options(r));
        } else if let Some(opt) = option_line(rest) {
            d.options.push(opt);
        } else {
            d.text = format!("{} {}", d.text, rest).trim().to_string();
        }
    }
    flush(&mut current, &mut out);
    out
}

fn strip_bullet(s: &str) -> Option<&str> {
    ["- ", "* ", "+ "]
        .iter()
        .find_map(|m| s.strip_prefix(m))
        .map(str::trim)
}

fn is_nothing(text: &str) -> bool {
    let t: String = text
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace() || *c == '/')
        .collect::<String>()
        .trim()
        .to_ascii_lowercase();
    matches!(
        t.as_str(),
        "nothing" | "none" | "n/a" | "na" | "nothing yet"
    )
}

/// Case-insensitive `label` prefix, returning the rest.
fn strip_label<'a>(s: &'a str, labels: &[&str]) -> Option<&'a str> {
    let lower = s.to_ascii_lowercase();
    labels
        .iter()
        .find(|l| lower.starts_with(*l))
        .map(|l| s[l.len()..].trim())
}

/// `D3`-style id at the start of `s` (decorated with `[ ] ( ) ** :` …) and
/// the text after it.
fn split_id(s: &str) -> (Option<String>, &str) {
    let lead = s.trim_start_matches(['[', '(', '*', '`', ' ']);
    let mut chars = lead.char_indices();
    let Some((_, first)) = chars.next() else {
        return (None, s);
    };
    if first != 'D' && first != 'd' {
        return (None, s);
    }
    let digits: String = lead[1..].chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return (None, s);
    }
    let after = &lead[1 + digits.len()..];
    // `Decide…` or `D3x` is a word, not an id.
    if after
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        return (None, s);
    }
    let Ok(n) = digits.parse::<u32>() else {
        return (None, s);
    };
    let rest = after
        .trim_start_matches([']', ')', '*', '`', ':', '.', ' '])
        .trim_start_matches(['—', '–', '-', ':'])
        .trim();
    (Some(format!("D{}", n)), rest)
}

/// Byte offset of the first case-insensitive `needle` in `hay`.
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
    hay.to_ascii_lowercase().find(needle)
}

fn trim_separators(s: &str) -> &str {
    s.trim()
        .trim_end_matches(['—', '–', '-', ',', ';', ':'])
        .trim_start_matches(['—', '–', '-', ',', ';', ':'])
        .trim()
}

fn parse_bullet(raw: &str) -> ParsedDecision {
    let (id, body) = split_id(raw);
    let mut text = body;
    let mut recommended = String::new();
    if let Some(at) = ["recommended:", "recommendation:"]
        .iter()
        .find_map(|l| find_ci(text, l).map(|i| (i, l.len())))
    {
        recommended = recommended_value(&text[at.0 + at.1..]);
        text = &text[..at.0];
    }
    let mut options = Vec::new();
    if let Some(i) = find_ci(text, "options:") {
        options = parse_options(&text[i + "options:".len()..]);
        text = &text[..i];
    } else if let Some(i) = option_marker_at(text, 'a').filter(|&i| {
        // Bare `a) … b) …` only counts as options when a second one follows.
        option_marker_at(&text[i..], 'b').is_some()
    }) {
        options = parse_options(&text[i..]);
        text = &text[..i];
    }
    ParsedDecision {
        id,
        text: trim_separators(text).to_string(),
        options,
        recommended,
    }
}

/// `a` from `a — because…`, `a)`, `**b**`; the whole phrase otherwise.
fn recommended_value(s: &str) -> String {
    let t = s
        .trim()
        .trim_start_matches(['*', '`', '('])
        .trim_end_matches('.');
    let mut chars = t.chars();
    if let Some(c) = chars.next() {
        if c.is_ascii_alphanumeric()
            && chars
                .next()
                .is_none_or(|n| !n.is_alphanumeric() && n != '\'')
        {
            return c.to_ascii_lowercase().to_string();
        }
    }
    trim_separators(t.split(" — ").next().unwrap_or(t)).to_string()
}

/// Offset of the `key)` option marker in `s`, at the start or after a space.
fn option_marker_at(s: &str, key: char) -> Option<usize> {
    let bytes = s.as_bytes();
    let k = key as u8;
    (0..bytes.len().saturating_sub(1)).find(|&i| {
        bytes[i].eq_ignore_ascii_case(&k)
            && bytes[i + 1] == b')'
            && (i == 0 || matches!(bytes[i - 1], b' ' | b'(' | b'\t'))
    })
}

/// `a) foo b) bar` → two options.
fn parse_options(s: &str) -> Vec<DecisionOption> {
    let mut starts: Vec<(usize, char)> = Vec::new();
    let bytes = s.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        let c = bytes[i];
        if c.is_ascii_alphanumeric()
            && bytes[i + 1] == b')'
            && (i == 0 || matches!(bytes[i - 1], b' ' | b'\t'))
        {
            starts.push((i, (c as char).to_ascii_lowercase()));
        }
    }
    let mut out = Vec::new();
    for (k, (at, key)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map_or(s.len(), |(e, _)| *e);
        let text = trim_separators(&s[at + 2..end]);
        if !text.is_empty() {
            out.push(DecisionOption {
                key: key.to_string(),
                text: text.to_string(),
            });
        }
    }
    out
}

/// A nested `a) foo` line.
fn option_line(s: &str) -> Option<DecisionOption> {
    let mut chars = s.chars();
    let key = chars.next().filter(char::is_ascii_alphanumeric)?;
    if chars.next() != Some(')') {
        return None;
    }
    let text = trim_separators(&s[2..]);
    (!text.is_empty()).then(|| DecisionOption {
        key: key.to_ascii_lowercase().to_string(),
        text: text.to_string(),
    })
}

fn normalized(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn id_number(id: &str) -> u32 {
    id.strip_prefix('D')
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// Pure: the id the next new decision gets — one past the highest ever used,
/// so a dropped id is never reused.
pub fn next_decision_id(file: &DecisionsFile) -> String {
    let max = file
        .decisions
        .iter()
        .map(|d| id_number(&d.id))
        .max()
        .unwrap_or(0);
    format!("D{}", max + 1)
}

/// Pure: merge one hand-off's decisions into the file. Returns whether the
/// file changed (it must then be written).
///
/// Only a hand-off whose hash differs from the last merged one is merged. A
/// new id is added open; an open one takes the hand-off's newer wording; an
/// answered or dropped one is never reopened — an answered one listed again
/// after its answer was handed out is queued to be handed out again. A
/// bullet without an id matches a record by its text, else takes the next id.
/// A decision the hand-off no longer lists stays as it is.
pub fn merge_decisions(
    file: &mut DecisionsFile,
    parsed: &[ParsedDecision],
    handoff_hash: &str,
    iteration: u32,
) -> bool {
    if file.handoff_hash == handoff_hash {
        return false;
    }
    file.handoff_hash = handoff_hash.to_string();
    for p in parsed {
        let found = match &p.id {
            Some(id) => file.decisions.iter().position(|d| &d.id == id),
            None => {
                let key = normalized(&p.text);
                file.decisions
                    .iter()
                    .position(|d| normalized(&d.text) == key)
            }
        };
        match found {
            Some(i) => {
                let d = &mut file.decisions[i];
                match d.status {
                    DecisionStatus::Open => {
                        d.text = p.text.clone();
                        if !p.options.is_empty() {
                            d.options = p.options.clone();
                        }
                        if !p.recommended.is_empty() {
                            d.recommended = p.recommended.clone();
                        }
                    }
                    DecisionStatus::Answered if d.delivered_iteration != 0 => {
                        d.delivered_iteration = 0;
                    }
                    _ => {}
                }
            }
            None => {
                let id = p.id.clone().unwrap_or_else(|| next_decision_id(file));
                file.decisions.push(DecisionRecord {
                    id,
                    text: p.text.clone(),
                    options: p.options.clone(),
                    recommended: p.recommended.clone(),
                    first_seen_iteration: iteration,
                    ..Default::default()
                });
            }
        }
    }
    true
}

/// Pure: the hash [`merge_decisions`] compares; empty for no hand-off.
pub fn handoff_hash(handoff_md: &str) -> String {
    if handoff_md.trim().is_empty() {
        String::new()
    } else {
        crate::application::workflow::line_hash(handoff_md)
    }
}

fn find_mut<'a>(file: &'a mut DecisionsFile, id: &str) -> Result<&'a mut DecisionRecord, String> {
    file.decisions
        .iter_mut()
        .find(|d| d.id == id)
        .ok_or_else(|| format!("no decision '{}'", id))
}

/// Pure: record the human's answer (also re-answers, and reopens a dropped
/// one). The answer is handed to the next round.
pub fn answer_decision(
    file: &mut DecisionsFile,
    id: &str,
    answer: &str,
    now: i64,
) -> Result<(), String> {
    let answer = answer.trim();
    if answer.is_empty() {
        return Err("an answer cannot be empty".to_string());
    }
    let d = find_mut(file, id)?;
    d.status = DecisionStatus::Answered;
    d.answer = answer.to_string();
    d.answered_at = now;
    d.delivered_iteration = 0;
    Ok(())
}

/// Pure: drop a decision — nobody needs to answer it any more.
pub fn drop_decision(file: &mut DecisionsFile, id: &str, now: i64) -> Result<(), String> {
    let d = find_mut(file, id)?;
    d.status = DecisionStatus::Dropped;
    d.answered_at = now;
    Ok(())
}

/// Answered decisions a launch at `iteration` hands to its round: never
/// handed out, or handed to this same iteration (a relaunch, or the next
/// phase before any change round).
fn pending_answers(file: &DecisionsFile, iteration: u32) -> impl Iterator<Item = &DecisionRecord> {
    file.decisions.iter().filter(move |d| {
        d.status == DecisionStatus::Answered
            && (d.delivered_iteration == 0 || d.delivered_iteration == iteration)
    })
}

/// One line, for the kickoff.
fn flat(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Pure: the kickoff's ` Decisions:` field — answers to apply, ids still
/// open, and the id a new decision takes. Empty when the item has none.
pub fn decisions_clause(file: &DecisionsFile, iteration: u32) -> String {
    if file.decisions.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = pending_answers(file, iteration)
        .map(|d| {
            format!(
                "{} (answered: \"{}\")",
                d.id,
                flat(&d.answer).replace('"', "'")
            )
        })
        .collect();
    parts.extend(
        file.decisions
            .iter()
            .filter(|d| d.status == DecisionStatus::Open)
            .map(|d| format!("{} (open)", d.id)),
    );
    let listed = if parts.is_empty() {
        "none open".to_string()
    } else {
        parts.join(", ")
    };
    format!(
        " Decisions: {}; a new one takes {}.",
        listed,
        next_decision_id(file)
    )
}

/// Pure: the answers never handed to any round — what a launch or a change
/// request hands out now, to be stamped by [`stamp_delivered`] once it did.
pub fn undelivered_answer_ids(file: &DecisionsFile) -> Vec<String> {
    file.decisions
        .iter()
        .filter(|d| d.status == DecisionStatus::Answered && d.delivered_iteration == 0)
        .map(|d| d.id.clone())
        .collect()
}

/// Pure: stamp `ids` as handed to the round of `iteration`, those still
/// answered and unsent. Returns whether anything changed.
pub fn stamp_delivered(file: &mut DecisionsFile, ids: &[String], iteration: u32) -> bool {
    let mut changed = false;
    for d in &mut file.decisions {
        if d.status == DecisionStatus::Answered && d.delivered_iteration == 0 && ids.contains(&d.id)
        {
            d.delivered_iteration = iteration;
            changed = true;
        }
    }
    changed
}

/// Pure: the `### Decisions` section a change request appends to its note —
/// every answer not yet handed to a round — and the ids it carries, to be
/// stamped once the note is recorded. Empty when there is none.
pub fn change_request_section(file: &DecisionsFile) -> (String, Vec<String>) {
    let ids = undelivered_answer_ids(file);
    let lines: Vec<String> = file
        .decisions
        .iter()
        .filter(|d| ids.contains(&d.id))
        .map(|d| format!("- **{}** {} → {}", d.id, flat(&d.text), flat(&d.answer)))
        .collect();
    if lines.is_empty() {
        return (String::new(), ids);
    }
    (format!("### Decisions\n\n{}\n", lines.join("\n")), ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HANDOFF: &str = "# Hand-off — implement · iteration 3
## Done
- [D9] not a decision, wrong section
## Unsure / needs your decision
- [D3] Keep the legacy flag? — options: a) keep it b) drop it — recommended: a
- **D5** Which queue for retries
  - a) the default queue
  - b) a dedicated one
  - recommended: b
- D7: Rename the table now or later
- Should the cron move to UTC?
## Checked
- cargo test
**Next:** approve — nothing left
";

    #[test]
    fn parses_ids_options_and_recommendations_in_every_spelling() {
        let d = parse_decisions(HANDOFF);
        assert_eq!(d.len(), 4);
        assert_eq!(d[0].id.as_deref(), Some("D3"));
        assert_eq!(d[0].text, "Keep the legacy flag?");
        assert_eq!(
            d[0].options,
            vec![
                DecisionOption {
                    key: "a".into(),
                    text: "keep it".into()
                },
                DecisionOption {
                    key: "b".into(),
                    text: "drop it".into()
                },
            ]
        );
        assert_eq!(d[0].recommended, "a");
        assert_eq!(d[1].id.as_deref(), Some("D5"));
        assert_eq!(d[1].text, "Which queue for retries");
        assert_eq!(d[1].options.len(), 2);
        assert_eq!(d[1].recommended, "b");
        assert_eq!(d[2].id.as_deref(), Some("D7"));
        assert_eq!(d[2].text, "Rename the table now or later");
        assert_eq!(d[3].id, None);
        assert_eq!(d[3].text, "Should the cron move to UTC?");
    }

    #[test]
    fn nothing_and_words_starting_with_d_are_not_decisions() {
        let md = "## Unsure / needs your decision\n- nothing\n- Decide later: D3 is fine\n";
        let d = parse_decisions(md);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].id, None);
        assert_eq!(d[0].text, "Decide later: D3 is fine");
        assert!(parse_decisions("## Unsure\n- (nothing)\n").is_empty());
        assert!(parse_decisions("").is_empty());
    }

    #[test]
    fn bare_options_need_a_second_one_and_prose_recommendations_survive() {
        let d = parse_decisions(
            "## Decisions\n- (D2) Ship behind a flag a) yes b) no — recommended: yes, it is risky\n- D4 — pick a) the only one\n",
        );
        assert_eq!(d[0].id.as_deref(), Some("D2"));
        assert_eq!(d[0].options.len(), 2);
        assert_eq!(d[0].recommended, "yes, it is risky");
        assert_eq!(d[1].text, "pick a) the only one");
        assert!(d[1].options.is_empty());
    }

    fn merged(md: &str, iteration: u32, file: &mut DecisionsFile) -> bool {
        merge_decisions(file, &parse_decisions(md), &handoff_hash(md), iteration)
    }

    #[test]
    fn a_new_handoff_adds_open_decisions_and_numbers_unnamed_ones() {
        let mut f = DecisionsFile::default();
        assert!(merged(HANDOFF, 3, &mut f));
        let ids: Vec<&str> = f.decisions.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, ["D3", "D5", "D7", "D8"]);
        assert!(f.decisions.iter().all(|d| d.status == DecisionStatus::Open));
        assert_eq!(f.decisions[0].first_seen_iteration, 3);
        // The same hand-off again changes nothing, so nothing is written.
        assert!(!merged(HANDOFF, 3, &mut f));
        // The unnamed one keeps its id by its text.
        let again = HANDOFF.replace("cargo test", "cargo test --all");
        assert!(merged(&again, 3, &mut f));
        assert_eq!(f.decisions.len(), 4);
    }

    #[test]
    fn answers_and_drops_survive_the_agent_listing_them_again() {
        let mut f = DecisionsFile::default();
        merged(HANDOFF, 1, &mut f);
        answer_decision(&mut f, "D3", "a) keep it", 10).unwrap();
        drop_decision(&mut f, "D5", 11).unwrap();
        let reworded = HANDOFF.replace("Keep the legacy flag?", "Keep the old flag?");
        merged(&reworded, 1, &mut f);
        assert_eq!(f.decisions[0].status, DecisionStatus::Answered);
        assert_eq!(f.decisions[0].text, "Keep the legacy flag?");
        assert_eq!(f.decisions[1].status, DecisionStatus::Dropped);
        // An open one takes the newer wording.
        let d7 = HANDOFF.replace("Rename the table now or later", "Rename the table now?");
        merged(&d7, 1, &mut f);
        assert_eq!(f.decisions[2].text, "Rename the table now?");
        // A decision no longer listed stays.
        merged("# Hand-off\n## Unsure\n- nothing\n", 2, &mut f);
        assert_eq!(f.decisions.len(), 4);
        assert_eq!(f.decisions[2].status, DecisionStatus::Open);
    }

    #[test]
    fn a_dropped_id_is_never_reused() {
        let mut f = DecisionsFile::default();
        merged("## Decisions\n- [D1] one\n", 1, &mut f);
        drop_decision(&mut f, "D1", 1).unwrap();
        merged("## Decisions\n- something new\n", 1, &mut f);
        assert_eq!(f.decisions[1].id, "D2");
    }

    #[test]
    fn the_kickoff_hands_out_answers_once_and_names_the_open_ids() {
        let mut f = DecisionsFile::default();
        assert_eq!(decisions_clause(&f, 1), "");
        merged(HANDOFF, 1, &mut f);
        answer_decision(&mut f, "D3", "a) keep\n it \"now\"", 5).unwrap();
        assert_eq!(
            decisions_clause(&f, 1),
            " Decisions: D3 (answered: \"a) keep it 'now'\"), D5 (open), D7 (open), D8 (open); a new one takes D9."
        );
        let sent = undelivered_answer_ids(&f);
        assert_eq!(sent, ["D3"]);
        assert!(stamp_delivered(&mut f, &sent, 1));
        assert!(!stamp_delivered(&mut f, &sent, 1));
        // A relaunch at the same iteration still carries it…
        assert!(decisions_clause(&f, 1).contains("D3 (answered"));
        // …a later round does not.
        assert!(!decisions_clause(&f, 2).contains("D3"));
        // Listed again by a new hand-off: handed out again.
        merged("## Decisions\n- [D3] Keep the legacy flag?\n", 2, &mut f);
        assert!(decisions_clause(&f, 2).contains("D3 (answered"));
        for id in ["D3", "D5", "D7", "D8"] {
            drop_decision(&mut f, id, 9).unwrap();
        }
        assert_eq!(
            decisions_clause(&f, 2),
            " Decisions: none open; a new one takes D9."
        );
    }

    #[test]
    fn a_change_request_carries_each_unsent_answer_once() {
        let mut f = DecisionsFile::default();
        merged(HANDOFF, 2, &mut f);
        answer_decision(&mut f, "D5", "b) a dedicated one", 5).unwrap();
        let (section, ids) = change_request_section(&f);
        assert_eq!(
            section,
            "### Decisions\n\n- **D5** Which queue for retries → b) a dedicated one\n"
        );
        // An answer given after the note was composed is not stamped with it.
        answer_decision(&mut f, "D7", "later", 6).unwrap();
        assert!(stamp_delivered(&mut f, &ids, 3));
        assert_eq!(f.decisions[1].delivered_iteration, 3);
        assert_eq!(f.decisions[2].delivered_iteration, 0);
        assert_eq!(change_request_section(&f).1, ["D7"]);
        // The round that reads the section gets it in its kickoff too.
        assert!(decisions_clause(&f, 3).contains("D5 (answered"));
    }

    #[test]
    fn answering_needs_words_and_a_known_id() {
        let mut f = DecisionsFile::default();
        merged(HANDOFF, 1, &mut f);
        assert!(answer_decision(&mut f, "D3", "  ", 1).is_err());
        assert!(answer_decision(&mut f, "D42", "x", 1).is_err());
        drop_decision(&mut f, "D3", 1).unwrap();
        answer_decision(&mut f, "D3", "keep", 2).unwrap();
        assert_eq!(f.decisions[0].status, DecisionStatus::Answered);
    }
}
