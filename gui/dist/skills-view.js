// Pure model of the ☰ Skills tab: one row per skill name, one chip per agent
// CLI, and which harness the viewer opens. No DOM here: app.js renders it,
// gui/tests/skills_view.test.js exercises it. The backend (`list_skills`)
// groups the copies; the states are the installer's (docs/workflows.md →
// *Skills per agent CLI*).
(function () {
  const HARNESSES = [
    { agent: "claude", short: "CC", label: "Claude Code" },
    { agent: "omp", short: "OMP", label: "OMP" },
    { agent: "hermes", short: "HM", label: "Hermes" },
  ];

  const STATES = {
    current: { glyph: "✓", title: "current — what clash ships" },
    edited: { glyph: "✎", title: "hand-edited since clash wrote it" },
    outdated: { glyph: "⚠", title: "outdated — refreshed at the next start" },
    missing: { glyph: "—", title: "not installed" },
    present: { glyph: "", title: "installed" },
  };

  /// The chips of one row, in harness order. A harness whose skills folder
  /// does not exist sends no copy and gets no chip; an unmanaged skill gets a
  /// chip only where it is installed.
  function skillChips(skill) {
    const copies = new Map((skill.copies || []).map((c) => [c.agent, c]));
    const out = [];
    for (const h of HARNESSES) {
      const c = copies.get(h.agent);
      if (!c) continue;
      const s = STATES[c.state] || STATES.present;
      out.push({
        agent: h.agent,
        short: h.short,
        state: c.state,
        glyph: s.glyph,
        title: `${h.label}: ${s.title}`,
      });
    }
    return out;
  }

  /// Something about this skill wants attention: a managed copy that is not
  /// what clash ships.
  function skillNeedsAttention(skill) {
    return !!skill.managed && (skill.copies || []).some((c) => c.state === "edited" || c.state === "outdated");
  }

  /// The harnesses the viewer's switch offers (those with a copy on disk),
  /// and which one opens: the remembered pick when it has a copy, else the
  /// first copy that needs attention, else the first copy.
  function skillViewerChoice(skill, remembered) {
    const onDisk = skillChips(skill).filter((c) => c.state !== "missing");
    const agents = onDisk.map((c) => c.agent);
    let pick = agents.includes(remembered) ? remembered : "";
    if (!pick) pick = (onDisk.find((c) => c.state === "edited" || c.state === "outdated") || onDisk[0] || {}).agent || "";
    return { agents, pick };
  }

  /// Rows in display order under their group heading: clash's own first.
  function skillGroups(skills) {
    const managed = skills.filter((s) => s.managed);
    const other = skills.filter((s) => !s.managed);
    return [
      { title: "Managed by clash", skills: managed },
      { title: "Other skills", skills: other },
    ].filter((g) => g.skills.length);
  }

  const api = { SKILL_HARNESSES: HARNESSES, skillChips, skillNeedsAttention, skillViewerChoice, skillGroups };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
