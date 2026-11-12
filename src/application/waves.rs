//! Waves — a multi-wave workflow item's schedule, pure.
//!
//! An item that is a whole project rather than one task is planned as
//! **waves**: slices of the plan that are implemented and approved one after
//! another, in dependency order, inside the same item (one worktree; each
//! wave on its own branch, `<item branch>-<id>`, with its own PRs). The planner declares them in `waves.json`; clash records which are
//! approved and which the current round covers in `meta.waves`. Two waves may
//! run in the same round when neither depends on the other and the areas they
//! touch are disjoint — the executor then works them in parallel. Full rules:
//! `docs/workflows.md` → *Waves*.

use std::collections::{BTreeMap, BTreeSet};

use crate::domain::workflow::{
    Wave, WaveBoard, WavePlan, WaveProgress, WaveState, WaveView, WorkflowStatus,
};

/// Parse and validate `waves.json`. An unusable file is an error, never a
/// guess: clash starts agents off this schedule.
pub fn parse_wave_plan(raw: &str) -> Result<WavePlan, String> {
    let plan: WavePlan =
        serde_json::from_str(raw).map_err(|e| format!("waves.json is not valid: {e}"))?;
    validate(&plan)?;
    Ok(plan)
}

/// Ids present and unique, dependencies known and acyclic.
pub fn validate(plan: &WavePlan) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for w in &plan.waves {
        let id = w.id.trim();
        if id.is_empty() {
            return Err("a wave in waves.json has no id".into());
        }
        if !seen.insert(id) {
            return Err(format!("wave id {id} is used twice"));
        }
    }
    for w in &plan.waves {
        for d in &w.depends_on {
            if d.trim() == w.id.trim() {
                return Err(format!("wave {} depends on itself", w.id));
            }
            if !seen.contains(d.trim()) {
                return Err(format!("wave {} depends on unknown wave {d}", w.id));
            }
        }
    }
    // Every wave gets a level only when the graph is acyclic.
    if levels(plan).len() != plan.waves.len() {
        return Err("the waves' dependencies form a cycle".into());
    }
    Ok(())
}

fn find<'a>(plan: &'a WavePlan, id: &str) -> Option<&'a Wave> {
    plan.waves.iter().find(|w| w.id.trim() == id.trim())
}

