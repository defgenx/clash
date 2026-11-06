// Diff viewer — the pure half: file status and stats, split-view pairing,
// word-level change marks, the file navigator's tree, what a diff can be
// compared with, and the blame gutter. The viewer itself is the *Diff viewer*
// section of app.js; parsing is wf-diff.js's `parseUnifiedDiff`.
//
// Loaded before app.js (plain script) and tested by gui/tests/diff_view.test.js.
(function (global) {
  "use strict";

  /// A parsed file's status, as git's own letters name it.
  function fileStatus(f) {
    if (f.newFile || (!f.oldPath && f.newPath)) return "added";
    if (f.deletedFile || (f.oldPath && !f.newPath)) return "deleted";
    if (f.renamedFrom || (f.oldPath && f.newPath && f.oldPath !== f.newPath)) return "renamed";
    return "modified";
  }

  const STATUS_LETTERS = { added: "A", deleted: "D", renamed: "R", modified: "M" };

  function fileStats(f) {
    let adds = 0;
    let dels = 0;
    for (const h of f.hunks || [])
      for (const l of h.lines) {
        if (l.kind === "add") adds++;
        else if (l.kind === "del") dels++;
      }
    return { adds, dels };
  }

  /// GitHub's five-square change bar: the share of additions and deletions,
  /// rounded so a one-line change still shows one square.
  function statBlocks(adds, dels, n = 5) {
    const total = adds + dels;
    if (!total) return Array(n).fill("none");
    let a = Math.round((adds / total) * n);
    if (adds && !a) a = 1;
    if (dels && a === n) a = n - 1;
    const d = Math.min(n - a, dels ? Math.max(1, Math.round((dels / total) * n)) : 0);
    return [...Array(a).fill("add"), ...Array(d).fill("del"), ...Array(n - a - d).fill("none")];
  }

  /// A hunk's lines as side-by-side rows: context on both sides, and each run
  /// of deletions paired line by line with the additions that follow it.
  /// Indices point into `lines`; `null` is an empty half.
  function splitRows(lines) {
    const rows = [];
    let i = 0;
    while (i < lines.length) {
      if (lines[i].kind === "ctx") {
        rows.push({ left: i, right: i });
        i++;
        continue;
      }
      const dels = [];
      const adds = [];
      while (i < lines.length && lines[i].kind === "del") dels.push(i++);
      while (i < lines.length && lines[i].kind === "add") adds.push(i++);
      for (let k = 0; k < Math.max(dels.length, adds.length); k++) {
        rows.push({ left: dels[k] ?? null, right: adds[k] ?? null });
      }
    }
    return rows;
  }

  const TOKEN = /\w+|\s+|[^\w\s]/g;
  const MAX_TOKENS = 300;

  /// Which characters of an old/new line pair actually changed: `[old, new]`
  /// lists of `[start, end)` ranges, or null when the lines share too little
  /// for word marks to help (the whole line is the change).
  function intralineRanges(a, b) {
    if (a === b) return null;
    const ta = a.match(TOKEN) || [];
    const tb = b.match(TOKEN) || [];
    if (ta.length > MAX_TOKENS || tb.length > MAX_TOKENS) return null;
    // LCS over tokens.
    const m = ta.length;
    const n = tb.length;
    const dp = Array.from({ length: m + 1 }, () => new Uint16Array(n + 1));
    for (let i = m - 1; i >= 0; i--)
      for (let j = n - 1; j >= 0; j--)
        dp[i][j] = ta[i] === tb[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
    const keepA = new Array(m).fill(false);
    const keepB = new Array(n).fill(false);
    let i = 0;
    let j = 0;
    while (i < m && j < n) {
      if (ta[i] === tb[j]) {
        keepA[i++] = true;
        keepB[j++] = true;
      } else if (dp[i + 1][j] >= dp[i][j + 1]) i++;
      else j++;
    }
    const ranges = (tokens, keep) => {
      const out = [];
      let pos = 0;
      let common = 0;
      tokens.forEach((t, k) => {
        const end = pos + t.length;
        if (keep[k]) common += /\S/.test(t) ? t.length : 0;
        else if (out.length && out[out.length - 1][1] === pos) out[out.length - 1][1] = end;
        else out.push([pos, end]);
        pos = end;
      });
      return { out, common, len: tokens.join("").replace(/\s/g, "").length };
    };
    const ra = ranges(ta, keepA);
    const rb = ranges(tb, keepB);
    const shared = (ra.common + rb.common) / Math.max(1, ra.len + rb.len);
    if (shared < 0.3) return null;
    return [ra.out, rb.out];
  }

  /// Every changed line of a hunk that pairs with one on the other side, with
  /// its word marks: index → ranges.
  function hunkWordMarks(lines) {
    const marks = new Map();
    for (const row of splitRows(lines)) {
      if (row.left === null || row.right === null || row.left === row.right) continue;
      const r = intralineRanges(lines[row.left].text, lines[row.right].text);
      if (!r) continue;
      marks.set(row.left, r[0]);
      marks.set(row.right, r[1]);
    }
    return marks;
  }

  function escapeHtml(s) {
    return String(s).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
  }

  /// A line's text as HTML, its changed ranges wrapped in `<mark>`.
  function markedHtml(text, ranges) {
    if (!ranges || !ranges.length) return escapeHtml(text);
    let out = "";
    let pos = 0;
    for (const [s, e] of ranges) {
      out += escapeHtml(text.slice(pos, s)) + `<mark class="dv-word">${escapeHtml(text.slice(s, e))}</mark>`;
      pos = e;
    }
    return out + escapeHtml(text.slice(pos));
  }

  /// The navigator's rows: a folder tree of `paths` (in their given order of
  /// files), single-child folder chains collapsed into one row (`src/app/ui`).
  /// File rows carry the index of their path in `paths`.
  function navRows(paths) {
    const root = { dirs: new Map(), files: [] };
    paths.forEach((p, index) => {
      const parts = p.split("/");
      let node = root;
      for (const part of parts.slice(0, -1)) {
        if (!node.dirs.has(part)) node.dirs.set(part, { dirs: new Map(), files: [] });
        node = node.dirs.get(part);
      }
      node.files.push({ name: parts[parts.length - 1], index });
    });
    const rows = [];
    const walk = (node, depth, prefix) => {
      for (const [name, child] of [...node.dirs].sort(([a], [b]) => a.localeCompare(b))) {
        let label = name;
        let dir = child;
        let path = prefix ? `${prefix}/${name}` : name;
        while (dir.files.length === 0 && dir.dirs.size === 1) {
          const [[n, c]] = [...dir.dirs];
          label += `/${n}`;
          path += `/${n}`;
          dir = c;
        }
        rows.push({ kind: "dir", name: label, path, depth });
        walk(dir, depth + 1, path);
      }
      for (const f of [...node.files].sort((a, b) => a.name.localeCompare(b.name))) {
        rows.push({ kind: "file", name: f.name, index: f.index, depth });
      }
    };
    walk(root, 0, "");
    return rows;
  }

  /// Files whose path contains every word of `query` (case-insensitive).
  function matchesQuery(path, query) {
    const words = String(query || "").toLowerCase().split(/\s+/).filter(Boolean);
    const p = String(path).toLowerCase();
    return words.every((w) => p.includes(w));
  }

  /// What a diff is taken against, as the toolbar says it.
  function targetLabel(target, rev) {
    switch (target) {
      case "staged":
        return "Staged";
      case "unstaged":
        return "Unstaged";
      case "merge-base":
        return `Since branched from ${rev}`;
      case "ref":
        return `Working tree ↔ ${rev}`;
      case "commit":
        return `Commit ${String(rev).slice(0, 8)}`;
      default:
        return "Uncommitted changes";
    }
  }

  /// The "Compare with…" picker's groups: local changes, the branch point of
  /// a branch, a branch or tag tip, and a recent commit. The default branch
  /// leads each branch group (it is what "compare with" usually means).
  function compareChoices({ refs, defaultBranch, commits } = {}) {
    const local = refs?.local || [];
    const remote = refs?.remote || [];
    const lead = (list) => {
      const want = [defaultBranch, `origin/${defaultBranch}`];
      return [...list.filter((b) => want.includes(b)), ...list.filter((b) => !want.includes(b))];
    };
    const branches = lead([...local.filter((b) => b !== refs?.current), ...remote]);
    const groups = [
      {
        label: "Local changes",
        items: [
          { target: "head", label: "Uncommitted changes", detail: "working tree ↔ HEAD" },
          { target: "staged", label: "Staged", detail: "index ↔ HEAD" },
          { target: "unstaged", label: "Unstaged", detail: "working tree ↔ index" },
        ],
      },
    ];
    if (branches.length) {
      groups.push({
        label: "Since branched from",
        items: branches.map((b) => ({ target: "merge-base", rev: b, label: b, detail: "everything this branch changed" })),
      });
      groups.push({
        label: "Compare with branch",
        items: branches.map((b) => ({ target: "ref", rev: b, label: b, detail: "working tree ↔ branch tip" })),
      });
    }
    if (refs?.tags?.length) {
      groups.push({
        label: "Compare with tag",
        items: refs.tags.map((t) => ({ target: "ref", rev: t, label: t, detail: "working tree ↔ tag" })),
      });
    }
    if (commits?.length) {
      groups.push({
        label: "Show a commit",
        items: commits.map((c) => ({
          target: "commit",
          rev: c.sha,
          label: `${c.short} ${c.subject}`,
          detail: c.author,
        })),
      });
    }
    return groups;
  }

  /// "3 h ago" from epoch seconds.
  function relTime(secs, nowMs = Date.now()) {
    const d = Math.max(0, nowMs / 1000 - secs);
    if (d < 60) return "just now";
    const steps = [
      [60, "min"],
      [3600, "h"],
      [86400, "d"],
      [86400 * 30, "mo"],
      [86400 * 365, "y"],
    ];
    let unit = steps[0];
    for (const s of steps) if (d >= s[0]) unit = s;
    return `${Math.floor(d / unit[0])} ${unit[1]} ago`;
  }

  const UNCOMMITTED = "0".repeat(40);

  /// The blame gutter: one entry per line, the commit named only where a run
  /// of lines from it starts, and an age bucket 0 (newest) … 4 (oldest)
  /// relative to the file's own commits.
  function blameRows(blame, nowMs = Date.now()) {
    const lines = blame?.lines || [];
    const commits = blame?.commits || {};
    const times = [...new Set(lines)].map((s) => commits[s]?.time || 0).filter(Boolean);
    const oldest = Math.min(...times);
    const newest = Math.max(...times);
    return lines.map((sha, i) => {
      const c = commits[sha] || {};
      const uncommitted = sha === UNCOMMITTED;
      const span = newest - oldest;
      const age = uncommitted || !span ? 0 : Math.min(4, Math.floor(((newest - (c.time || newest)) / span) * 5));
      return {
        sha,
        first: i === 0 || lines[i - 1] !== sha,
        short: uncommitted ? "" : sha.slice(0, 7),
        author: uncommitted ? "You" : c.author || "",
        when: uncommitted ? "uncommitted" : c.time ? relTime(c.time, nowMs) : "",
        summary: uncommitted ? "Not committed yet" : c.summary || "",
        age,
        uncommitted,
      };
    });
  }

  /// The change to jump to from `current` (an index into `positions`, -1 =
  /// none yet) in direction `dir` (+1 / -1), wrapping around.
  function stepChange(count, current, dir) {
    if (!count) return -1;
    if (current < 0) return dir > 0 ? 0 : count - 1;
    return (current + dir + count) % count;
  }

  /// The viewer's preferences, persisted as `diffView` in gui-state.json.
  const DIFF_PREF_DEFAULTS = Object.freeze({
    mode: "unified",
    wrap: false,
    ignoreWhitespace: false,
    untracked: true,
    context: null,
    nav: true,
  });
  /// Context sizes offered (`null` = git's 3 lines; the last is "whole file").
  const CONTEXT_CHOICES = [null, 10, 25, 100000];

  /// Saved preferences, each field validated; anything unreadable falls back
  /// to its default.
  function diffPrefsFrom(raw) {
    const r = raw && typeof raw === "object" ? raw : {};
    const bool = (k) => (typeof r[k] === "boolean" ? r[k] : DIFF_PREF_DEFAULTS[k]);
    return {
      mode: r.mode === "split" ? "split" : "unified",
      wrap: bool("wrap"),
      ignoreWhitespace: bool("ignoreWhitespace"),
      untracked: bool("untracked"),
      context: CONTEXT_CHOICES.includes(r.context) ? r.context : null,
      nav: bool("nav"),
    };
  }

  /// Whether a target compares the working tree (so untracked files exist).
  function hasWorkTree(target) {
    return target === "head" || target === "unstaged" || target === "ref" || target === "merge-base";
  }

  const api = {
    DIFF_PREF_DEFAULTS,
    CONTEXT_CHOICES,
    diffPrefsFrom,
    hasWorkTree,
    fileStatus,
    STATUS_LETTERS,
    fileStats,
    statBlocks,
    splitRows,
    intralineRanges,
    hunkWordMarks,
    markedHtml,
    navRows,
    matchesQuery,
    targetLabel,
    compareChoices,
    relTime,
    blameRows,
    stepChange,
  };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else global.DiffView = api;
})(typeof window !== "undefined" ? window : globalThis);
