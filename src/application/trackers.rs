//! Ticket trackers — which system a ticket key lives in, and its URL.
//!
//! Jira and Linear keys share one shape (`PS-123`, `ENG-42`), so the system
//! is never read from the key itself: each configured tracker claims key
//! prefixes, and a key no tracker claims belongs to the primary one. That is
//! what lets one item mention both while a team migrates from one to the
//! other. Pure: the settings come in as strings, nothing here reads config.
//! Full rules: `docs/workflows.md` → *Ticket trackers*.

/// The ticket systems clash knows how to link and post to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackerKind {
    Jira,
    Linear,
}

impl TrackerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TrackerKind::Jira => "jira",
            TrackerKind::Linear => "linear",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            TrackerKind::Jira => "Jira",
            TrackerKind::Linear => "Linear",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "jira" => Some(TrackerKind::Jira),
            "linear" => Some(TrackerKind::Linear),
            _ => None,
        }
    }
}

/// One configured tracker: where its tickets live and which keys it owns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracker {
    pub kind: TrackerKind,
    /// Browse URL with `{key}` standing for the ticket key.
    pub template: String,
    /// Upper-cased key prefixes (`PS`, `ENG`) this tracker claims.
    pub prefixes: Vec<String>,
}

impl Tracker {
    pub fn link(&self, key: &str) -> String {
        self.template.replace("{key}", key)
    }
}

/// The trackers settings, resolved. `primary` is `None` when linking is off.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackerSet {
    pub trackers: Vec<Tracker>,
    /// The tracker a key no tracker claims belongs to.
    pub primary: Option<TrackerKind>,
    /// `trackers.primary = "none"`: mention keys, never link or ask.
    pub off: bool,
}

/// The raw settings a [`TrackerSet`] is built from (`[trackers]` in config).
#[derive(Debug, Clone, Copy, Default)]
pub struct TrackerSettings<'a> {
    pub primary: &'a str,
    pub jira_site: &'a str,
    pub jira_prefixes: &'a str,
    pub linear_workspace: &'a str,
    pub linear_prefixes: &'a str,
}

/// Pure: a Jira site (`acme.atlassian.net`, with or without a scheme or a
/// trailing path) as its browse URL template.
pub fn jira_template(site: &str) -> Option<String> {
    let s = site.trim().trim_end_matches('/');
    if s.is_empty() {
        return None;
    }
    let with_scheme = if s.contains("://") {
        s.to_string()
    } else {
        format!("https://{}", s)
    };
    // Only the origin: a pasted ticket or board URL must not become the base.
    let origin_end = with_scheme
        .find("://")
        .map(|i| i + 3)
        .and_then(|start| with_scheme[start..].find('/').map(|j| start + j))
        .unwrap_or(with_scheme.len());
    Some(format!("{}/browse/{{key}}", &with_scheme[..origin_end]))
}

/// Pure: a Linear workspace — its slug (`acme`) or any URL inside it
/// (`https://linear.app/acme/issue/ENG-1`) — as its issue URL template.
pub fn linear_template(workspace: &str) -> Option<String> {
    let s = workspace.trim().trim_end_matches('/');
    if s.is_empty() {
        return None;
    }
    let after_host = match s.find("linear.app/") {
        Some(i) => &s[i + "linear.app/".len()..],
        None if !s.contains('/') && !s.contains('.') => s,
        None => return None,
    };
    let slug = after_host.split('/').next().unwrap_or("").trim();
    let valid = !slug.is_empty()
        && slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    valid.then(|| format!("https://linear.app/{}/issue/{{key}}", slug))
}

/// Pure: a prefixes setting (`PS, ops ENG`) as upper-cased, de-duplicated
/// prefixes. Anything that cannot start a ticket key is dropped.
pub fn parse_prefixes(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in raw.split(|c: char| c == ',' || c.is_whitespace()) {
        let p = p.trim().trim_end_matches('-').to_ascii_uppercase();
        let valid = p.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && p.chars().all(|c| c.is_ascii_alphanumeric());
        if valid && !out.contains(&p) {
            out.push(p);
        }
    }
    out
}

