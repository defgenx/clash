//! Sized review rounds — pick a diff-judging round's models from the size of
//! the change it reads, pure. A big change gets stronger finders, a tiny one
//! does not run on the most expensive lead. Full rules: `docs/workflows.md` →
//! *Sized review rounds*.

use crate::domain::entities::AgentKind;
use crate::domain::workflow::ReviewSize;

/// At most this many changed lines (additions + deletions) *and*
/// [`SMALL_MAX_FILES`] files is a small change.
pub const SMALL_MAX_LINES: usize = 150;
pub const SMALL_MAX_FILES: usize = 5;
/// At least this many changed lines *or* [`LARGE_MIN_FILES`] files is a large one.
pub const LARGE_MIN_LINES: usize = 800;
pub const LARGE_MIN_FILES: usize = 25;

/// Pure: the size of a change of `changed_lines` (additions + deletions) over
/// `files`, spread across `prs` pull requests. More than one PR in scope (a
/// multi-repo change or a stack) is never small: the contract between the
/// pieces is part of what the round reads.
pub fn review_size(changed_lines: usize, files: usize, prs: usize) -> ReviewSize {
    if changed_lines >= LARGE_MIN_LINES || files >= LARGE_MIN_FILES {
        ReviewSize::Large
    } else if changed_lines <= SMALL_MAX_LINES && files <= SMALL_MAX_FILES && prs <= 1 {
        ReviewSize::Small
    } else {
        ReviewSize::Medium
    }
}

/// A session's models where they differ from the regular workflow pins
/// (`workflows.lead_model` / `workflows.subagent_model`). `None` = the pin.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoundModels {
    pub lead: Option<String>,
    pub subagent: Option<String>,
}

/// The `workflows.review_sizing` / tier settings a round's models come from.
#[derive(Debug, Clone, Copy, Default)]
pub struct SizingSettings<'a> {
    /// `auto` | `off`; anything but `off` is `auto`.
    pub sizing: &'a str,
    /// Lead of a small round; empty = the regular lead pin.
    pub small_lead: &'a str,
    /// Subagents of a large round; empty = the regular subagent pin.
    pub large_subagent: &'a str,
}

impl SizingSettings<'_> {
    pub fn enabled(&self) -> bool {
        !self.sizing.trim().eq_ignore_ascii_case("off")
    }
}

/// Pure: the models a round of `size` runs on, as overrides of the regular
/// pins. Small swaps the lead, large swaps the subagents, medium (and an
/// unmeasured round) keeps both. Claude Code only — OMP's model ids are the
/// user's providers', so a Claude id would mean nothing there.
pub fn models_for_round(
    agent: AgentKind,
    size: Option<ReviewSize>,
    settings: &SizingSettings,
) -> RoundModels {
    let pick = |m: &str| Some(m.trim().to_string()).filter(|m| !m.is_empty());
    if agent != AgentKind::Claude || !settings.enabled() {
        return RoundModels::default();
    }
    match size {
        Some(ReviewSize::Small) => RoundModels {
            lead: pick(settings.small_lead),
            subagent: None,
        },
        Some(ReviewSize::Large) => RoundModels {
            lead: None,
            subagent: pick(settings.large_subagent),
        },
        _ => RoundModels::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_follow_the_thresholds_at_their_boundaries() {
        use ReviewSize::*;
        assert_eq!(review_size(0, 0, 1), Small);
        assert_eq!(review_size(150, 5, 1), Small);
        assert_eq!(review_size(151, 5, 1), Medium);
        assert_eq!(review_size(150, 6, 1), Medium);
        assert_eq!(review_size(799, 24, 1), Medium);
        assert_eq!(review_size(800, 1, 1), Large);
        assert_eq!(review_size(10, 25, 1), Large);
    }

    #[test]
    fn several_prs_are_never_small() {
        assert_eq!(review_size(10, 2, 2), ReviewSize::Medium);
        assert_eq!(review_size(900, 2, 3), ReviewSize::Large);
    }

    fn settings(sizing: &str) -> SizingSettings<'_> {
        SizingSettings {
            sizing,
            small_lead: "claude-sonnet-5-5",
            large_subagent: "claude-opus-5-5",
        }
    }

    #[test]
    fn small_swaps_the_lead_and_large_swaps_the_subagents() {
        let s = settings("auto");
        let small = models_for_round(AgentKind::Claude, Some(ReviewSize::Small), &s);
        assert_eq!(small.lead.as_deref(), Some("claude-sonnet-5-5"));
        assert_eq!(small.subagent, None);
        let large = models_for_round(AgentKind::Claude, Some(ReviewSize::Large), &s);
        assert_eq!(large.lead, None);
        assert_eq!(large.subagent.as_deref(), Some("claude-opus-5-5"));
        for medium in [Some(ReviewSize::Medium), Some(ReviewSize::Unknown), None] {
            assert_eq!(
                models_for_round(AgentKind::Claude, medium, &s),
                RoundModels::default()
            );
        }
    }

    #[test]
    fn off_omp_and_empty_tiers_keep_the_regular_pins() {
        for size in [ReviewSize::Small, ReviewSize::Large] {
            assert_eq!(
                models_for_round(AgentKind::Claude, Some(size), &settings("off")),
                RoundModels::default()
            );
            assert_eq!(
                models_for_round(AgentKind::Omp, Some(size), &settings("auto")),
                RoundModels::default()
            );
            let empty = SizingSettings {
                sizing: "auto",
                small_lead: " ",
                large_subagent: "",
            };
            assert_eq!(
                models_for_round(AgentKind::Claude, Some(size), &empty),
                RoundModels::default()
            );
        }
    }
}
