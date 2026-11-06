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

  // File name → which icon it gets and its tint (a theme colour, see style.css
  // `.fk-*`). Tests and lockfiles are recognised by name before extension.
  // Tables as Maps: the highlighter test reads every quoted object value in
  // this file as a highlight.js language.
  const KIND_BY_EXT = new Map();
  for (const [kind, exts] of [
    ["code", "rs js mjs cjs jsx ts tsx py rb go java kt kts swift c h cc cpp cxx hpp cs m php pl lua r scala ex exs erl hs ml clj dart zig nim vue svelte"],
    ["markup", "html htm css scss less sass xml svg"],
    ["data", "json jsonl json5 yaml yml toml ini cfg csv tsv sql graphql gql proto"],
    ["doc", "md markdown mdx txt rst adoc org tex log"],
    ["image", "png jpg jpeg gif webp bmp ico icns tiff heic avif"],
    ["shell", "sh bash zsh fish ps1 bat cmd"],
    ["archive", "zip tar gz tgz bz2 xz 7z rar jar dmg"],
    ["lock", "lock"],
  ])
    for (const e of exts.split(" ")) KIND_BY_EXT.set(e, kind);
  const KIND_ICONS = new Map([
    ["code", "file-code"],
    ["markup", "file-code"],
    ["data", "file-data"],
    ["doc", "file-text"],
    ["image", "file-image"],
    ["shell", "file-shell"],
    ["archive", "file-archive"],
    ["lock", "file-lock"],
    ["config", "file-config"],
    ["test", "file-test"],
    ["git", "file-git"],
    ["plain", "file"],
  ]);

  /// `{ icon, tone }` for a file row: the icon name (app.js ICONS) and the
  /// tint class suffix.
  function fileKind(name) {
    const n = String(name || "").toLowerCase();
    let kind = "plain";
    const dot = n.lastIndexOf(".");
    const ext = dot > 0 ? n.slice(dot + 1) : "";
    if (n.startsWith(".git")) kind = "git";
    else if (/(^|[._-])(test|spec)s?\.[^.]+$/.test(n) || /_test\.[^.]+$/.test(n)) kind = "test";
    else if (n === "package-lock.json" || n === "yarn.lock" || n === "pnpm-lock.yaml" || n === "cargo.lock") kind = "lock";
    else if (n === "dockerfile" || n === "makefile" || n.startsWith(".env") || /^\.[a-z0-9]+rc$/.test(n) || n === ".editorconfig") kind = "config";
    else if (ext && KIND_BY_EXT.has(ext)) kind = KIND_BY_EXT.get(ext);
    else if (n.startsWith(".")) kind = "config";
    return { icon: KIND_ICONS.get(kind), tone: kind };
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

  /// What a terminal drop types: the paths, space-separated, each escaped the
  /// way iTerm2/Terminal escape a Finder drop (a backslash before every
  /// shell-special character), so an agent's image-path detection and a shell
  /// both read it as one argument per file.
  function dropPasteText(paths) {
    return paths
      .filter(Boolean)
      .map((p) => String(p).replace(/[^A-Za-z0-9_\-.,:\/@%+=]/g, (c) => `\\${c}`))
      .join(" ");
  }

  /// Local paths carried by a drop's `text/uri-list` (file:// URLs only;
  /// comment lines and other schemes are skipped).
  function fileUriPaths(uriList) {
    const out = [];
    for (const line of String(uriList || "").split(/\r?\n/)) {
      const t = line.trim();
      if (!t || t.startsWith("#") || !t.startsWith("file://")) continue;
      try {
        const u = new URL(t);
        if (u.host && u.host !== "localhost") continue;
        out.push(decodeURIComponent(u.pathname));
      } catch (_) {
        /* not a URL */
      }
    }
    return out;
  }

  const api = {
    fileKind,
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
    dropPasteText,
    fileUriPaths,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
