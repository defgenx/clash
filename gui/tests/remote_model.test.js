// node --test gui/tests/ — clash instances: the Machines view's and the phone
// page's pure model (gui/dist/remote-model.js).
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const vm = require("node:vm");
const m = require("../dist/remote-model.js");

const TOKEN = "ab".repeat(32);
const status = (extra = {}) => ({
  app: "clash",
  hostname: "studio",
  version: "1.2.3",
  pid: 1,
  startedAt: 0,
  now: 3 * 3600e3 + 5 * 60e3,
  sessionsRunning: 2,
  needsYou: 4,
  ...extra,
});

test("a pairing link yields the origin and token, nothing else", () => {
  assert.equal(m.pairingToken(`#token=${TOKEN}`), TOKEN);
  assert.equal(m.pairingToken(`#x=1&token=${TOKEN.toUpperCase()}`), TOKEN);
  assert.equal(m.pairingToken("#token=abc"), null);
  assert.deepEqual(m.parsePairingLink(` http://100.64.1.2:7878/remote/#token=${TOKEN} `), {
    url: "http://100.64.1.2:7878",
    token: TOKEN,
  });
  assert.equal(m.parsePairingLink(`ftp://h/#token=${TOKEN}`), null);
  assert.equal(m.parsePairingLink("not a url"), null);
});

test("instances: local first and never stored, remote one per origin", () => {
  let list = [m.localInstance()];
  list = m.addInstance(list, { url: "http://b:1", token: "t1" });
  list = m.addInstance(list, { url: "http://a:1", token: "t2", name: "Alpha" });
  list = m.addInstance(list, { url: "http://b:1", token: "t3" });
  assert.equal(list.length, 3);
  assert.equal(list.find((i) => i.url === "http://b:1").token, "t3");
  assert.deepEqual(
    m.orderInstances(list).map((i) => i.kind + ":" + i.name),
    ["local:This machine", "remote:Alpha", "remote:b"]
  );
  assert.equal(m.instanceKey(list[0]), "local");
  assert.equal(m.instanceKey(list.find((i) => i.name === "b")), "http://b:1");

  const mem = new Map();
  const storage = { getItem: (k) => mem.get(k) || null, setItem: (k, v) => mem.set(k, v) };
  assert.ok(m.saveInstances(storage, list));
  const back = m.loadInstances(storage);
  assert.equal(back.length, 2, "the local instance is not persisted");
  assert.ok(back.every((i) => i.kind === "remote"));
  assert.deepEqual(m.removeInstance(list, "http://a:1").map((i) => i.url).sort(), ["", "http://b:1"]);
  // Storage that throws (private mode) is an empty list, never a crash.
  const broken = { getItem: () => { throw new Error("denied"); }, setItem: () => { throw new Error("denied"); } };
  assert.deepEqual(m.loadInstances(broken), []);
  assert.equal(m.saveInstances(broken, list), false);
});

test("connected, reconnecting, unreachable — from the polls alone", () => {
  const inst = { kind: "remote", name: "b", url: "http://b:1", token: TOKEN };
  const t0 = 1_000_000;
  let p = m.recordPoll(null, { ok: true, status: status() }, t0);
  let v = m.instanceView(inst, p, t0);
  assert.equal(v.state, "connected");
  assert.equal(v.name, "studio");
  assert.deepEqual(v.facts, ["v1.2.3", "up 3 h 5 min", "2 sessions running"]);
  assert.equal(v.needsYou, 4);
  assert.deepEqual(m.connectionIndicator(v), { state: "connected", text: "Connected to studio" });

  p = m.recordPoll(p, { error: "no answer" }, t0 + 5000);
  v = m.instanceView(inst, p, t0 + 5000);
  assert.equal(v.state, "reconnecting");
  assert.equal(m.connectionIndicator(v).state, "reconnecting");

  for (let i = 0; i < m.REMOTE_LOST_AFTER; i++) p = m.recordPoll(p, { error: "no answer" }, t0 + 120e3);
  v = m.instanceView(inst, p, t0 + 180e3);
  assert.equal(v.state, "unreachable");
  assert.equal(v.detail, "unreachable — last seen 3 min ago");
  assert.equal(v.needsYou, null, "a stale count is not shown as current");
  assert.equal(m.connectionIndicator(v).state, "lost");

  // A good answer clears everything.
  p = m.recordPoll(p, { ok: true, status: status() }, t0 + 200e3);
  assert.equal(m.instanceView(inst, p, t0 + 200e3).state, "connected");

  // Never answered; refused token; something that is not clash.
  assert.equal(m.instanceView(inst, m.recordPoll(null, { error: "x" }, 1), 1).detail, "unreachable — never answered");
  assert.equal(m.instanceView(inst, m.recordPoll(null, { unauthorized: true }, 1), 1).state, "unpaired");
  assert.equal(m.instanceView(inst, m.recordPoll(null, { ok: true, status: { app: "nginx" } }, 1), 1).state, "unreachable");
  assert.equal(m.instanceView(inst, null, 1).state, "checking");
  assert.deepEqual(m.connectionIndicator(null), { state: "none", text: "No instance paired" });
});

test("the local instance reads as this clash", () => {
  const v = m.instanceView(m.localInstance(), m.recordPoll(null, { ok: true, status: status() }, 1), 1);
  assert.equal(v.kind, "local");
  assert.equal(v.detail, "this clash");
  assert.equal(m.connectionIndicator(v).text, "studio · this clash");
});

test("the browser branch publishes every name the GUI and the phone page use", () => {
  const dist = path.join(__dirname, "..", "dist");
  const win = {};
  vm.runInNewContext(fs.readFileSync(path.join(dist, "remote-model.js"), "utf8"), { window: win });
  const app = fs.readFileSync(path.join(dist, "app.js"), "utf8");
  const page = fs.readFileSync(path.join(dist, "remote", "remote.js"), "utf8");
  for (const name of [
    "REMOTE_POLL_MS",
    "parsePairingLink",
    "localInstance",
    "instanceKey",
    "orderInstances",
    "recordPoll",
    "instanceView",
  ]) {
    assert.ok(name in win, `${name} must be on window`);
    assert.ok(app.includes(name), `app.js is expected to use ${name}`);
  }
  for (const name of ["pairingToken", "addInstance", "removeInstance", "loadInstances", "saveInstances", "recordPoll", "instanceView", "connectionIndicator", "remoteAgo"]) {
    assert.ok(name in win, `${name} must be on window`);
    assert.ok(page.includes(name), `remote.js is expected to use ${name}`);
  }
  const html = fs.readFileSync(path.join(dist, "index.html"), "utf8");
  assert.ok(html.indexOf("remote-model.js") > 0 && html.indexOf("remote-model.js") < html.indexOf('src="app.js"'));
  const phone = fs.readFileSync(path.join(dist, "remote", "index.html"), "utf8");
  for (const s of ["attention.js", "remote-model.js", "remote.js"]) assert.ok(phone.includes(`src="${s}"`), s);
  assert.ok(phone.indexOf("attention.js") < phone.indexOf('src="remote.js"'));
});

test("the phone page never puts the token in a URL", () => {
  const page = fs.readFileSync(path.join(__dirname, "..", "dist", "remote", "remote.js"), "utf8");
  assert.match(page, /Authorization: `Bearer \$\{machine\.token\}`/);
  assert.doesNotMatch(page, /[?&]token=/);
  // The pairing fragment is dropped from the address bar once read.
  assert.match(page, /history\.replaceState\(null, "", location\.pathname\)/);
});
