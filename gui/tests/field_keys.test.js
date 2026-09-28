// node --test gui/tests/ — forward delete and the function-key filter that
// keep macOS from typing U+F728 into a text field when Delete/Suppr is pressed.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { isFunctionKeyText, forwardDeleteRange } = require("../dist/field-keys.js");

const del = (value, start, end = start, mods) => {
  const r = forwardDeleteRange(value, start, end, mods);
  return r && value.slice(0, r.from) + value.slice(r.to);
};

test("forward delete removes the next character", () => {
  assert.equal(del("hello", 0), "ello");
  assert.equal(del("hello", 4), "hell");
});

test("forward delete at the end deletes nothing", () => {
  assert.equal(forwardDeleteRange("hello", 5, 5), null);
});

test("a selection is deleted whatever the modifiers", () => {
  assert.equal(del("hello world", 2, 7), "heorld");
  assert.equal(del("hello world", 2, 7, { altKey: true }), "heorld");
});

test("a surrogate pair is removed whole", () => {
  assert.equal(del("a😀b", 1), "ab");
});

test("⌥ deletes to the end of the next word", () => {
  assert.equal(del("foo bar baz", 3, 3, { altKey: true }), "foo baz");
  assert.equal(del("foo bar", 1, 1, { altKey: true }), "f bar");
  assert.equal(del("foo --", 3, 3, { altKey: true }), "foo");
});

test("⌘ deletes to the end of the line", () => {
  assert.equal(del("one two\nthree", 3, 3, { metaKey: true }), "one\nthree");
  assert.equal(del("one\nthree", 3, 3, { metaKey: true }), "onethree");
});

test("only macOS function-key characters are filtered", () => {
  assert.equal(isFunctionKeyText(""), true); // NSDeleteFunctionKey
  assert.equal(isFunctionKeyText(""), true); // Home, End
  assert.equal(isFunctionKeyText("a"), false);
  assert.equal(isFunctionKeyText("a"), false);
  assert.equal(isFunctionKeyText(""), false);
  assert.equal(isFunctionKeyText(null), false);
});

test("index.html loads field-keys.js before app.js", () => {
  const html = fs.readFileSync(path.join(__dirname, "../dist/index.html"), "utf8");
  const mod = html.indexOf('src="field-keys.js"');
  assert.ok(mod !== -1 && mod < html.indexOf('src="app.js"'));
});
