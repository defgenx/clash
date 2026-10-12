// node --test gui/tests/ — zero-dependency tests for workspace session
// ownership across conversation renames.
//
// Claude conversation ids move: `/clear` re-keys the registry via the status
// hook, and `claude --resume` forks into a new transcript that
// `heal_registry_forks` folds into the registry at startup. If a workspace's
// ownership list doesn't follow, the stale id is pruned as dead and the current
// id shows up under UNASSIGNED — a session the user never launched, reappearing
// on every relaunch. These are the two guards against that.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const {
  persistedSessionIds,
  remapWorkspaceIds,
  ownershipTransfers,
  pruneOwnership,
  moveTabToWorkspace,
} = require("../dist/ws-ownership.js");

// Mirrors app.js's isRealSessionId.
const isSessionId = (id) =>
  !!id &&
  !id.startsWith("browser-") &&
  !id.startsWith("shellterm-") &&
  !id.startsWith("view:");

test("collects ids from panes AND ownership, deduped", () => {
  const wss = [
    { panes: ["a", null, "b"], sessions: ["a", "c"] },
    { panes: ["d"], sessions: ["d", "e"] },
  ];
  assert.deepEqual(persistedSessionIds(wss, isSessionId).sort(), [
    "a",
    "b",
    "c",
    "d",
    "e",
  ]);
});

test("skips synthetic pane keys and empty slots", () => {
  const wss = [
    {
      panes: [null, "browser-1", "shellterm-7", "view:wfboard", "real"],
      sessions: ["real", "shellterm-7"],
    },
  ];
  assert.deepEqual(persistedSessionIds(wss, isSessionId), ["real"]);
});

test("an owned-but-closed session is still collected", () => {
  // The regression: ownership-only ids were left out, so a session with no open
  // pane never got resolved forward.
  const wss = [{ panes: [null], sessions: ["owned-not-open"] }];
  assert.deepEqual(persistedSessionIds(wss, isSessionId), ["owned-not-open"]);
});

test("remap moves both panes and ownership", () => {
  const wss = [{ panes: ["old", null], sessions: ["old", "other"] }];
  assert.equal(remapWorkspaceIds(wss, new Map([["old", "new"]])), true);
  assert.deepEqual(wss[0].panes, ["new", null]);
  assert.deepEqual(wss[0].sessions, ["new", "other"]);
});

test("remap dedupes when two persisted ids collapse onto one session", () => {
  // Pre-fork id and current id both owned — the rename must not leave a dupe.
  const wss = [{ panes: ["pre"], sessions: ["pre", "cur"] }];
  remapWorkspaceIds(wss, new Map([["pre", "cur"]]));
  assert.deepEqual(wss[0].sessions, ["cur"]);
  assert.deepEqual(wss[0].panes, ["cur"]);
});

test("remap reports no change for an empty or identity map", () => {
  const wss = [{ panes: ["a"], sessions: ["a"] }];
  assert.equal(remapWorkspaceIds(wss, new Map()), false);
  assert.equal(remapWorkspaceIds(wss, null), false);
  assert.deepEqual(wss[0].sessions, ["a"]);
});

test("transfer accepts a rename that lands on a listed, unowned session", () => {
  const moved = ownershipTransfers(["old"], ["new"], new Set(["new"]), ["old"]);
  assert.deepEqual([...moved], [["old", "new"]]);
});

test("transfer refuses a target missing from the session list", () => {
  // Genuinely dead: the registry still knows a lineage but nothing is listed.
  const moved = ownershipTransfers(["old"], ["new"], new Set(), ["old"]);
  assert.equal(moved.size, 0);
});

test("transfer refuses a target another workspace already owns", () => {
  const moved = ownershipTransfers(["old"], ["new"], new Set(["new"]), ["new"]);
  assert.equal(moved.size, 0);
});

test("transfer never hands the same target to two vanished ids", () => {
  const moved = ownershipTransfers(
    ["oldA", "oldB"],
    ["same", "same"],
    new Set(["same"]),
    []
  );
  assert.deepEqual([...moved], [["oldA", "same"]]);
});

test("transfer passes through unresolved ids untouched", () => {
  // resolve_session_ids echoes unknown ids back unchanged.
  const moved = ownershipTransfers(["ghost"], ["ghost"], new Set(["ghost"]), []);
  assert.equal(moved.size, 0);
});

test("transfer tolerates a failed resolve (null)", () => {
  const moved = ownershipTransfers(["old"], null, new Set(["new"]), []);
  assert.equal(moved.size, 0);
});

test("prune transfers renamed sessions and drops dead ones", () => {
  const wss = [{ panes: [null], sessions: ["renamed", "dead", "alive"] }];
  const changed = pruneOwnership(
    wss,
    new Set(["renamed", "dead"]),
    new Map([["renamed", "current"]])
  );
  assert.equal(changed, true);
  assert.deepEqual(wss[0].sessions, ["current", "alive"]);
});

test("prune leaves untouched workspaces alone", () => {
  const wss = [{ panes: [null], sessions: ["a"] }, { panes: [null], sessions: ["b"] }];
  assert.equal(pruneOwnership(wss, new Set(["zzz"]), new Map()), false);
  assert.deepEqual(wss[0].sessions, ["a"]);
  assert.deepEqual(wss[1].sessions, ["b"]);
});

