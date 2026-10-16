// Workspace session-ownership bookkeeping — pure decisions, no DOM, no IPC.
//
// A GUI workspace owns a set of session ids (`w.sessions`), persisted in
// gui-state.json alongside its pane layout. The ids are Claude *conversation*
// ids, and those are not stable: `/clear` re-keys the registry via the status
// hook, and `claude --resume` forks the conversation into a brand-new
// transcript (healed into the registry by `heal_registry_forks` at backend
// startup). Either way `list_sessions` starts reporting a DIFFERENT id for the
// same session.
//
// If ownership isn't carried across that rename, the old id is pruned as
// "gone" and the current id — owned by nobody — surfaces under UNASSIGNED on
// every relaunch, for a session the user never launched anew. These helpers
// are the two halves of keeping ownership pinned to the session (the third,
// `moveTabToWorkspace`, is the user's own reassignment of any tab):
//   • restore: resolve every persisted id forward before matching the list
//   • refresh: transfer ownership instead of pruning when an id merely moved
//
// Loaded before app.js (plain script, no build step) and unit-tested with
// `node --test gui/tests/`.
(function (global) {
  /// Every session id persisted across all workspaces — pane slots AND
  /// ownership — deduped, with non-session slots (browser tabs, shell
  /// terminals, view tabs, empty panes) filtered out by `isSessionId`.
  ///
  /// Ownership ids are included deliberately: a session that is owned but not
  /// currently open in a pane is exactly the one whose rename would otherwise
  /// go unnoticed.
  function persistedSessionIds(workspaces, isSessionId) {
    const out = new Set();
    for (const w of workspaces) {
      for (const id of [...(w.panes || []), ...(w.sessions || [])]) {
        if (id && isSessionId(id)) out.add(id);
      }
    }
    return [...out];
  }

  /// Rewrite persisted ids in place through `remap` (old id -> current id).
  /// Panes and ownership both move; ownership is deduped because a rename can
  /// collapse two persisted ids onto one session (the pre-fork id and the
  /// current one both owned). Returns true when anything changed.
  function remapWorkspaceIds(workspaces, remap) {
    if (!remap || !remap.size) return false;
    let changed = false;
    for (const w of workspaces) {
      const panes = (w.panes || []).map((p) => (p && remap.get(p)) || p);
      const sessions = [...new Set((w.sessions || []).map((s) => remap.get(s) || s))];
      if (String(panes) !== String(w.panes) || String(sessions) !== String(w.sessions)) {
        changed = true;
      }
      w.panes = panes;
      w.sessions = sessions;
    }
    return changed;
  }

  /// Decide which vanished ids merely moved. `vanished[i]` resolved forward to
  /// `resolved[i]` (as returned by the `resolve_session_ids` command, which
  /// passes unknown ids through unchanged).
  ///
  /// A move is only accepted when the target is actually in the current session
  /// list and is not already owned — one session belongs to exactly one
  /// workspace, so a claimed id is never re-assigned. Everything else is a
  /// genuinely dead session and drops out.
  ///
  /// Resolving forward only covers half the renames: a live session's row keeps
  /// the id its PTY was spawned with, which is the *older* id. So `unowned` /
  /// `unownedResolved` (listed rows nobody owns, and where they resolve) also
  /// count as a target when they resolve to the same session as the vanished id.
  function ownershipTransfers(vanished, resolved, known, owned, unowned = [], unownedResolved = null) {
    const moved = new Map();
    const taken = new Set(owned);
    vanished.forEach((id, i) => {
      const to = resolved ? resolved[i] : null;
      if (!to) return;
      if (to !== id && known.has(to) && !taken.has(to)) {
        moved.set(id, to);
        taken.add(to);
        return;
      }
      if (!unownedResolved) return;
      const j = unowned.findIndex(
        (u, k) => u !== id && !taken.has(u) && known.has(u) && unownedResolved[k] === to
      );
      if (j >= 0) {
        moved.set(id, unowned[j]);
        taken.add(unowned[j]);
      }
    });
    return moved;
  }

  /// Apply a prune pass in place: every id in `gone` is replaced by its
  /// `moved` target (ownership transfer) or dropped (dead session). An id
  /// `keep` holds (an open tab) stays owned beside its target. Returns true
  /// when any workspace's ownership changed.
  function pruneOwnership(workspaces, gone, moved, keep = () => false) {
    let changed = false;
    for (const w of workspaces) {
      const sessions = w.sessions || [];
      if (!sessions.some((id) => gone.has(id))) continue;
      const next = [
        ...new Set(
          sessions.flatMap((id) => {
            if (!gone.has(id)) return [id];
            const kept = keep(id) ? [id] : [];
            return moved.has(id) ? [...kept, moved.get(id)] : kept;
          })
        ),
      ];
      if (String(next) === String(sessions)) continue;
      w.sessions = next;
      changed = true;
    }
    return changed;
  }

  /// Move tab `id` (a session, shell, browser or view tab) to workspace `to`,
  /// in place: every other workspace drops its ownership and empties the panes
  /// holding it — or holding any tab `follows(paneId)` says goes with it (a
  /// session's own conversation/diff views). The target takes ownership and,
  /// with `place` (an open tab), shows it in its focused pane when that is
  /// empty, else in its first empty pane, else in the focused pane (whose
  /// previous content stays a tab). `along` are ids that change owner with it
  /// but are never placed (a workflow item's agent sessions). Returns false
  /// when `to` is not a workspace.
  function moveTabToWorkspace(workspaces, id, to, { follows = () => false, place = true, along = [] } = {}) {
    const target = workspaces[to];
    if (!target || !id) return false;
    const owned = new Set([id, ...along]);
    const goes = (p) => !!p && (owned.has(p) || follows(p));
    workspaces.forEach((w, i) => {
      if (i === to) return;
      w.panes = (w.panes || []).map((p) => (goes(p) ? null : p));
      w.sessions = (w.sessions || []).filter((x) => !owned.has(x));
    });
    target.sessions = target.sessions || [];
    for (const x of owned) if (!target.sessions.includes(x)) target.sessions.push(x);
    if (!place) return true;
    if (!target.panes || !target.panes.length) target.panes = [null];
    const at = target.panes.indexOf(id);
    if (at >= 0) {
      target.focused = at;
      return true;
    }
    const f = Number.isInteger(target.focused) && target.focused < target.panes.length ? target.focused : 0;
    const empty = target.panes.indexOf(null);
    const slot = target.panes[f] == null ? f : empty >= 0 ? empty : f;
    target.panes[slot] = id;
    target.focused = slot;
    return true;
  }

  const api = {
    persistedSessionIds,
    remapWorkspaceIds,
    ownershipTransfers,
    pruneOwnership,
    moveTabToWorkspace,
  };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(global, api);
})(typeof window !== "undefined" ? window : globalThis);
