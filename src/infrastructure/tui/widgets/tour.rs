use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::centered::centered_rect;

use crate::infrastructure::tui::theme;

/// A single step in the guided tour.
pub struct TourStep {
    pub title: &'static str,
    pub body: &'static [&'static str],
    /// Key hints shown in bold for this step.
    pub keys: &'static [(&'static str, &'static str)],
}

/// All tour steps.
pub const TOUR_STEPS: &[TourStep] = &[
    TourStep {
        title: "Welcome to clash",
        body: &[
            "clash is a terminal UI for managing Claude Code sessions and agent teams.",
            "",
            "This guided tour will walk you through the main features.",
            "You can restart it anytime with  :tour",
        ],
        keys: &[],
    },
    TourStep {
        title: "Sessions View",
        body: &[
            "The Sessions view is your home screen. It shows every Claude Code session across all your projects.",
            "",
            "Sessions are grouped into three sections: Active (working), Done (idle/stashed), and Fail (errored).",
            "Within each section, sessions are sorted alphabetically by name for stable ordering.",
            "Press  A  to cycle through showing only one section at a time.",
            "",
            "Each row shows the session's status, project, summary, active agents, git branch, and worktree (⊟ project/name).",
            "",
            "Status indicators:",
        ],
        keys: &[
            ("PROMPTING", "Claude needs your approval (tool use)"),
            ("WAITING", "Claude is waiting for your text input"),
            ("THINKING", "Claude is reasoning / generating"),
            ("RUNNING", "Claude is actively executing tools"),
            ("STARTING", "Session just spawned"),
            ("IDLE", "Session exited or inactive"),
        ],
    },
    TourStep {
        title: "Navigating Sessions",
        body: &[
            "Use these keys to navigate and manage sessions:",
        ],
        keys: &[
            ("j / k", "Move selection up / down"),
            ("g / G", "Jump to first / last"),
            ("i / Enter", "Inspect session details"),
            ("a", "Attach to a session (live terminal)"),
            ("p", "View git diff for session"),
            ("e", "Open project in IDE"),
            ("o", "Open session in new pane/tab"),
            ("O", "Open ALL running sessions (confirm first)"),
            ("c / n", "Create session (picks from presets if available)"),
            ("s", "Stash / unstash session"),
            ("S", "Stash / unstash ALL sessions"),
            ("w", "Open in git worktree"),
            ("d", "Delete selected session"),
            ("D", "Delete ALL sessions"),
            ("Tab", "Expand / collapse subagents"),
            ("A", "Cycle section filter (Active/Done/Fail)"),
            ("/", "Filter sessions by text"),
        ],
    },
    TourStep {
        title: "Attaching to Sessions",
        body: &[
            "Press  a  to attach inline — the TUI suspends and you interact with Claude directly. Press  o  to open in a new pane or tab instead (clash stays visible).",
            "",
            "While attached:",
        ],
        keys: &[
            ("Ctrl+B", "Detach / close pane"),
            ("Esc", "Detach (inline only)"),
        ],
    },
    TourStep {
        title: "Creating Sessions",
        body: &[
            "Press  c  or  n  to create a new Claude session.",
            "",
            "You'll be prompted for the working directory. The default is where you launched clash. Edit the path or press Enter to accept.",
            "",
            "You can also use the command:",
        ],
        keys: &[
            (":new <path>", "Create session in a specific directory"),
            (":rename <name>", "Rename session (from detail view)"),
        ],
    },
    TourStep {
        title: "Subagents & Visual Indicators",
        body: &[
            "Sessions that spawn subagents show a  count  in the AGENTS column.",
            "",
            "Press  Tab  to expand a session row and see its subagents inline.",
            "",
            "Status summary and indicators:",
        ],
        keys: &[
            ("3 (1! 2\u{25ce})", "3 agents: 1 prompting, 2 thinking"),
            ("\u{25b6} / \u{25bc}", "Collapsed / expanded subagents"),
            ("\u{229e}", "Session open in external pane/tab"),
            ("\u{229f}", "Session in a git worktree"),
        ],
    },
    TourStep {
        title: "Session Detail",
        body: &[
            "Press  Enter  on a session to see its details: full conversation, metadata, and subagent list.",
            "",
            "From Session Detail:",
        ],
        keys: &[
            ("Enter / s", "View subagents table"),
            ("t", "Jump to linked team"),
            ("m", "Jump to team members (agents)"),
            ("a", "Attach to this session"),
            ("e", "Open project in IDE"),
            ("o", "Open in new pane/tab"),
            ("w", "Open in git worktree"),
            ("d", "Delete this session"),
            ("j / k", "Scroll the detail view"),
        ],
    },
    TourStep {
        title: "Diff View",
        body: &[
            "Press  p  on a session (or  :diff  in command mode) to view its git diff.",
            "",
            "The diff view shows a two-panel layout: a file list on the left and the selected file's diff on the right.",
            "Each file shows its addition/deletion counts. The diff auto-refreshes every ~3 seconds while the session is active.",
        ],
        keys: &[
            ("p", "Open diff from Sessions or Session Detail"),
            (":diff", "Open diff via command mode"),
            ("r", "Refresh diff manually"),
            ("j / k", "Scroll diff content"),
            ("n / p", "Next / previous file"),
            ("Esc", "Go back"),
        ],
    },
    TourStep {
        title: "Teams & Agents",
        body: &[
            "clash can manage Claude Code Agent Teams — groups of Claude instances collaborating on tasks.",
        ],
        keys: &[
            (":sessions", "Navigate to Sessions view"),
            (":teams", "Navigate to Teams view"),
            (":agents", "Navigate to Agents view"),
            (":tasks", "Navigate to Tasks view"),
            (":create team X", "Create a new team"),
            (":delete team X", "Delete a team"),
        ],
    },
    TourStep {
        title: "Commands & Filtering",
        body: &[
            "Press  :  to enter command mode. Type a command and press Enter.",
            "",
            "Press  /  to enter filter mode. Type to filter the current table by text. Press  Esc  to clear.",
            "",
            "Press  ?  anytime to see context-sensitive keybindings.",
        ],
        keys: &[
            (":", "Command mode"),
            ("/", "Filter mode"),
            ("?", "Help overlay (scrollable)"),
            (":active / :all", "Filter active or all sessions"),
            (":update", "Update clash to latest version"),
            ("Esc", "Go back / cancel"),
            ("q", "Quit clash (stashes sessions)"),
        ],
    },
    TourStep {
        title: "You're ready!",
        body: &[
            "That covers the essentials. Here's a quick cheat sheet:",
            "",
            "  Sessions are your home screen",
            "  a  to attach inline,  e  to open in IDE",
            "  o  to open in pane/tab",
            "  O  to open all running sessions at once",
            "  Tab expands subagents inline",
            "  \u{229e} = open externally,  \u{229f} = git worktree",
            "  Status updates in real-time via hooks",
            "  :update keeps clash up to date",
            "",
            "Run  :tour  anytime to revisit this guide.",
            "Press  ?  for context help on any screen.",
        ],
        keys: &[],
    },
];