/// Dependency depth of every wave (0 = no dependencies). A wave caught in a
/// cycle gets none, which is how `validate` detects one.
fn levels(plan: &WavePlan) -> BTreeMap<String, u32> {
    let mut out: BTreeMap<String, u32> = BTreeMap::new();
    // At most one pass per wave settles an acyclic graph.
    for _ in 0..=plan.waves.len() {
        let mut changed = false;
        for w in &plan.waves {
            if out.contains_key(w.id.trim()) {
                continue;
            }
            let deps: Option<Vec<u32>> = w
                .depends_on
                .iter()
                .map(|d| out.get(d.trim()).copied())
                .collect();
            if let Some(deps) = deps {
                out.insert(
                    w.id.trim().to_string(),
                    deps.iter().map(|l| l + 1).max().unwrap_or(0),
                );
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    out
}

/// Every wave `id` depends on, directly or not.
fn ancestors(plan: &WavePlan, id: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut todo: Vec<String> = find(plan, id)
        .map(|w| w.depends_on.clone())
        .unwrap_or_default();
    while let Some(d) = todo.pop() {
        let d = d.trim().to_string();
        if out.insert(d.clone()) {
            if let Some(w) = find(plan, &d) {
                todo.extend(w.depends_on.iter().cloned());
            }
        }
    }
    out
}

/// An area as a path prefix: `./src/auth/**` → `src/auth`. Anything after a
/// glob character is dropped, which can only widen the area — the safe side.
/// Empty means the whole repository.
fn area_prefix(raw: &str) -> String {
    let s = raw.trim().trim_start_matches("./");
    let s = match s.find(['*', '?', '[', '{']) {
        Some(i) => &s[..i],
        None => s,
    };
    let s = s.trim_end_matches('/');
    if s == "." {
        String::new()
    } else {
        s.to_string()
    }
}

/// Do two areas lists touch a common path?
pub fn areas_overlap(a: &[String], b: &[String]) -> bool {
    a.iter().any(|x| {
        let x = area_prefix(x);
        b.iter().any(|y| {
            let y = area_prefix(y);
            x.is_empty()
                || y.is_empty()
                || x == y
                || x.starts_with(&format!("{y}/"))
                || y.starts_with(&format!("{x}/"))
        })
    })
}

/// May waves `a` and `b` run in the same round? Only when neither depends on
/// the other (through any chain) and both declared areas that do not
/// overlap. A wave with no areas runs alone: nothing says what it touches.
pub fn compatible(plan: &WavePlan, a: &str, b: &str) -> bool {
    let (Some(wa), Some(wb)) = (find(plan, a), find(plan, b)) else {
        return false;
    };
    if wa.id.trim() == wb.id.trim() || wa.areas.is_empty() || wb.areas.is_empty() {
        return false;
    }
    if ancestors(plan, a).contains(wb.id.trim()) || ancestors(plan, b).contains(wa.id.trim()) {
        return false;
    }
    !areas_overlap(&wa.areas, &wb.areas)
}

/// Where one wave stands.
fn state_of(
    wave: &Wave,
    progress: &WaveProgress,
    status: WorkflowStatus,
) -> (WaveState, Vec<String>) {
    let id = wave.id.trim();
    if progress.done.iter().any(|d| d == id) {
        return (WaveState::Done, Vec::new());
    }
    let blocked_by: Vec<String> = wave
        .depends_on
        .iter()
        .map(|d| d.trim().to_string())
        .filter(|d| !progress.done.contains(d))
        .collect();
    if progress.scope.iter().any(|s| s == id) {
        use WorkflowStatus::*;
        match status {
            Implementing | ChangesRequested => return (WaveState::Active, blocked_by),
            DiffReview | Reviewing | PrDraft | PrReady => return (WaveState::Review, blocked_by),
            // Rewound to the plan, or finished: the round no longer stands.
            _ => {}
        }
    }
    if blocked_by.is_empty() {
        (WaveState::Ready, blocked_by)
    } else {
        (WaveState::Blocked, blocked_by)
    }
}

/// One of the item's PRs, as the wave matching sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrRef {
    pub url: String,
    /// The PR's head branch, where ticket keys usually live.
    pub head_ref: String,
}

/// Does `text` name `key` as a whole ticket key? `TT-1536` is not in
/// `feat/TT-15360-cache`: the next character must not extend the number.
fn names_ticket(text: &str, key: &str) -> bool {
    let (hay, key) = (text.to_ascii_uppercase(), key.trim().to_ascii_uppercase());
    if key.is_empty() {
        return false;
    }
    hay.match_indices(&key).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + key.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

/// The branch a wave is built on: `<item branch>-<id>` (the id alone when the
/// item has no branch yet).
pub fn wave_branch(item_branch: &str, id: &str) -> String {
    match item_branch.trim() {
        "" => id.trim().to_string(),
        b => format!("{b}-{}", id.trim()),
    }
}

/// A PR URL reduced to `host/owner/repo/pull/<n>`, so a trailing slash, a
/// `/files` tab or a case difference still names the same PR.
fn pr_key(url: &str) -> String {
    let u = url.trim().to_ascii_lowercase();
    let u = u
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.");
    match u.find("/pull/") {
        Some(i) => {
            let n: String = u[i + 6..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            format!("{}/pull/{n}", &u[..i])
        }
        None => u.trim_end_matches('/').to_string(),
    }
}

/// Which waves each PR belongs to: every wave listing the PR's URL claims it;
/// otherwise the wave whose branch (`wave_branch`) is the PR's head; otherwise
/// a ticket key in the head branch claims it for the one wave that lists that
/// ticket. A ticket several waves share decides nothing, and a PR nothing
/// claims is left out — guessing would file work under the wrong wave.
pub fn assign_prs(
    plan: &WavePlan,
    prs: &[PrRef],
    item_branch: &str,
) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    for pr in prs {
        let key = pr_key(&pr.url);
        let mut owners: Vec<String> = plan
            .waves
            .iter()
            .filter(|w| w.prs.iter().any(|u| pr_key(u) == key))
            .map(|w| w.id.trim().to_string())
            .collect();
        if owners.is_empty() {
            let head = pr.head_ref.trim();
            if let Some(w) = plan.waves.iter().find(|w| {
                !head.is_empty() && head.eq_ignore_ascii_case(&wave_branch(item_branch, &w.id))
            }) {
                owners.push(w.id.trim().to_string());
            }
        }
        if owners.is_empty() {
            let hits: Vec<&Wave> = plan
                .waves
                .iter()
                .filter(|w| w.tickets.iter().any(|t| names_ticket(&pr.head_ref, t)))
                .collect();
            if hits.len() == 1 {
                owners.push(hits[0].id.trim().to_string());
            }
        }
        if !owners.is_empty() {
            out.insert(pr.url.clone(), owners);
        }
    }
    out
}

/// The board the GUI renders: every wave with its state, level, its PRs and
/// the waves it may run alongside, or the reason `waves.json` cannot be used.
pub fn wave_board(
    plan: Result<WavePlan, String>,
    progress: &WaveProgress,
    status: WorkflowStatus,
    prs: &[PrRef],
    item_branch: &str,
) -> WaveBoard {
    let plan = match plan {
        Ok(p) => p,
        Err(issue) => {
            return WaveBoard {
                waves: Vec::new(),
                issue: Some(issue),
                unassigned_prs: Vec::new(),
            }
        }
    };
    let owner = assign_prs(&plan, prs, item_branch);
    let lv = levels(&plan);
    let waves = plan
        .waves
        .iter()
        .map(|w| {
            let (state, blocked_by) = state_of(w, progress, status);
            WaveView {
                id: w.id.trim().to_string(),
                title: w.title.clone(),
                summary: w.summary.clone(),
                depends_on: w.depends_on.iter().map(|d| d.trim().to_string()).collect(),
                areas: w.areas.clone(),
                state,
                blocked_by,
                level: lv.get(w.id.trim()).copied().unwrap_or(0),
                compatible_with: plan
                    .waves
                    .iter()
                    .filter(|o| compatible(&plan, &w.id, &o.id))
                    .map(|o| o.id.trim().to_string())
                    .collect(),
                start: progress
                    .starts
                    .get(w.id.trim())
                    .cloned()
                    .unwrap_or_default(),
                end: progress.ends.get(w.id.trim()).cloned().unwrap_or_default(),
                signed_off_at: progress.signed_off.get(w.id.trim()).copied().unwrap_or(0),
                tickets: w.tickets.iter().map(|t| t.trim().to_string()).collect(),
                ticket_urls: Default::default(),
                prs: prs
                    .iter()
                    .filter(|p| {
                        owner
                            .get(&p.url)
                            .is_some_and(|ws| ws.iter().any(|x| x == w.id.trim()))
                    })
                    .map(|p| p.url.clone())
                    .collect(),
            }
        })
        .collect();
    WaveBoard {
        waves,
        issue: None,
        unassigned_prs: prs
            .iter()
            .filter(|p| !owner.contains_key(&p.url))
            .map(|p| p.url.clone())
            .collect(),
    }
}

/// Validate a request to start `ids` in one round, and return them in plan
/// order. Each must exist, not be approved yet, have every dependency
/// approved, and be compatible with every other wave of the round.
/// Restarting the current scope (a change round, a relaunch) passes too.
pub fn check_start(
    plan: &WavePlan,
    progress: &WaveProgress,
    ids: &[String],
) -> Result<Vec<String>, String> {
    let wanted: BTreeSet<&str> = ids
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();
    if wanted.is_empty() {
        return Err("waves: pick at least one wave to start".into());
    }
    for id in &wanted {
        let Some(w) = find(plan, id) else {
            return Err(format!("waves: unknown wave {id}"));
        };
        if progress.done.iter().any(|d| d == id) {
            return Err(format!("waves: {id} is already approved"));
        }
        let missing: Vec<&str> = w
            .depends_on
            .iter()
            .map(|d| d.trim())
            .filter(|d| !progress.done.iter().any(|x| x == d))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "waves: {id} waits for {} to be approved",
                missing.join(", ")
            ));
        }
    }
    let list: Vec<&str> = wanted.iter().copied().collect();
    for (i, a) in list.iter().enumerate() {
        for b in &list[i + 1..] {
            if !compatible(plan, a, b) {
                return Err(format!(
                    "waves: {a} and {b} cannot run together — they depend on each other, touch overlapping areas, or one declares no areas"
                ));
            }
        }
    }
    Ok(plan
        .waves
        .iter()
        .map(|w| w.id.trim().to_string())
        .filter(|id| wanted.contains(id.as_str()))
        .collect())
}

