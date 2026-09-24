// clash status extension — loaded by `omp -e <this file>` when clash spawns omp.
// The OMP counterpart of status-hook.sh: same status files, same registry
// re-key on a conversation switch. Contract: docs/hooks.md ("OMP sessions").
import * as fs from "node:fs";
import * as path from "node:path";

const DATA_DIR = "{DATA_DIR}";

// A session's clash id is its file's uuid suffix (`<timestamp>_<uuid>.jsonl`),
// not the header id: clash names the file of every session it spawns.
function idFromFile(file) {
  if (!file) return "";
  const stem = path.basename(file).replace(/\.jsonl$/, "");
  const i = stem.indexOf("_");
  return i >= 0 ? stem.slice(i + 1) : stem;
}

function currentId(ctx) {
  const sm = ctx && ctx.sessionManager;
  if (!sm) return "";
  return idFromFile(sm.getSessionFile && sm.getSessionFile()) || (sm.getSessionId && sm.getSessionId()) || "";
}

function writeAtomic(file, content) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const tmp = `${file}.tmp.${process.pid}.${Date.now()}`;
  fs.writeFileSync(tmp, content);
  fs.renameSync(tmp, file);
}

function writeStatus(sid, status) {
  if (!sid) return;
  try {
    writeAtomic(path.join(DATA_DIR, "status", sid), JSON.stringify({ status, session_id: sid }));
  } catch {
    // Best effort: a missed status costs a stale row, never the session.
  }
}

// `/new` or `/resume` inside omp moved this process to another conversation.
// Re-key the registry entry that answered to the old id, exactly as the
// Claude hook does on `/clear`, so the row follows the process.
function rekey(prevId, newId) {
  if (!prevId || !newId || prevId === newId) return;
  const reg = path.join(DATA_DIR, "sessions.json");
  let data;
  try {
    data = JSON.parse(fs.readFileSync(reg, "utf8"));
  } catch {
    return;
  }
  const key = Object.keys(data).find(k => {
    const v = data[k] || {};
    return k === prevId || v.claude_session_id === prevId || (v.previous_ids || []).includes(prevId);
  });
  if (!key || key === newId) return;
  const entry = { ...data[key] };
  const prev = [...(entry.previous_ids || [])];
  if (!prev.includes(key)) prev.push(key);
  entry.previous_ids = prev;
  entry.claude_session_id = newId;
  entry.session_id = newId;
  delete data[key];
  data[newId] = entry;
  try {
    writeAtomic(reg, JSON.stringify(data, null, 2));
  } catch {}
  try {
    const name = fs.readFileSync(path.join(DATA_DIR, "names", prevId), "utf8").trim();
    if (name) {
      const m = name.match(/^(.*)-(\d+)$/);
      const next = m ? `${m[1]}-${Number(m[2]) + 1}` : `${name}-2`;
      writeAtomic(path.join(DATA_DIR, "names", newId), next);
    }
  } catch {}
}

export default function clashStatus(pi) {
  pi.on("session_start", (_e, ctx) => writeStatus(currentId(ctx), "waiting"));
  pi.on("agent_start", (_e, ctx) => writeStatus(currentId(ctx), "thinking"));
  pi.on("agent_end", (e, ctx) => {
    if (!e || !e.willContinue) writeStatus(currentId(ctx), "waiting");
  });
  pi.on("tool_approval_requested", (_e, ctx) => writeStatus(currentId(ctx), "prompting"));
  pi.on("tool_approval_resolved", (_e, ctx) => writeStatus(currentId(ctx), "thinking"));
  // The `ask` tool blocks on the human exactly like an approval does.
  pi.on("tool_call", (e, ctx) => {
    if (e && e.toolName === "ask") writeStatus(currentId(ctx), "prompting");
  });
  pi.on("tool_result", (e, ctx) => {
    if (e && e.toolName === "ask") writeStatus(currentId(ctx), "thinking");
  });
  pi.on("session_switch", (e, ctx) => {
    const prevId = idFromFile(e && e.previousSessionFile);
    const newId = currentId(ctx);
    rekey(prevId, newId);
    writeStatus(newId, "waiting");
  });
  pi.on("session_shutdown", (_e, ctx) => writeStatus(currentId(ctx), "idle"));
}