/// Pure: does this look like a ticket key (`PROJ-123`)? The project part is
/// letters/digits starting with a letter, the number part all digits — the
/// shape Jira and Linear share.
pub fn valid_ticket_key(s: &str) -> bool {
    let s = s.trim();
    let Some((proj, num)) = s.split_once('-') else {
        return false;
    };
    proj.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && proj.chars().all(|c| c.is_ascii_alphanumeric())
        && !num.is_empty()
        && num.chars().all(|c| c.is_ascii_digit())
}

impl TrackerSet {
    /// Build from the settings. A tracker exists once its site/workspace is
    /// set; the primary falls back to the only configured tracker, so a
    /// Linear-only setup works without also choosing Linear as primary.
    pub fn from_settings(s: TrackerSettings) -> Self {
        let mut trackers = Vec::new();
        if let Some(template) = jira_template(s.jira_site) {
            trackers.push(Tracker {
                kind: TrackerKind::Jira,
                template,
                prefixes: parse_prefixes(s.jira_prefixes),
            });
        }
        if let Some(template) = linear_template(s.linear_workspace) {
            trackers.push(Tracker {
                kind: TrackerKind::Linear,
                template,
                prefixes: parse_prefixes(s.linear_prefixes),
            });
        }
        let off = s.primary.trim().eq_ignore_ascii_case("none");
        let wanted = TrackerKind::parse(s.primary);
        let primary = if off {
            None
        } else {
            wanted
                .filter(|k| trackers.iter().any(|t| t.kind == *k))
                .or_else(|| trackers.first().map(|t| t.kind))
        };
        TrackerSet {
            trackers,
            primary,
            off,
        }
    }

    pub fn get(&self, kind: TrackerKind) -> Option<&Tracker> {
        self.trackers.iter().find(|t| t.kind == kind)
    }

    /// The tracker `key` belongs to: the one claiming its prefix, else the
    /// primary. `None` when nothing is configured or linking is off.
    pub fn for_key(&self, key: &str) -> Option<&Tracker> {
        if self.off {
            return None;
        }
        let prefix = key
            .trim()
            .split('-')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        self.trackers
            .iter()
            .find(|t| t.prefixes.contains(&prefix))
            .or_else(|| self.primary.and_then(|k| self.get(k)))
    }

    /// The browse URL of `key`, when a tracker owns it.
    pub fn link(&self, key: &str) -> Option<String> {
        self.for_key(key).map(|t| t.link(key))
    }

    /// What a workflow kickoff says about linking tickets.
    pub fn ticket_links(&self) -> TicketLinks {
        if self.off {
            return TicketLinks::Off;
        }
        let Some(primary) = self.primary.and_then(|k| self.get(k)) else {
            return TicketLinks::Unknown;
        };
        let routes = self
            .trackers
            .iter()
            .filter(|t| !t.prefixes.is_empty())
            .map(|t| (t.prefixes.clone(), t.template.clone()))
            .collect();
        TicketLinks::Map {
            routes,
            other: primary.template.clone(),
        }
    }
}

/// How an agent links the tickets it mentions — the kickoff's `Ticket links:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TicketLinks {
    /// Keys with a listed prefix use that template; every other key `other`.
    /// `{key}` in a template stands for the key.
    Map {
        routes: Vec<(Vec<String>, String)>,
        other: String,
    },
    /// Linking is off: mention keys plainly, never ask for a host.
    Off,
    /// Nothing configured: the agent asks the human for the host when it
    /// first needs one (or, autonomous, leaves the key unlinked and says so).
    Unknown,
}