test("prune keeps ownership in the workspace that held the old id", () => {
  const wss = [
    { panes: [null], sessions: ["old"] },
    { panes: [null], sessions: ["untouched"] },
  ];
  pruneOwnership(wss, new Set(["old"]), new Map([["old", "new"]]));
  assert.deepEqual(wss[0].sessions, ["new"]);
  assert.deepEqual(wss[1].sessions, ["untouched"]);
});

test("full reopen scenario: resumed session stays in its workspace", () => {
  // Session "pre" was owned but closed; a resume forked it to "post", which the
  // backend healed into the registry. On reopen list_sessions reports "post".
  const wss = [{ name: "main", panes: ["open"], sessions: ["open", "pre"] }];
  const saved = persistedSessionIds(wss, isSessionId);
  const resolved = saved.map((id) => (id === "pre" ? "post" : id));
  const remap = new Map();
  saved.forEach((id, i) => {
    if (resolved[i] !== id) remap.set(id, resolved[i]);
  });
  remapWorkspaceIds(wss, remap);
  // "post" is owned by main, so it never renders under UNASSIGNED.
  assert.deepEqual(wss[0].sessions, ["open", "post"]);
  assert.equal(wss[0].sessions.includes("pre"), false);
});

test("transfer follows a live row back to its older PTY id", () => {
  // Ownership moved old → new on a one-tick flip; the row is listed as `old`
  // again. `new` resolves to itself and `old` resolves to `new`: same session.
  const moved = ownershipTransfers(
    ["new"],
    ["new"],
    new Set(["old"]),
    ["new"],
    ["old"],
    ["new"]
  );
  assert.deepEqual([...moved], [["new", "old"]]);
});

test("transfer never claims an unowned row of a different session", () => {
  const moved = ownershipTransfers(
    ["mine"],
    ["mine"],
    new Set(["other"]),
    ["mine"],
    ["other"],
    ["other"]
  );
  assert.equal(moved.size, 0);
});

// ── moveTabToWorkspace: the user's own reassignment of a tab ──

test("moving a tab transfers ownership and empties its old pane", () => {
  const wss = [
    { panes: ["a", "b"], focused: 0, sessions: ["a", "b"] },
    { panes: [null], focused: 0, sessions: [] },
  ];
  assert.equal(moveTabToWorkspace(wss, "a", 1), true);
  assert.deepEqual(wss[0], { panes: [null, "b"], focused: 0, sessions: ["b"] });
  assert.deepEqual(wss[1], { panes: ["a"], focused: 0, sessions: ["a"] });
});

test("an unowned tab (browser, view) becomes owned by the target only", () => {
  const wss = [
    { panes: ["view:inbox"], focused: 0, sessions: [] },
    { panes: [null], focused: 0, sessions: [] },
  ];
  moveTabToWorkspace(wss, "view:inbox", 1);
  assert.deepEqual(wss[0].panes, [null]);
  assert.deepEqual(wss[1].sessions, ["view:inbox"]);
});

test("the target fills its focused pane, else the first empty, else replaces focus", () => {
  const empty = [{ panes: [], sessions: [] }, { panes: ["x", null, null], focused: 2, sessions: [] }];
  moveTabToWorkspace(empty, "a", 1);
  assert.deepEqual(empty[1].panes, ["x", null, "a"]);
  assert.equal(empty[1].focused, 2);

  const firstEmpty = [{ panes: [], sessions: [] }, { panes: ["x", null], focused: 0, sessions: [] }];
  moveTabToWorkspace(firstEmpty, "a", 1);
  assert.deepEqual(firstEmpty[1].panes, ["x", "a"]);
  assert.equal(firstEmpty[1].focused, 1);

  const full = [{ panes: [], sessions: [] }, { panes: ["x", "y"], focused: 1, sessions: [] }];
  moveTabToWorkspace(full, "a", 1);
  assert.deepEqual(full[1].panes, ["x", "a"]);
});

test("a tab already shown in the target only gains focus", () => {
  const wss = [{ panes: ["a"], focused: 0, sessions: [] }, { panes: ["a", "b"], focused: 1, sessions: [] }];
  moveTabToWorkspace(wss, "a", 1);
  assert.deepEqual(wss[1].panes, ["a", "b"]);
  assert.equal(wss[1].focused, 0);
  assert.deepEqual(wss[1].sessions, ["a"]);
});

test("follows() carries a session's own view tabs out of other panes", () => {
  const wss = [
    { panes: ["a", "view:conv:a", "view:conv:b"], focused: 0, sessions: ["a"] },
    { panes: [null], focused: 0, sessions: [] },
  ];
  moveTabToWorkspace(wss, "a", 1, { follows: (p) => p === "view:conv:a" });
  assert.deepEqual(wss[0].panes, [null, null, "view:conv:b"]);
});

test("place: false only reassigns ownership (a closed session)", () => {
  const wss = [
    { panes: ["x"], focused: 0, sessions: ["a", "x"] },
    { panes: ["y"], focused: 0, sessions: ["y"] },
  ];
  moveTabToWorkspace(wss, "a", 1, { place: false });
  assert.deepEqual(wss[0].sessions, ["x"]);
  assert.deepEqual(wss[1], { panes: ["y"], focused: 0, sessions: ["y", "a"] });
});

test("an unknown target workspace changes nothing", () => {
  const wss = [{ panes: ["a"], focused: 0, sessions: ["a"] }];
  assert.equal(moveTabToWorkspace(wss, "a", 3), false);
  assert.deepEqual(wss[0], { panes: ["a"], focused: 0, sessions: ["a"] });
});