/// Approve the current round's waves: they join `done`, and the scope is
/// cleared for the next round. Their start commits are kept (the record of
/// what each wave changed).
pub fn approve_scope(progress: &WaveProgress, head: Option<&str>, now_ms: i64) -> WaveProgress {
    let mut next = progress.clone();
    for id in &progress.scope {
        if !next.done.contains(id) {
            next.done.push(id.clone());
        }
        // Sealed at the commit the sign-off saw: with its start, the wave's
        // range. No commit (not a git checkout) still records the moment.
        if let Some(sha) = head.filter(|h| !h.is_empty()) {
            next.ends.insert(id.clone(), sha.to_string());
        }
        next.signed_off.insert(id.clone(), now_ms);
    }
    next.scope.clear();
    next
}

/// `abc1234..def5678` for a signed-off wave, from what was recorded.
fn wave_range(progress: &WaveProgress, id: &str) -> Option<String> {
    let short = |s: &String| s.chars().take(7).collect::<String>();
    match (progress.starts.get(id), progress.ends.get(id)) {
        // Nothing moved in the item's checkout (the work is on other
        // branches or in other repositories): no range to give.
        (Some(a), Some(b)) if a == b => None,
        (Some(a), Some(b)) => Some(format!("{}..{}", short(a), short(b))),
        (None, Some(b)) => Some(format!("up to {}", short(b))),
        _ => None,
    }
}