impl TicketLinks {
    /// The kickoff field; nothing when unknown, so the skill's ask applies.
    /// One template alone is written bare, as the skills read it simplest.
    pub fn clause(&self) -> String {
        match self {
            TicketLinks::Map { routes, other } => {
                let listed: Vec<String> = routes
                    .iter()
                    .filter(|(_, t)| t != other)
                    .map(|(p, t)| format!("{} → {}", p.join(", "), t))
                    .collect();
                if listed.is_empty() {
                    format!(" Ticket links: {}.", other)
                } else {
                    format!(
                        " Ticket links: {}; any other key → {}.",
                        listed.join("; "),
                        other
                    )
                }
            }
            TicketLinks::Off => " Ticket links: none.".to_string(),
            TicketLinks::Unknown => String::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(primary: &str, jira: &str, jp: &str, linear: &str, lp: &str) -> TrackerSet {
        TrackerSet::from_settings(TrackerSettings {
            primary,
            jira_site: jira,
            jira_prefixes: jp,
            linear_workspace: linear,
            linear_prefixes: lp,
        })
    }

    #[test]
    fn sites_and_workspaces_become_templates() {
        assert_eq!(
            jira_template("acme.atlassian.net/").as_deref(),
            Some("https://acme.atlassian.net/browse/{key}")
        );
        // A pasted ticket URL keeps only its origin.
        assert_eq!(
            jira_template("https://acme.atlassian.net/browse/PS-1").as_deref(),
            Some("https://acme.atlassian.net/browse/{key}")
        );
        assert_eq!(
            linear_template("acme").as_deref(),
            Some("https://linear.app/acme/issue/{key}")
        );
        assert_eq!(
            linear_template("https://linear.app/acme/issue/ENG-4/title").as_deref(),
            Some("https://linear.app/acme/issue/{key}")
        );
        assert_eq!(linear_template("acme.example.com"), None);
        assert_eq!(jira_template("  "), None);
    }

    #[test]
    fn prefixes_are_normalized() {
        assert_eq!(
            parse_prefixes(" ps, OPS eng- ps 9x"),
            vec!["PS", "OPS", "ENG"]
        );
        assert!(parse_prefixes("").is_empty());
    }

    #[test]
    fn a_key_goes_to_the_tracker_claiming_its_prefix_else_the_primary() {
        let both = set("jira", "acme.atlassian.net", "PS, OPS", "acme", "ENG");
        assert_eq!(both.for_key("eng-42").unwrap().kind, TrackerKind::Linear);
        assert_eq!(both.for_key("PS-1").unwrap().kind, TrackerKind::Jira);
        // Unclaimed prefix: the primary.
        assert_eq!(both.for_key("WEB-9").unwrap().kind, TrackerKind::Jira);
        assert_eq!(
            both.link("ENG-42").as_deref(),
            Some("https://linear.app/acme/issue/ENG-42")
        );
    }

    #[test]
    fn the_primary_falls_back_to_the_only_configured_tracker() {
        let linear_only = set("jira", "", "", "acme", "");
        assert_eq!(linear_only.primary, Some(TrackerKind::Linear));
        assert_eq!(
            linear_only.for_key("PS-1").unwrap().kind,
            TrackerKind::Linear
        );
        assert_eq!(set("jira", "", "", "", "").primary, None);
    }

    #[test]
    fn ticket_links_clause_states_the_routes() {
        let both = set("linear", "acme.atlassian.net", "PS, OPS", "acme", "");
        assert_eq!(
            both.ticket_links().clause(),
            " Ticket links: PS, OPS → https://acme.atlassian.net/browse/{key}; \
             any other key → https://linear.app/acme/issue/{key}."
        );
        // One tracker: the template alone.
        assert_eq!(
            set("jira", "acme.atlassian.net", "PS", "", "")
                .ticket_links()
                .clause(),
            " Ticket links: https://acme.atlassian.net/browse/{key}."
        );
        assert_eq!(
            set("none", "acme.atlassian.net", "", "", "").ticket_links(),
            TicketLinks::Off
        );
        assert_eq!(
            set("none", "x", "", "", "").ticket_links().clause(),
            " Ticket links: none."
        );
        assert_eq!(set("jira", "", "", "", "").ticket_links().clause(), "");
    }

    #[test]
    fn linking_off_links_nothing() {
        let off = set("none", "acme.atlassian.net", "PS", "acme", "ENG");
        assert!(off.for_key("PS-1").is_none());
        assert!(off.link("ENG-1").is_none());
    }

    #[test]
    fn ticket_keys_have_one_shape() {
        assert!(valid_ticket_key("PS-123"));
        assert!(valid_ticket_key("eng2-9"));
        assert!(!valid_ticket_key("PS-"));
        assert!(!valid_ticket_key("1PS-2"));
        assert!(!valid_ticket_key("PS 12"));
    }
}
