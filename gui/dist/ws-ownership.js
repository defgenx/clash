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
// are the two halves of keeping ownership pinned to the session:
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
  function ownershipTransfers(vanished, resolved, known, owned) {
    const moved = new Map();
    const taken = new Set(owned);
    vanished.forEach((id, i) => {
      const to = resolved ? resolved[i] : null;
      if (to && to !== id && known.has(to) && !taken.has(to)) {
        moved.set(id, to);
        taken.add(to);
      }
    });
    return moved;
  }

  /// Apply a prune pass in place: every id in `gone` is replaced by its
  /// `moved` target (ownership transfer) or dropped (dead session). Returns
  /// true when any workspace's ownership changed.
  function pruneOwnership(workspaces, gone, moved) {
    let changed = false;
    for (const w of workspaces) {
      const sessions = w.sessions || [];
      if (!sessions.some((id) => gone.has(id))) continue;
      w.sessions = [
        ...new Set(
          sessions.flatMap((id) =>
            gone.has(id) ? (moved.has(id) ? [moved.get(id)] : []) : [id]
          )
        ),
      ];
      changed = true;
    }
    return changed;
  }

  const api = {
    persistedSessionIds,
    remapWorkspaceIds,
    ownershipTransfers,
    pruneOwnership,
  };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(global, api);
})(typeof window !== "undefined" ? window : globalThis);
