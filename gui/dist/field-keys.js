// Text-field editing keys clash must implement itself — pure, no DOM.
//
// On macOS the forward-delete key (fn+⌫, or "Suppr"/⌦ on an extended
// keyboard) reaches a WKWebView text field as the private-use character
// U+F728 (NSDeleteFunctionKey). With no Edit menu (see handleInputClipboard in
// app.js), nothing turns it into a deletion and the field inserts that
// character instead. The other function keys share the U+F700–U+F8FF block.
//
// Loaded before app.js (plain script, no build step) and unit-tested with
// `node --test gui/tests/`.
(function (global) {
  const WORD = /[\p{L}\p{N}_]/u;

  /// True when `data` is (only) macOS function-key characters, which must
  /// never be inserted into a text field.
  function isFunctionKeyText(data) {
    if (!data) return false;
    for (const ch of data) {
      const c = ch.codePointAt(0);
      if (c < 0xf700 || c > 0xf8ff) return false;
    }
    return true;
  }

  /// The range a forward delete removes from `value` with the selection at
  /// [start, end): the selection if there is one, otherwise the next
  /// character (a whole code point), the rest of the next word with ⌥, or the
  /// rest of the line with ⌘. Returns null when there is nothing to delete.
  function forwardDeleteRange(value, start, end, { altKey = false, metaKey = false } = {}) {
    if (end > start) return { from: start, to: end };
    if (start >= value.length) return null;
    let to = start;
    if (metaKey) {
      const nl = value.indexOf("\n", start);
      to = nl === -1 ? value.length : nl === start ? start + 1 : nl;
    } else if (altKey) {
      const chars = [...value.slice(start)];
      let i = 0;
      while (i < chars.length && !WORD.test(chars[i])) i++;
      while (i < chars.length && WORD.test(chars[i])) i++;
      to = start + chars.slice(0, Math.max(i, 1)).join("").length;
    } else {
      to = start + String.fromCodePoint(value.codePointAt(start)).length;
    }
    return { from: start, to };
  }

  const api = { isFunctionKeyText, forwardDeleteRange };
  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(global, api);
})(typeof window !== "undefined" ? window : globalThis);
