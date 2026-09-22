// Shared by the render smoke tests: pull one top-level function out of app.js
// and give it a DOM to run against.
//
// app.js cannot be `require`d (it is a plain script against the Tauri globals),
// so the alternative to this is discovering a dangling reference at click time,
// in a webview with no console, while clash.log still shows a healthy boot line.
const assert = require("node:assert/strict");

/// Extract a top-level `function name(...) {...}` by brace matching, starting
/// at the BODY brace — the parameter list may hold a destructuring `{…}` of its
/// own, which a count-from-the-first-brace would mistake for the body.
function extractFunction(source, name) {
  let start = source.indexOf(`function ${name}(`);
  assert.ok(start >= 0, `${name} must exist in app.js`);
  // Keep a leading `async`: without it the extracted body still contains
  // `await` and fails to parse, which reads as a bug in the function under
  // test rather than in this extractor.
  if (source.slice(start - 6, start) === "async ") start -= 6;
  const body = /\)\s*\{/.exec(source.slice(start));
  assert.ok(body, `${name} must have a body`);
  let i = start + body.index + body[0].length - 1;
  let depth = 0;
  for (; i < source.length; i++) {
    if (source[i] === "{") depth++;
    else if (source[i] === "}" && --depth === 0) break;
  }
  return source.slice(start, i + 1);
}

/// A DOM-element stand-in: absorbs any property set, records children, and
/// no-ops the methods the render paths call.
function stubEl(tag = "div") {
  const target = { tagName: tag.toUpperCase(), children: [] };
  return new Proxy(target, {
    get(t, p) {
      if (p === "classList")
        return {
          add(c) {
            t.className = `${t.className || ""} ${c}`.trim();
          },
          remove() {},
          toggle() {},
        };
      if (p === "style" || p === "dataset" || p === "attrs") return t[p] || (t[p] = {});
      // Recorded rather than no-oped: `sandbox`, `role` and `aria-selected`
      // are load-bearing (an iframe that gains `allow-scripts` stops being
      // inert), so a test has to be able to read them back.
      if (p === "setAttribute")
        return (k, v) => {
          (t.attrs || (t.attrs = {}))[k] = String(v);
        };
      if (p === "getAttribute") return (k) => (t.attrs || {})[k] ?? null;
      if (p === "removeAttribute")
        return (k) => {
          delete (t.attrs || {})[k];
        };
      // Accumulated into `__html` so an assertion can see what a render wrote
      // this way — the empty states use it instead of a child element.
      if (p === "insertAdjacentHTML")
        return (_pos, html) => {
          t.__html = `${t.__html || ""}${html}`;
        };
      if (p === "appendChild" || p === "append")
        return (...cs) => {
          t.children.push(...cs);
          return cs[0];
        };
      if (
        [
          "remove",
          "focus",
          "addEventListener",
          "prepend",
          "scrollIntoView",
          "setSelectionRange",
          "dispatchEvent",
        ].includes(p)
      )
        return () => {};
      if (p === "querySelector") return () => stubEl();
      if (p === "querySelectorAll") return () => [];
      return t[p];
    },
    set(t, p, v) {
      t[p] = v;
      return true;
    },
  });
}

/// Every element created under a root, depth-first — for asserting on what a
/// render produced without a real DOM query engine.
function descendants(el) {
  const out = [];
  for (const c of el.children || []) {
    out.push(c);
    out.push(...descendants(c));
  }
  return out;
}

module.exports = { extractFunction, stubEl, descendants };
