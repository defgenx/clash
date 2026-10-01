// Files panel — the pure half: which tree rows are visible, what each row's
// git mark is, which highlighter language a file gets, and how a path is
// mentioned to an agent. The panel itself is in `app.js` (the `wf-prs.js`
// precedent); the backend half is `explorer_*` over `infrastructure::explorer`.
(function () {
  "use strict";

  /// Depth-first rows of the tree: the root's children, and the children of
  /// every expanded folder whose listing is loaded. `children` maps a folder's
  /// rel path ("" = root) to its sorted entries.
  function fileTreeRows(children, expanded) {
    const rows = [];
    const walk = (dir, depth) => {
      for (const e of children[dir] || []) {
        rows.push({ ...e, depth });
        if (e.isDir && expanded.has(e.rel)) walk(e.rel, depth + 1);
      }
    };
    walk("", 0);
    return rows;
  }

  /// Index a `GitSummary` for row lookups. Untracked folders arrive whole
  /// (`dir/`), so they cover their contents; every ancestor of a change is
  /// "dirty", which is what a collapsed folder shows.
  function statusIndex(summary) {
    const marks = new Map();
    const untrackedDirs = [];
    const dirty = new Set();
    for (const e of summary?.entries || []) {
      if (e.rel.endsWith("/")) {
        const dir = e.rel.slice(0, -1);
        untrackedDirs.push(dir);
        marks.set(dir, e.mark);
      } else {
        marks.set(e.rel, e.mark);
      }
      const parts = e.rel.replace(/\/$/, "").split("/");
      for (let i = 1; i < parts.length; i++) dirty.add(parts.slice(0, i).join("/"));
    }
    return { marks, untrackedDirs, dirty };
  }

  /// The mark a row shows: its own status, its untracked ancestor's, or
  /// "dirty" for a folder holding changes. null when clean.
  function markFor(index, rel, isDir) {
    if (!index) return null;
    const own = index.marks.get(rel);
    if (own) return own;
    if (index.untrackedDirs.some((d) => rel.startsWith(d + "/"))) return "untracked";
    if (isDir && index.dirty.has(rel)) return "dirty";
    return null;
  }

  const MARK_GLYPHS = {
    modified: { letter: "M", title: "Modified" },
    added: { letter: "A", title: "Added" },
    deleted: { letter: "D", title: "Deleted" },
    renamed: { letter: "R", title: "Renamed" },
    untracked: { letter: "U", title: "Untracked" },
    conflicted: { letter: "!", title: "Conflict" },
    dirty: { letter: "•", title: "Contains changes" },
  };

  function markGlyph(mark) {
    return MARK_GLYPHS[mark] || null;
  }

  // Extension / file name → a language in the vendored highlight.js "common"
  // bundle. Anything absent renders as plain text rather than a wrong guess.
  const LANG_BY_EXT = {
    rs: "rust", js: "javascript", mjs: "javascript", cjs: "javascript", jsx: "javascript",
    ts: "typescript", tsx: "typescript", py: "python", rb: "ruby", go: "go",
    java: "java", kt: "kotlin", kts: "kotlin", swift: "swift", c: "c", h: "c",
    cc: "cpp", cpp: "cpp", cxx: "cpp", hpp: "cpp", cs: "csharp", m: "objectivec",
    php: "php", pl: "perl", lua: "lua", r: "r", sql: "sql", sh: "bash", bash: "bash",
    zsh: "bash", fish: "shell", css: "css", scss: "scss", less: "less",
    html: "xml", htm: "xml", xml: "xml", svg: "xml", vue: "xml", json: "json",
    jsonl: "json", yaml: "yaml", yml: "yaml", toml: "ini", ini: "ini", cfg: "ini",
    md: "markdown", markdown: "markdown", diff: "diff", patch: "diff",
    graphql: "graphql", gql: "graphql", mk: "makefile", vb: "vbnet", wat: "wasm",
  };
  const LANG_BY_NAME = {
    makefile: "makefile", gnumakefile: "makefile", gemfile: "ruby", rakefile: "ruby",
    ".bashrc": "bash", ".zshrc": "bash", ".profile": "bash", "cargo.lock": "ini",
  };

  function hljsLanguage(path) {
    const name = String(path || "").split("/").pop().toLowerCase();
    if (LANG_BY_NAME[name]) return LANG_BY_NAME[name];
    const dot = name.lastIndexOf(".");
    return dot > 0 ? LANG_BY_EXT[name.slice(dot + 1)] || null : null;
  }

  /// `abs` relative to `base` when it lies under it, else `abs` unchanged.
  function relativeTo(base, abs) {
    const b = String(base || "").replace(/\/+$/, "");
    if (b && abs.startsWith(b + "/")) return abs.slice(b.length + 1);
    return abs;
  }

  /// What "Mention in session" types: `@path ` relative to the session's
  /// directory (Claude Code resolves `@` against its cwd), quoted when the
  /// path has whitespace. The trailing space closes the `@` completion menu.
  function mentionText(sessionCwd, abs) {
    const p = relativeTo(sessionCwd, abs);
    return /\s/.test(p) ? `@"${p}" ` : `@${p} `;
  }

  function joinPath(root, rel) {
    if (!rel) return root;
    return `${String(root).replace(/\/+$/, "")}/${rel}`;
  }

  function escapeHtml(s) {
    return String(s).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");
  }

  /// A finder result with its matched characters bolded (positions index
  /// characters, not UTF-16 units).
  function highlightMatch(rel, positions) {
    const set = new Set(positions || []);
    let out = "";
    let i = 0;
    for (const ch of String(rel)) {
      out += set.has(i) ? `<b>${escapeHtml(ch)}</b>` : escapeHtml(ch);
      i++;
    }
    return out;
  }

  function formatSize(bytes) {
    const n = Number(bytes) || 0;
    if (n < 1024) return `${n} B`;
    if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
    return `${(n / 1024 / 1024).toFixed(1)} MB`;
  }

  /// Folders to re-list on a refresh: the root plus every expanded folder,
  /// minus folders under a collapsed ancestor (they are not on screen).
  function foldersToRefresh(expanded) {
    const open = [...expanded].filter((rel) => {
      const parts = rel.split("/");
      for (let i = 1; i < parts.length; i++) {
        if (!expanded.has(parts.slice(0, i).join("/"))) return false;
      }
      return true;
    });
    return ["", ...open.sort()];
  }

  const api = {
    fileTreeRows,
    statusIndex,
    markFor,
    markGlyph,
    hljsLanguage,
    mentionText,
    joinPath,
    highlightMatch,
    formatSize,
    foldersToRefresh,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