/// `w1 (abc1234..def5678), w2`: the signed-off waves with their ranges.
fn signed_off_list(progress: &WaveProgress) -> String {
    progress
        .done
        .iter()
        .map(|id| match wave_range(progress, id) {
            Some(r) => format!("{id} ({r})"),
            None => id.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The signed-off waves, for an executor kickoff — the work the human
/// accepted, which an executor must not rework on its own initiative. Empty
/// when none is signed off.
fn signed_off_clause(progress: &WaveProgress) -> String {
    if progress.done.is_empty() {
        return String::new();
    }
    format!(
        " Signed off: {} — accepted work: leave it alone unless the note asks.",
        signed_off_list(progress)
    )
}

/// The plan/revise kickoff's wave field: a plan revision may reshape the
/// waves still to come, never the signed-off ones. Empty when none is.
pub fn plan_clause(progress: &WaveProgress) -> String {
    if progress.done.is_empty() {
        return String::new();
    }
    format!(
        " Signed off: {} — accepted waves: never change, remove or renumber them in plan.md or waves.json, and never reuse their ids.",
        signed_off_list(progress)
    )
}

/// Waves neither signed off nor in the current round, in plan order.
fn not_started(plan: &WavePlan, progress: &WaveProgress) -> Vec<String> {
    plan.waves
        .iter()
        .map(|w| w.id.trim().to_string())
        .filter(|id| !progress.done.contains(id) && !progress.scope.contains(id))
        .collect()
}

fn titled(plan: &WavePlan, id: &str) -> String {
    match find(plan, id) {
        Some(w) if !w.title.trim().is_empty() => format!("{id} ({})", w.title.trim()),
        _ => id.to_string(),
    }
}

/// Waves not approved yet, in plan order.
pub fn remaining(plan: &WavePlan, progress: &WaveProgress) -> Vec<String> {
    plan.waves
        .iter()
        .map(|w| w.id.trim().to_string())
        .filter(|id| !progress.done.contains(id))
        .collect()
}

/// The executor kickoff's `Waves:` field on a multi-wave item: the waves this
/// round implements, with their titles and branches, and how many there are
/// in all. Also sent with the `pr` phase, which opens the waves' PRs.
pub fn executor_clause(plan: &WavePlan, progress: &WaveProgress, item_branch: &str) -> String {
    let n = plan.waves.len();
    if progress.scope.is_empty() {
        // A fix round between waves: the executor must not take the next
        // wave on its own initiative — starting one is the human's call.
        return if progress.done.is_empty() {
            format!(" Waves: none of {n} started — a fix round; start no wave.")
        } else {
            format!(
                " Waves: none of {n} — a fix round over the signed-off waves; start no wave.{}",
                signed_off_clause(progress)
            )
        };
    }
    let named: Vec<String> = progress.scope.iter().map(|id| titled(plan, id)).collect();
    let branches: Vec<String> = progress
        .scope
        .iter()
        .map(|id| format!("{id} → `{}`", wave_branch(item_branch, id)))
        .collect();
    format!(
        " Waves: {} of {n}. Wave branches: {} — in every repository the wave changes, base its branch on the branch of the wave it depends on while that wave's PR is unmerged, else on the base branch; its PRs come from that branch and are listed in its `prs`.{}",
        named.join(", "),
        branches.join(", "),
        signed_off_clause(progress)
    )
}

/// The executor kickoff's `Waves:` field when `waves.json` cannot be read:
/// the round still runs (a fix round must never be blocked by the file it may
/// have to repair), on the recorded waves.
pub fn unusable_clause(issue: &str, progress: &WaveProgress) -> String {
    let what = if progress.scope.is_empty() {
        "start no wave".to_string()
    } else {
        format!("continue {}", progress.scope.join(", "))
    };
    format!(
        " Waves: waves.json is unusable ({issue}) — {what}, and repair waves.json in this round.{}",
        signed_off_clause(progress)
    )
}

/// The review kickoff's `Waves:` field on a multi-wave item: the round's
/// waves with their branch, PRs and start commit, the signed-off ones, and
/// the ones not started — which a reviewer must not report as missing.
pub fn review_clause(
    plan: &WavePlan,
    progress: &WaveProgress,
    item_branch: &str,
    wave_prs: &BTreeMap<String, Vec<String>>,
) -> String {
    let mut out = if progress.scope.is_empty() {
        " Waves: none in review.".to_string()
    } else {
        let named: Vec<String> = progress
            .scope
            .iter()
            .map(|id| {
                let mut facts = vec![format!("branch `{}`", wave_branch(item_branch, id))];
                if let Some(prs) = wave_prs.get(id).filter(|p| !p.is_empty()) {
                    facts.push(format!("PRs {}", prs.join(", ")));
                }
                if let Some(sha) = progress.starts.get(id) {
                    facts.push(format!("since {}", sha.chars().take(7).collect::<String>()));
                }
                let t = find(plan, id)
                    .map(|w| w.title.trim().to_string())
                    .unwrap_or_default();
                if t.is_empty() {
                    format!("{id} ({})", facts.join("; "))
                } else {
                    format!("{id} ({t}; {})", facts.join("; "))
                }
            })
            .collect();
        format!(" Waves: {}.", named.join(", "))
    };
    if !progress.done.is_empty() {
        out.push_str(&format!(" Signed off: {}.", signed_off_list(progress)));
    }
    let later = not_started(plan, progress);
    if !later.is_empty() {
        out.push_str(&format!(
            " Not started: {} — not missing.",
            later.join(", ")
        ));
    }
    out
}

/// Which PRs each wave has: the inverse of [`assign_prs`], in PR order.
pub fn prs_by_wave(
    owner: &BTreeMap<String, Vec<String>>,
    prs: &[PrRef],
) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for p in prs {
        for w in owner.get(&p.url).into_iter().flatten() {
            out.entry(w.clone()).or_default().push(p.url.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(id: &str, deps: &[&str], areas: &[&str]) -> Wave {
        Wave {
            id: id.into(),
            title: format!("{id} title"),
            depends_on: deps.iter().map(|s| s.to_string()).collect(),
            areas: areas.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    /// w1 → (w2, w3) → w4; w2 and w3 touch different areas.
    fn project() -> WavePlan {
        WavePlan {
            waves: vec![
                wave("w1", &[], &["src/db/**"]),
                wave("w2", &["w1"], &["src/api/"]),
                wave("w3", &["w1"], &["web/src"]),
                wave("w4", &["w2", "w3"], &["docs"]),
            ],
        }
    }

    fn progress(done: &[&str], scope: &[&str]) -> WaveProgress {
        WaveProgress {
            done: done.iter().map(|s| s.to_string()).collect(),
            scope: scope.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn an_unusable_plan_is_reported_never_guessed() {
        assert!(parse_wave_plan("not json").is_err());
        let dup = r#"{"waves":[{"id":"w1"},{"id":"w1"}]}"#;
        assert!(parse_wave_plan(dup).unwrap_err().contains("twice"));
        let unknown = r#"{"waves":[{"id":"w1","dependsOn":["w9"]}]}"#;
        assert!(parse_wave_plan(unknown)
            .unwrap_err()
            .contains("unknown wave w9"));
        let cycle = r#"{"waves":[{"id":"a","dependsOn":["b"]},{"id":"b","dependsOn":["a"]}]}"#;
        assert!(parse_wave_plan(cycle).unwrap_err().contains("cycle"));
        let ok = r#"{"waves":[{"id":"w1","title":"Schema","areas":["db/"]},{"id":"w2","dependsOn":["w1"]}]}"#;
        assert_eq!(parse_wave_plan(ok).unwrap().waves.len(), 2);
    }

    #[test]
    fn waves_run_in_dependency_order() {
        let b = wave_board(
            Ok(project()),
            &progress(&[], &[]),
            WorkflowStatus::PlanReview,
            &[],
            "",
        );
        let st: Vec<_> = b
            .waves
            .iter()
            .map(|w| (w.id.as_str(), w.state, w.level))
            .collect();
        assert_eq!(
            st,
            vec![
                ("w1", WaveState::Ready, 0),
                ("w2", WaveState::Blocked, 1),
                ("w3", WaveState::Blocked, 1),
                ("w4", WaveState::Blocked, 2),
            ]
        );
        assert_eq!(b.waves[3].blocked_by, vec!["w2", "w3"]);
        // w1 approved: both of its dependents are ready.
        let b = wave_board(
            Ok(project()),
            &progress(&["w1"], &[]),
            WorkflowStatus::DiffReview,
            &[],
            "",
        );
        assert_eq!(b.waves[1].state, WaveState::Ready);
        assert_eq!(b.waves[2].state, WaveState::Ready);
        assert_eq!(b.waves[3].state, WaveState::Blocked);
    }

    #[test]
    fn a_rounds_waves_follow_the_item_status() {
        let p = progress(&["w1"], &["w2", "w3"]);
        let at = |s| wave_board(Ok(project()), &p, s, &[], "").waves[1].state;
        assert_eq!(at(WorkflowStatus::Implementing), WaveState::Active);
        assert_eq!(at(WorkflowStatus::ChangesRequested), WaveState::Active);
        assert_eq!(at(WorkflowStatus::DiffReview), WaveState::Review);
        assert_eq!(at(WorkflowStatus::Reviewing), WaveState::Review);
        // Rewound to the plan: the round no longer stands.
        assert_eq!(at(WorkflowStatus::PlanReview), WaveState::Ready);
    }

    #[test]
    fn compatible_waves_share_no_dependency_path_and_no_area() {
        let plan = project();
        assert!(compatible(&plan, "w2", "w3"));
        // A dependency, direct or not, keeps them apart.
        assert!(!compatible(&plan, "w1", "w2"));
        assert!(!compatible(&plan, "w1", "w4"));
        // Overlapping areas keep them apart too, whatever the spelling.
        let mut overlap = project();
        overlap.waves[2].areas = vec!["./src/api/handlers/*.rs".into()];
        assert!(!compatible(&overlap, "w2", "w3"));
        // A wave that declares no areas runs alone.
        let mut silent = project();
        silent.waves[2].areas.clear();
        assert!(!compatible(&silent, "w2", "w3"));
        let b = wave_board(
            Ok(project()),
            &WaveProgress::default(),
            WorkflowStatus::Draft,
            &[],
            "",
        );
        assert_eq!(b.waves[1].compatible_with, vec!["w3"]);
        assert!(b.waves[0].compatible_with.is_empty());
    }

    #[test]
    fn areas_overlap_by_path_prefix() {
        let v = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(areas_overlap(&v(&["src"]), &v(&["src/a.rs"])));
        assert!(!areas_overlap(&v(&["src/api"]), &v(&["src/apiv2"])));
        assert!(areas_overlap(&v(&["."]), &v(&["web"])));
        assert!(areas_overlap(&v(&["**/*.rs"]), &v(&["web"])));
    }

    #[test]
    fn starting_waves_checks_order_and_compatibility() {
        let plan = project();
        assert_eq!(
            check_start(&plan, &progress(&[], &[]), &["w1".into()]).unwrap(),
            vec!["w1"]
        );
        let early = check_start(&plan, &progress(&[], &[]), &["w2".into()]).unwrap_err();
        assert!(early.contains("waits for w1"), "{early}");
        // Compatible siblings start together, in plan order.
        assert_eq!(
            check_start(&plan, &progress(&["w1"], &[]), &["w3".into(), "w2".into()]).unwrap(),
            vec!["w2", "w3"]
        );
        let mut overlap = project();
        overlap.waves[2].areas = vec!["src".into()];
        let both = check_start(
            &overlap,
            &progress(&["w1"], &[]),
            &["w2".into(), "w3".into()],
        );
        assert!(both.unwrap_err().contains("cannot run together"));
        assert!(check_start(&plan, &progress(&["w1"], &[]), &["w1".into()]).is_err());
        assert!(check_start(&plan, &progress(&[], &[]), &[]).is_err());
        // The current scope restarts (a change round, a relaunch).
        assert!(check_start(&plan, &progress(&["w1"], &["w2"]), &["w2".into()]).is_ok());
    }

    #[test]
    fn approving_a_round_moves_its_waves_to_done() {
        let mut p = progress(&["w1"], &["w2", "w3"]);
        p.starts.insert("w2".into(), "abc".into());
        let next = approve_scope(&p, Some("def4567890"), 42);
        assert_eq!(next.done, vec!["w1", "w2", "w3"]);
        assert!(next.scope.is_empty());
        assert_eq!(next.starts.get("w2").map(String::as_str), Some("abc"));
        // A sign-off seals the round's waves at the commit it saw.
        assert_eq!(next.ends.get("w2").map(String::as_str), Some("def4567890"));
        assert_eq!(next.signed_off.get("w3"), Some(&42));
        assert_eq!(
            next.ends.get("w1"),
            None,
            "an earlier wave keeps its own seal"
        );
        assert_eq!(remaining(&project(), &next), vec!["w4"]);
        // Outside git the moment is still recorded.
        let bare = approve_scope(&p, None, 7);
        assert!(bare.ends.is_empty());
        assert_eq!(bare.signed_off.get("w2"), Some(&7));
    }

    #[test]
    fn prs_belong_to_the_wave_that_lists_them_or_names_their_ticket() {
        let mut plan = project();
        plan.waves[0].tickets = vec!["TT-15360".into(), "TT-14998".into()];
        plan.waves[1].tickets = vec!["TT-14998".into()];
        plan.waves[2].prs = vec!["https://github.com/o/web/pull/9".into()];
        let pr = |url: &str, head: &str| PrRef {
            url: url.into(),
            head_ref: head.into(),
        };
        let prs = vec![
            pr(
                "https://github.com/o/api/pull/1",
                "fix/TT-15360-cache-expiry",
            ),
            // A ticket two waves share decides nothing.
            pr("https://github.com/o/api/pull/2", "feat/TT-14998-contract"),
            // The explicit list wins over any branch name.
            pr("https://github.com/o/web/pull/9", "feat/TT-15360-web"),
            // A longer number is another ticket.
            pr("https://github.com/o/api/pull/3", "feat/TT-153601-other"),
        ];
        let owner = assign_prs(&plan, &prs, "item");
        let of = |u: &str| owner.get(u).cloned().unwrap_or_default();
        assert_eq!(of(&prs[0].url), vec!["w1"]);
        assert!(of(&prs[1].url).is_empty());
        assert_eq!(of(&prs[2].url), vec!["w3"]);
        assert!(of(&prs[3].url).is_empty());
        let b = wave_board(
            Ok(plan),
            &WaveProgress::default(),
            WorkflowStatus::DiffReview,
            &prs,
            "",
        );
        assert_eq!(b.waves[0].prs, vec![prs[0].url.clone()]);
        assert_eq!(b.waves[2].prs, vec![prs[2].url.clone()]);
        assert_eq!(
            b.unassigned_prs,
            vec![prs[1].url.clone(), prs[3].url.clone()]
        );
        assert_eq!(b.waves[0].tickets, vec!["TT-15360", "TT-14998"]);
    }

    #[test]
    fn a_pr_may_belong_to_several_waves_and_matches_by_branch() {
        let mut plan = project();
        // One URL listed by two waves, spelled two ways.
        plan.waves[0].prs = vec!["https://github.com/o/api/pull/5".into()];
        plan.waves[1].prs = vec!["https://GitHub.com/o/api/pull/5/files".into()];
        let prs = vec![
            PrRef {
                url: "https://github.com/o/api/pull/5/".into(),
                head_ref: String::new(),
            },
            // No list names it: its head is w3's branch.
            PrRef {
                url: "https://github.com/o/web/pull/7".into(),
                head_ref: "item-w3".into(),
            },
        ];
        let owner = assign_prs(&plan, &prs, "item");
        assert_eq!(owner[&prs[0].url], vec!["w1", "w2"]);
        assert_eq!(owner[&prs[1].url], vec!["w3"]);
        let b = wave_board(
            Ok(plan),
            &WaveProgress::default(),
            WorkflowStatus::DiffReview,
            &prs,
            "item",
        );
        assert_eq!(b.waves[0].prs, b.waves[1].prs);
        assert!(b.unassigned_prs.is_empty());
        assert_eq!(wave_branch("item", " w2 "), "item-w2");
        assert_eq!(wave_branch("", "w2"), "w2");
    }

    #[test]
    fn the_kickoffs_name_the_rounds_waves() {
        let mut p = progress(&["w1"], &["w2", "w3"]);
        let ex = executor_clause(&project(), &p, "item");
        assert!(
            ex.starts_with(" Waves: w2 (w2 title), w3 (w3 title) of 4. Wave branches: w2 → `item-w2`, w3 → `item-w3`"),
            "{ex}"
        );
        assert!(
            ex.ends_with(" Signed off: w1 — accepted work: leave it alone unless the note asks.")
        );
        // A sealed wave names its range; an empty one does not.
        let mut sealed = p.clone();
        sealed.starts.insert("w1".into(), "1111111aaaa".into());
        sealed.ends.insert("w1".into(), "2222222bbbb".into());
        assert!(executor_clause(&project(), &sealed, "item")
            .contains("Signed off: w1 (1111111..2222222)"));
        sealed.ends.insert("w1".into(), "1111111aaaa".into());
        assert!(executor_clause(&project(), &sealed, "item").contains("Signed off: w1 — "));
        // Fix rounds between waves start nothing, and say what is signed off.
        assert_eq!(
            executor_clause(&project(), &progress(&["w1"], &[]), "item"),
            " Waves: none of 4 — a fix round over the signed-off waves; start no wave. Signed off: w1 — accepted work: leave it alone unless the note asks."
        );
        assert_eq!(
            executor_clause(&project(), &WaveProgress::default(), "item"),
            " Waves: none of 4 started — a fix round; start no wave."
        );
        // Reviews: the round's waves, their branch, PRs and start; what is
        // signed off; what has not started and so is not missing.
        p.starts.insert("w2".into(), "abc1234567".into());
        let mut wp = BTreeMap::new();
        wp.insert("w2".to_string(), vec!["https://g/o/a/pull/1".to_string()]);
        assert_eq!(
            review_clause(&project(), &p, "item", &wp),
            " Waves: w2 (w2 title; branch `item-w2`; PRs https://g/o/a/pull/1; since abc1234), w3 (w3 title; branch `item-w3`). Signed off: w1. Not started: w4 — not missing."
        );
        assert_eq!(
            review_clause(
                &project(),
                &progress(&["w1"], &[]),
                "item",
                &BTreeMap::new()
            ),
            " Waves: none in review. Signed off: w1. Not started: w2, w3, w4 — not missing."
        );
        // Plan revisions keep signed-off waves.
        assert!(
            plan_clause(&progress(&["w1"], &[])).contains("never change, remove or renumber them")
        );
        assert_eq!(plan_clause(&WaveProgress::default()), "");
        // An unreadable waves.json never blocks the round.
        assert_eq!(
            unusable_clause("cycle", &progress(&[], &["w2"])),
            " Waves: waves.json is unusable (cycle) — continue w2, and repair waves.json in this round."
        );
    }
}
