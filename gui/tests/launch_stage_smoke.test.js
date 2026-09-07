// node --test gui/tests/ — the launch status line, run for real.
//
// `launchStageLine` is the whole user-visible half of the launch loader, and a
// regex over the source proves only that a `case` exists. This evaluates the
// function against the payloads the backend actually sends, so a template
// literal reading a field the payload does not carry shows up as "undefined"
// here instead of on the status line during a 44-second checkout.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const { extractFunction } = require("./extract-fn.js");

const APP = fs.readFileSync(path.join(__dirname, "..", "dist", "app.js"), "utf8");
// eslint-disable-next-line no-new-func
const launchStageLine = new Function(
  `${extractFunction(APP, "launchStageLine")}; return launchStageLine;`
)();

test("every step of a launch reads as a sentence about that step", () => {
  const lines = {
    read: launchStageLine({ key: "p/s", stage: "read" }),
    branch: launchStageLine({ key: "p/s", stage: "branch", branch: "user-consent-revocation" }),
    worktree: launchStageLine({ key: "p/s", stage: "worktree", branch: "user-consent-revocation" }),
    checkout: launchStageLine({
      key: "p/s",
      stage: "checkout",
      branch: "user-consent-revocation",
      percent: 47,
      files: 13176,
      total: 28034,
    }),
    record: launchStageLine({ key: "p/s", stage: "record" }),
    spawn: launchStageLine({ key: "p/s", stage: "spawn" }),
  };

  for (const [stage, line] of Object.entries(lines)) {
    assert.ok(line && line.length > 4, `stage ${stage} produced "${line}"`);
    // The failure this catches: a field the payload doesn't carry, rendered.
    assert.doesNotMatch(line, /undefined|NaN|\[object/, `stage ${stage}: "${line}"`);
  }

  // The steps that know a branch name say it; the ones that don't must not
  // leave the empty placeholder behind as a dangling pair of quotes.
  assert.match(lines.branch, /user-consent-revocation/);
  assert.match(lines.worktree, /user-consent-revocation/);
  assert.doesNotMatch(lines.record, /“”|""/);
  assert.doesNotMatch(lines.spawn, /“”|""/);

  // The long step carries git's counters AND why it is about to sit there —
  // a bare percentage reads as a stall rather than as a copy in progress.
  assert.match(lines.checkout, /47%/);
  assert.match(lines.checkout, /13176\/28034/);
  assert.match(lines.worktree, /checks out the whole repo/);

  // An unknown step still says something: a stage added on the backend with
  // no wording here degrades to a generic line, never to a blank one.
  const unknown = launchStageLine({ key: "p/s", stage: "fetching" });
  assert.ok(unknown && !/undefined/.test(unknown), `unknown stage gave "${unknown}"`);
});
