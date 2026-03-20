//! Navigation stack with breadcrumb support.

use crate::adapters::views::ViewKind;

/// An entry in the navigation stack.
#[derive(Debug, Clone)]
pub struct NavEntry {
    pub view: ViewKind,
    /// Optional context (e.g., team name, task id).
    pub context: Option<String>,
}

/// Navigation stack with breadcrumb support.
#[derive(Debug)]
pub struct NavigationStack {
    stack: Vec<NavEntry>,
}

impl Default for NavigationStack {
    fn default() -> Self {
        Self::new()
    }
}

impl NavigationStack {
    pub fn new() -> Self {
        Self {
            stack: vec![NavEntry {
                view: ViewKind::Sessions,
                context: None,
            }],
        }
    }

    /// Get the current view.
    pub fn current(&self) -> &NavEntry {
        self.stack
            .last()
            .expect("Navigation stack should never be empty")
    }

    /// Push a new view onto the stack.
    pub fn push(&mut self, view: ViewKind, context: Option<String>) {
        self.stack.push(NavEntry { view, context });
    }

    /// Pop the current view. Returns false if at root.
    pub fn pop(&mut self) -> bool {
        if self.stack.len() > 1 {
            self.stack.pop();
            true
        } else {
            false
        }
    }

    /// Replace the entire stack with a single view.
    #[allow(dead_code)]
    pub fn replace(&mut self, view: ViewKind) {
        self.stack.clear();
        self.stack.push(NavEntry {
            view,
            context: None,
        });
    }

    /// Get breadcrumb trail as strings.
    pub fn breadcrumbs(&self) -> Vec<String> {
        self.stack
            .iter()
            .map(|entry| {
                if let Some(ctx) = &entry.context {
                    format!("{} > {}", entry.view.label(), ctx)
                } else {
                    entry.view.label().to_string()
                }
            })
            .collect()
    }

    /// Get the context for a parent view.
    pub fn context_for(&self, view: ViewKind) -> Option<&str> {
        self.stack
            .iter()
            .find(|e| e.view == view)
            .and_then(|e| e.context.as_deref())
    }

    /// Get the currently selected team name from the nav context.
    pub fn current_team(&self) -> Option<&str> {
        for entry in self.stack.iter().rev() {
            if matches!(
                entry.view,
                ViewKind::Teams | ViewKind::TeamDetail | ViewKind::Agents | ViewKind::Tasks
            ) {
                return entry.context.as_deref();
            }
        }
        self.context_for(ViewKind::TeamDetail)
            .or_else(|| self.context_for(ViewKind::Teams))
            .or_else(|| self.context_for(ViewKind::Agents))
            .or_else(|| self.context_for(ViewKind::Tasks))
    }

    /// Get a reference to the internal stack entries.
    pub fn entries(&self) -> &[NavEntry] {
        &self.stack
    }

    /// Restore the navigation stack from a list of (view, context) pairs.
    pub fn restore_from(&mut self, entries: Vec<(ViewKind, Option<String>)>) {
        if entries.is_empty() {
            return;
        }
        self.stack = entries
            .into_iter()
            .map(|(view, context)| NavEntry { view, context })
            .collect();
    }

    /// Get the current session ID from nav context.
    pub fn current_session(&self) -> Option<&str> {
        for entry in self.stack.iter().rev() {
            if entry.view == ViewKind::SessionDetail || entry.view == ViewKind::Subagents {
                if let Some(ref ctx) = entry.context {
                    return Some(ctx.as_str());
                }
            }
        }
        self.context_for(ViewKind::SessionDetail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let nav = NavigationStack::new();
        assert_eq!(nav.current().view, ViewKind::Sessions);
        assert_eq!(nav.breadcrumbs().len(), 1);
    }

    #[test]
    fn test_push_pop() {
        let mut nav = NavigationStack::new();
        nav.push(ViewKind::Tasks, Some("my-team".to_string()));
        assert_eq!(nav.current().view, ViewKind::Tasks);
        assert_eq!(nav.breadcrumbs().len(), 2);

        assert!(nav.pop());
        assert_eq!(nav.current().view, ViewKind::Sessions);
    }

    #[test]
    fn test_pop_at_root() {
        let mut nav = NavigationStack::new();
        assert!(!nav.pop());
    }

    #[test]
    fn test_replace() {
        let mut nav = NavigationStack::new();
        nav.push(ViewKind::Tasks, None);
        nav.replace(ViewKind::Agents);
        assert_eq!(nav.breadcrumbs().len(), 1);
        assert_eq!(nav.current().view, ViewKind::Agents);
    }

    #[test]
    fn test_breadcrumbs() {
        let mut nav = NavigationStack::new();
        nav.push(ViewKind::TeamDetail, Some("my-team".to_string()));
        nav.push(ViewKind::Tasks, None);
        let crumbs = nav.breadcrumbs();
        assert_eq!(crumbs.len(), 3);
        assert_eq!(crumbs[0], "Sessions");
        assert_eq!(crumbs[1], "Team > my-team");
    }
}
