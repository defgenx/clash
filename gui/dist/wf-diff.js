// Pure unified-diff parsing for the workflow diff view (render side only —
// annotation anchoring is backend-owned, see application::workflow in Rust).
//
// Kept in its own file with a UMD-ish tail so `node --test gui/tests/` can
// exercise the line-numbering logic (a mis-parse here would mis-anchor every
// new comment: the composer reads data-line off the rendered DOM).
(function (global) {
  "use strict";

  function stripGitPrefix(p) {
    if (p.startsWith("a/") || p.startsWith("b/")) return p.slice(2);
    return p;
  }

  // "@@ -a[,b] +c[,d] @@ ctx" -> { oldStart, newStart } or null.
  function parseHunkHeader(line) {
    const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if (!m) return null;
    return { oldStart: parseInt(m[1], 10), newStart: parseInt(m[2], 10) };
  }

  /// Parse a unified diff into files → hunks → numbered lines.
  /// Line shape: { kind: "add"|"del"|"ctx", oldNo: number|null, newNo: number|null, text }.
  /// Mirrors the Rust structural parser's semantics (git prefixes stripped,
  /// /dev/null → "", `rename from` tracked, metadata lines skipped).
  function parseUnifiedDiff(text) {
    const files = [];
    let file = null;
    let hunk = null;
    let oldNo = 0;
    let newNo = 0;
    let inHunk = false;

    // Rust-`lines()` semantics: a trailing newline does not produce a final
    // empty line (split("\n") would, phantom-extending the last hunk).
    const rawLines = String(text ?? "").split("\n");
    if (rawLines.length && rawLines[rawLines.length - 1] === "") rawLines.pop();
    for (const line of rawLines) {
      if (line.startsWith("diff --git ")) {
        inHunk = false;
        hunk = null;
        file = { oldPath: "", newPath: "", renamedFrom: null, hunks: [] };
        const rest = line.slice("diff --git ".length);
        const sp = rest.indexOf(" ");
        if (sp > 0) {
          file.oldPath = stripGitPrefix(rest.slice(0, sp));
          file.newPath = stripGitPrefix(rest.slice(sp + 1));
        }
        files.push(file);
        continue;
      }
      if (!file) {
        // Headerless diff: synthesize a file on "---".
        if (line.startsWith("--- ")) {
          file = { oldPath: "", newPath: "", renamedFrom: null, hunks: [] };
          files.push(file);
        } else {
          continue;
        }
      }
      if (!inHunk) {
        if (line.startsWith("rename from ")) {
          file.renamedFrom = line.slice("rename from ".length);
          continue;
        }
        if (line.startsWith("rename to ")) {
          file.newPath = line.slice("rename to ".length);
          continue;
        }
        if (line.startsWith("--- ")) {
          const p = line.slice(4);
          file.oldPath = p === "/dev/null" ? "" : stripGitPrefix(p);
          continue;
        }
        if (line.startsWith("+++ ")) {
          const p = line.slice(4);
          file.newPath = p === "/dev/null" ? "" : stripGitPrefix(p);
          continue;
        }
      }
      if (line.startsWith("@@")) {
        const h = parseHunkHeader(line);
        if (h) {
          hunk = { header: line, lines: [] };
          file.hunks.push(hunk);
          oldNo = h.oldStart;
          newNo = h.newStart;
          inHunk = true;
        }
        continue;
      }
      if (!inHunk || !hunk) continue;
      if (line.startsWith("+")) {
        hunk.lines.push({ kind: "add", oldNo: null, newNo: newNo++, text: line.slice(1) });
      } else if (line.startsWith("-")) {
        hunk.lines.push({ kind: "del", oldNo: oldNo++, newNo: null, text: line.slice(1) });
      } else if (line.startsWith(" ")) {
        hunk.lines.push({ kind: "ctx", oldNo: oldNo++, newNo: newNo++, text: line.slice(1) });
      } else if (line === "") {
        // Blank context line git sometimes emits without the leading space.
        hunk.lines.push({ kind: "ctx", oldNo: oldNo++, newNo: newNo++, text: "" });
      }
      // "\ No newline at end of file" and other metadata: skipped.
    }
    return files;
  }

  /// Display path of a parsed file (new side wins, old for deletions).
  function diffFilePath(f) {
    return f.newPath || f.oldPath;
  }

  /// Changed-line count (adds + dels) — drives the collapse guards.
  function diffFileChangedLines(f) {
    let n = 0;
    for (const h of f.hunks) for (const l of h.lines) if (l.kind !== "ctx") n++;
    return n;
  }

  const api = { parseUnifiedDiff, diffFilePath, diffFileChangedLines };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(global, api);
})(typeof window !== "undefined" ? window : globalThis);