/// Render the tour overlay.
pub fn render_tour(step_index: usize, frame: &mut Frame, area: Rect) {
    let step = match TOUR_STEPS.get(step_index) {
        Some(s) => s,
        None => return,
    };

    let popup_area = centered_rect(65, 70, area);
    frame.render_widget(Clear, popup_area);

    let mut lines: Vec<Line> = Vec::new();

    // Body text
    for &text in step.body {
        if text.is_empty() {
            lines.push(Line::from(""));
        } else {
            lines.push(Line::from(Span::styled(
                text,
                Style::default().fg(theme::TEXT),
            )));
        }
    }

    // Key hints
    if !step.keys.is_empty() {
        lines.push(Line::from(""));
        for &(key, desc) in step.keys {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:<16}", key),
                    Style::default()
                        .fg(theme::CLAUDE_COLOR)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(desc, Style::default().fg(theme::TEXT_DIM)),
            ]));
        }
    }

    // Footer with navigation
    lines.push(Line::from(""));
    lines.push(Line::from(""));

    let step_num = step_index + 1;
    let total = TOUR_STEPS.len();
    let is_last = step_index == total - 1;

    let nav_text = if is_last {
        format!(
            "  Step {}/{}  │  Enter: finish   Esc: close",
            step_num, total
        )
    } else {
        format!(
            "  Step {}/{}  │  Enter: next   Esc: skip tour",
            step_num, total
        )
    };
    lines.push(Line::from(Span::styled(
        nav_text,
        Style::default().fg(theme::MUTED),
    )));

    let block = Block::default()
        .title(format!(" {} ", step.title))
        .title_style(
            Style::default()
                .fg(theme::DIALOG_TITLE)
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::DIALOG_BORDER))
        .style(Style::default().bg(theme::BG));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, popup_area);
}
