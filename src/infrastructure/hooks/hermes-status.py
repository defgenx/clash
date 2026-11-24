"""clash status plugin for Hermes Agent.

Installed by clash into <hermes home>/plugins/clash-status/; Hermes loads it
once it is listed in plugins.enabled (`hermes plugins enable clash-status`).
The Hermes counterpart of status-hook.sh: it writes the same status files.
Contract: clash's docs/hooks.md ("Hermes sessions").

Hooks run in Hermes's gateway process, not in the session's own, so nothing
clash put in the session's environment is visible here: the session is named
by the title clash gave it (`clash:<clash id>`), read from Hermes's own store.
"""

import json
import os
import sqlite3

DATA_DIR = "{DATA_DIR}"
TITLE_PREFIX = "clash:"

# Hermes id -> the id clash lists the conversation under (None: not a
# terminal session, so not clash's business). A root's title and source do
# not change, so the answer is stable.
_ids = {}


def _home():
    try:
        from hermes_constants import get_hermes_home

        return str(get_hermes_home())
    except Exception:
        return os.environ.get("HERMES_HOME") or os.path.expanduser("~/.hermes")


def _row_id(session_id):
    """Walk compression continuations back to the conversation's root."""
    if session_id in _ids:
        return _ids[session_id]
    db = os.path.join(_home(), "state.db")
    try:
        conn = sqlite3.connect(f"file:{db}?mode=ro", uri=True, timeout=0.5)
    except Exception:
        return None
    try:
        sid = session_id
        for _ in range(256):
            row = conn.execute(
                "SELECT s.title, s.source, s.parent_session_id, p.end_reason "
                "FROM sessions s LEFT JOIN sessions p ON p.id = s.parent_session_id "
                "WHERE s.id = ?",
                (sid,),
            ).fetchone()
            if row is None:
                return None
            title, source, parent, parent_end = row
            if parent and parent_end == "compression":
                sid = parent
                continue
            if title and title.startswith(TITLE_PREFIX) and len(title) > len(TITLE_PREFIX):
                result = title[len(TITLE_PREFIX):]
            elif source == "cli":
                result = sid
            else:
                result = None
            _ids[session_id] = result
            return result
    except Exception:
        return None
    finally:
        conn.close()
    return None


def _write(session_id, status):
    if not session_id:
        return
    row_id = _row_id(session_id)
    if not row_id or "/" in row_id:
        return
    path = os.path.join(DATA_DIR, "status", row_id)
    tmp = f"{path}.tmp.{os.getpid()}"
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(tmp, "w") as f:
            json.dump({"status": status, "session_id": row_id}, f)
        os.replace(tmp, path)
    except Exception:
        # Best effort: a missed status costs a stale row, never the session.
        pass


def _on(status):
    def callback(session_id="", **_kwargs):
        _write(session_id, status)

    return callback


def register(ctx):
    ctx.register_hook("on_session_open", _on("waiting"))
    ctx.register_hook("pre_llm_call", _on("thinking"))
    # Fires at the end of every turn (one run per message), not of the session.
    ctx.register_hook("on_session_end", _on("waiting"))
    ctx.register_hook("pre_approval_request", _on("prompting"))
    ctx.register_hook("post_approval_response", _on("thinking"))
    # A clarify/sudo/secret prompt blocks on the human like an approval does.
    ctx.register_hook("on_human_input_request", _on("prompting"))
    ctx.register_hook("on_human_input_resolved", _on("thinking"))
    ctx.register_hook("on_session_finalize", _on("idle"))
