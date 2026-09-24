// Pure agent-choice model for workflow launches — which agent CLIs a start
// may run on, which are greyed out because their binary does not resolve, and
// which one is pre-selected. No DOM here: app.js renders it (the launch picker
// and the composers' agent row), gui/tests/wf_agent.test.js exercises it.
//
// The backend stays the authority: `launch_agent` refuses an unavailable
// binary, so a stale "available" here costs an error, never a broken spawn.
(function () {
  /// Every agent a workflow session can run on, each with whether it can be
  /// picked right now. A settings field that never arrived (the command
  /// failed) counts as available rather than greying out a working agent.
  function wfAgentChoices(settings = {}) {
    const claude = settings.claudeAvailable !== false;
    const omp = settings.ompAvailable !== false;
    return [
      {
        value: "claude",
        label: "Claude Code",
        available: claude,
        reason: claude ? "" : "claude not found — set the Claude binary in Settings",
      },
      {
        value: "omp",
        label: "OMP",
        available: omp,
        reason: omp
          ? ""
          : `omp not found (${settings.ompBin || "omp"}) — set the OMP binary in Settings`,
      },
    ];
  }

  /// The pre-selected agent: the item's own (the last one it was started on),
  /// else the global workflow agent — unless that one is unavailable, in which
  /// case the first agent that is. With nothing available the preference
  /// stands, so the backend's refusal names the binary that is missing.
  function wfAgentDefault(itemAgent, settings = {}) {
    const choices = wfAgentChoices(settings);
    const want = itemAgent || settings.workflowAgent || "claude";
    const hit = choices.find((c) => c.value === want && c.available);
    if (hit) return hit.value;
    const any = choices.find((c) => c.available);
    return any ? any.value : want;
  }

  const api = { wfAgentChoices, wfAgentDefault };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
