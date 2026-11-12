// Waves — the pure half of a multi-wave item's UI.
//
// A project-sized item is planned as waves (`waves.json`): slices approved one
// by one, in dependency order, inside the one item. The backend computes the
// board (`item.waves`: each wave's state, level and the waves it may run
// alongside — `application::waves`); this module turns it into what the bar,
// the Waves tab and the wave picker show, so none of those decides anything on
// its own. Contract: docs/workflows.md → Waves.
(function () {
  "use strict";

  /// The item's usable board, or null (no waves.json, or one with an issue —
  /// `wavesIssue` says which).
  function waveBoard(item) {
    const b = item && item.waves;
    return b && !b.issue && Array.isArray(b.waves) && b.waves.length ? b : null;
  }

  function wavesIssue(item) {
    return (item && item.waves && item.waves.issue) || null;
  }

  const ids = (ws) => ws.map((w) => w.id);
  const of = (board, state) => board.waves.filter((w) => w.state === state);
  const settledPr = (p) => p.state === "MERGED" || p.state === "CLOSED";

  /// What a person reads for a wave id: `w2` → "Wave 2". The id itself (the
  /// planner's `waves.json` key, clash's record) never reaches the screen; an
  /// id that is not `w<n>` is shown as written.
  function waveName(w) {
    const id = typeof w === "string" ? w : (w && w.id) || "";
    const m = /^w(\d+)$/i.exec(id);
    return m ? `Wave ${m[1]}` : id;
  }

  /// "Wave 2 · API".
  function waveLabel(w) {
    return w.title ? `${waveName(w)} · ${w.title}` : waveName(w);
  }

  /// "Wave 2", "Waves 2 and 3", "Waves 2, 3 and 5".
  function waveList(list) {
    const ids = list.map((w) => (typeof w === "string" ? w : w.id));
    const nums = ids.map((id) => /^w(\d+)$/i.exec(id));
    const and = (xs) => (xs.length > 1 ? `${xs.slice(0, -1).join(", ")} and ${xs[xs.length - 1]}` : xs[0] || "");
    if (ids.length > 1 && nums.every(Boolean)) return `Waves ${and(nums.map((m) => m[1]))}`;
    return and(ids.map(waveName));
  }

  /// Where the item's waves stand, in one place every surface reads.
  function waveSummary(item) {
    const board = waveBoard(item);
    if (!board) return null;
    const done = of(board, "done");
    const active = of(board, "active");
    const review = of(board, "review");
    const ready = of(board, "ready");
    const total = board.waves.length;
    // After approving the round in review, does anything remain?
    const left = board.waves.filter((w) => w.state !== "done" && w.state !== "review");
    let text = `Waves ${done.length}/${total}`;
    if (active.length) text += ` · ${waveList(active)} in progress`;
    else if (review.length) text += ` · ${waveList(review)} to sign off`;
    else if (ready.length && done.length < total) text += ` · ${waveList(ready)} ready`;
    if (done.length === total) text += " · all signed off";
    return {
      total,
      done: ids(done),
      active: ids(active),
      review: ids(review),
      ready: ids(ready),
      remainingAfterReview: ids(left),
      allDone: done.length === total,
      text,
    };
  }

  /// What a parked item's bar offers for its waves:
  /// - `approve` — the round's waves wait for approval and others remain;
  /// - `approve-last` — they are the last ones (the item's own approvals take
  ///   over, approving them on the way);
  /// - `start` — nothing waits, and some waves are ready;
  /// - `blocked` — nothing ready and nothing to approve (should not happen
  ///   with a valid plan; said rather than hidden);
  /// - null — no waves, or all approved.
  function waveBarState(item) {
    const s = waveSummary(item);
    if (!s || s.allDone) return null;
    if (s.review.length) return { kind: s.remainingAfterReview.length ? "approve" : "approve-last", ...s };
    if (s.active.length) return null; // an agent is on it; nothing to decide
    if (s.ready.length) return { kind: "start", ...s };
    return { kind: "blocked", ...s };
  }

  /// Do the item's final approvals (done, PR stage) wait on waves? An
  /// unreadable waves.json holds them too: the backend still counts its
  /// waves as open, so the item would never close.
  function wavesHoldApproval(item) {
    if (wavesIssue(item)) return true;
    const st = waveBarState(item);
    return !!st && st.kind !== "approve-last";
  }

  const CODE_STAGES = ["diff-review", "pr-draft", "pr-ready"];

  /// What the bar says about an unreadable waves.json, by stage: at plan
  /// review a change request repairs it; past it, a discussion or a move
  /// back does. Null when the file is fine (or absent).
  function wavesIssueGuidance(item) {
    const issue = wavesIssue(item);
    if (!issue) return null;
    const st = (item.meta && item.meta.status) || "";
    if (st === "plan-review")
      return `waves.json can't be read: ${issue} — request changes on the plan so the planner repairs it.`;
    return `waves.json can't be read: ${issue} — ask 💬 Discuss to repair it, or ↩ Move to… plan review.`;
  }

  /// The picker's rows: every ready wave, each disabled while it cannot run
  /// with the ones ticked (a dependency between them, overlapping areas, or
  /// areas nobody declared). Pure over the board and the current pick.
  function wavePickRows(item, picked) {
    const board = waveBoard(item);
    if (!board) return [];
    const ready = of(board, "ready");
    const chosen = new Set(picked || []);
    return ready.map((w) => {
      const clash = [...chosen].filter((id) => id !== w.id && !(w.compatibleWith || []).includes(id));
      const checked = chosen.has(w.id);
      const together = (w.compatibleWith || []).filter((id) => ready.some((r) => r.id === id));
      return {
        id: w.id,
        label: waveLabel(w),
        detail:
          (w.summary ? `${w.summary} ` : "") +
          (w.areas && w.areas.length ? `Touches ${w.areas.join(", ")}.` : "Declares no areas, so it runs alone."),
        checked,
        disabled: !checked && clash.length > 0,
        reason: !checked && clash.length ? `Can't run with ${waveList(clash)}` : "",
        together,
      };
    });
  }

  /// The default pick: the first ready wave, in plan order — waves go in
  /// order unless you choose to run compatible ones together.
  function defaultWavePick(item) {
    const board = waveBoard(item);
    if (!board) return [];
    const first = of(board, "ready")[0];
    return first ? [first.id] : [];
  }

  /// Toggle `id` in a pick, refusing a wave that cannot join the others.
  function toggleWavePick(item, picked, id) {
    const cur = new Set(picked || []);
    if (cur.has(id)) {
      cur.delete(id);
    } else {
      const row = wavePickRows(item, [...cur]).find((r) => r.id === id);
      if (!row || row.disabled) return [...cur];
      cur.add(id);
    }
    const order = (waveBoard(item) || { waves: [] }).waves.map((w) => w.id);
    return order.filter((x) => cur.has(x));
  }

  /// The roadmap's steps: waves grouped by dependency level, so a step holds
  /// the waves that become startable together once the previous step lands.
  /// `parallel` names the pairs of that step which may share a round.
  function waveColumns(item) {
    const board = waveBoard(item);
    if (!board) return [];
    const cols = [];
    for (const w of board.waves) {
      const l = w.level || 0;
      (cols[l] = cols[l] || []).push(w);
    }
    return cols.filter(Boolean).map((waves, i) => {
      const together = waves.filter((w) =>
        waves.some((o) => o.id !== w.id && (w.compatibleWith || []).includes(o.id))
      );
      return {
        title: `Step ${i + 1}`,
        detail:
          waves.length < 2
            ? ""
            : together.length === waves.length
              ? "These can run in the same round."
              : together.length
                ? `${waveList(together)} can run in the same round; the others one at a time.`
                : "One at a time: they overlap or depend on each other.",
        waves,
      };
    });
  }

  /// A wave's PRs, as `itemPrs` records (`prs` is the item's list), with
  /// their counts — what "how far along is this wave" is measured in.
  function wavePrs(w, prs) {
    const mine = (prs || []).filter((p) => (w.prs || []).includes(p.url));
    const n = (f) => mine.filter(f).length;
    return {
      prs: mine,
      total: mine.length,
      merged: n((p) => p.state === "MERGED"),
      closed: n((p) => p.state === "CLOSED"),
      open: n((p) => p.state !== "MERGED" && p.state !== "CLOSED"),
    };
  }

  /// The waves that become ready once `ids` are approved: waiting waves
  /// whose every missing dependency (`blockedBy`, from the backend) is one of
  /// them.
  function unlockedBy(board, ids) {
    return board.waves.filter(
      (w) =>
        w.state === "blocked" &&
        (w.blockedBy || []).length > 0 &&
        w.blockedBy.every((d) => ids.includes(d))
    );
  }

  /// Where the project stands, in sentences: what is happening now, and what
  /// you do next. `prs` is `itemPrs(item.meta)`. The roadmap opens on it, so
  /// nobody has to work out the state from badges. It follows the item's
  /// stage: a sign-off is only ever suggested where the bar offers one (a
  /// parked code stage).
  function waveNarrative(item, prs) {
    const status = (item && item.meta && item.meta.status) || "";
    const closed = status === "done" || status === "abandoned";
    const closedNext =
      status === "done" ? "Nothing — the item is done." : "Nothing — the item is abandoned; ↩ Reopen brings it back.";
    const issue = wavesIssue(item);
    if (issue)
      return {
        now: `waves.json can't be used: ${issue}.`,
        next: closed
          ? closedNext
          : status === "plan-review"
            ? "Request changes on the plan so the planner fixes it. Nothing starts until then."
            : CODE_STAGES.includes(status)
              ? "Ask 💬 Discuss to repair it, or ↩ Move to… plan review and request changes there. The item can't be approved until then."
              : "When the agent hands back, check that waves.json was repaired.",
      };
    const board = waveBoard(item);
    if (!board) return null;
    const s = waveSummary(item);
    const named = (ids) => ids.map((id) => waveLabel(board.waves.find((w) => w.id === id))).join(" and ");
    const progress = s.allDone ? `All ${s.total} waves are signed off.` : `${s.done.length} of ${s.total} waves signed off.`;
    const ready = board.waves.filter((w) => w.state === "ready");
    const pairs = ready.filter((w) => ready.some((o) => o.id !== w.id && (w.compatibleWith || []).includes(o.id)));
    const together = pairs.length ? ` — or ${waveList(pairs)} together: they touch different code` : "";
    if (closed) return { now: progress, next: closedNext };
    if (status === "reviewing") {
      const back = (item.meta.review && (item.meta.review.returnStatus || item.meta.review.return_status)) || "";
      const signs = CODE_STAGES.includes(back) && s.review.length;
      return {
        now: s.review.length ? `A review round is running on ${named(s.review)}.` : `A review round is running. ${progress}`,
        next: signs
          ? "When it hands back, read its findings — apply them, or sign the wave off."
          : "When it hands back, read its findings.",
      };
    }
    if (status === "changes-requested") {
      const on = s.active.length ? s.active : s.review;
      return {
        now: `A fix round is queued${on.length ? ` for ${named(on)}` : ""}.`,
        next: "Launch it from the bar.",
      };
    }
    if (status === "plan-review") {
      if (s.allDone || !ready.length)
        return { now: progress, next: "Approve the plan to implement it, or request changes." };
      return {
        now: `${progress} Ready to start: ${waveList(ready)}.`,
        next: `Approve the plan and start ${waveName(ready[0])}${together} — or request changes if the plan is wrong.`,
      };
    }
    if (s.active.length)
      return {
        now: `The agent is implementing ${named(s.active)}.`,
        next: "When it hands back, review the round, then sign the wave off.",
      };
    if (!CODE_STAGES.includes(status))
      return {
        now: progress,
        next: status === "draft" || status === "planning" ? "Wait for the plan." : "Wait for the agent to hand back.",
      };
    if (s.allDone)
      return {
        now: progress,
        next: "Finish the item: approve it, or mark it done once its PRs are merged.",
      };
    if (s.review.length) {
      const waves = s.review.map((id) => board.waves.find((w) => w.id === id));
      // A PR can belong to several waves: counted once.
      const seen = new Map();
      for (const w of waves) for (const p of wavePrs(w, prs).prs) seen.set(p.url, p);
      const all = [...seen.values()];
      const p = {
        total: all.length,
        merged: all.filter((x) => x.state === "MERGED").length,
        open: all.filter((x) => !settledPr(x)).length,
      };
      const prBit = p.total
        ? ` ${p.merged} of ${p.total} PR${p.total === 1 ? "" : "s"} merged${p.open ? `, ${p.open} still open` : ""}.`
        : "";
      const unlocks = unlockedBy(board, s.review);
      const after = unlocks.length ? ` — that unlocks ${waveList(unlocks)}` : "";
      const last = !board.waves.some((w) => w.state !== "done" && !s.review.includes(w.id));
      const close = last ? " The item closes only once every wave is signed off." : "";
      return {
        now: `${named(s.review)} ${s.review.length > 1 ? "are" : "is"} waiting for your sign-off.${prBit}`,
        next:
          (p.open > 0
            ? `Review it, get its PRs merged (or decide they can land later), then sign it off${after}.`
            : `Sign it off${after}.`) + close,
      };
    }
    if (ready.length)
      return {
        now: `${progress} Ready to start: ${waveList(ready)}.`,
        next: `Start ${waveName(ready[0])}${together}.`,
      };
    return {
      now: "No wave is ready: each one waits for a wave that isn't signed off yet.",
      next: "Sign off the wave in review, or move the item back to plan review if the order is wrong.",
    };
  }

  /// A wave state's badge text.
  function waveStateLabel(state) {
    switch (state) {
      case "done":
        return "signed off";
      case "review":
        return "to sign off";
      case "active":
        return "in progress";
      case "ready":
        return "ready";
      default:
        return "waiting";
    }
  }

  /// `Session.last_modified` ("YYYY-MM-DD HH:MM", local) as epoch millis.
  function stampMs(v) {
    if (typeof v === "number") return Number.isFinite(v) ? v : 0;
    const m = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})/.exec(String(v || ""));
    if (!m) return 0;
    const t = new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5]).getTime();
    return Number.isFinite(t) ? t : 0;
  }

  function agoText(ms) {
    if (!(ms >= 0)) return "";
    const min = Math.floor(ms / 60000);
    if (min < 1) return "just now";
    if (min < 60) return `${min} min ago`;
    const h = Math.floor(min / 60);
    if (h < 24) return `${h} h ago`;
    return `${Math.floor(h / 24)} d ago`;
  }

  /// The roadmap's live panel: the waves of the round in flight and what the
  /// agent on them is doing, or null when no wave is running. Waves that run
  /// in parallel share ONE round — one agent, one worktree — so the panel is
  /// one agent's state over several waves, never one agent per wave.
  ///
  /// `sessions` is `state.sessions`; the item's agent is its recorded session,
  /// else a running session stamped with the item. `kind` drives the panel's
  /// colour: `attention` (it waits on you), `working`, `idle` (turn finished),
  /// `stopped` (resumable), `gone`, `queued` (a fix round not launched yet).
  function waveLive(item, sessions, now = Date.now()) {
    const board = waveBoard(item);
    if (!board) return null;
    const active = board.waves.filter((w) => w.state === "active");
    if (!active.length) return null;
    const meta = item.meta || {};
    const waves = active.map((w) => ({ id: w.id, label: waveLabel(w), start: (w.start || "").slice(0, 8) }));
    // A queued fix round has no agent yet: the old session is history, and
    // the round starts from the bar.
    if (meta.status === "changes-requested")
      return {
        kind: "queued",
        title: `A fix round is queued for ${waveList(active)}`,
        shared: "",
        line: "A fix round is queued — launch it from the bar.",
        activity: "",
        waves,
        agentId: "",
        agentName: "",
      };
    const key = `${item.project}/${item.slug}`;
    const mine = (sessions || []).filter((s) => s.workflow_item === key);
    const agent =
      mine.find((s) => s.id === meta.sessionId) || mine.find((s) => s.is_running) || null;
    const status = agent ? String(agent.status || "").toLowerCase() : "";
    let kind;
    let line;
    if (item.agentAlive === false && !item.agentResumable) {
      kind = "gone";
      line = "The agent's session is gone — relaunch it from the card above.";
    } else if (item.agentResumable || (agent && !agent.is_running)) {
      kind = "stopped";
      line = "The agent is stopped — resume it to carry on where it left off.";
    } else if (status === "prompting") {
      kind = "attention";
      line = "The agent is waiting for you — a question or a permission.";
    } else if (status === "waiting") {
      kind = "idle";
      line = "The agent finished its turn and is at its prompt.";
    } else if (status === "starting") {
      kind = "working";
      line = "The agent is starting.";
    } else {
      kind = "working";
      line = "The agent is working.";
    }
    const last = agent ? stampMs(agent.last_modified) : 0;
    const ids = active.map((w) => w.id);
    return {
      kind,
      title:
        active.length > 1
          ? `${waveList(ids)} running together`
          : `${waveLabel(active[0])} in progress`,
      shared:
        active.length > 1
          ? "One round, one agent, one worktree — these waves touch different code, so they are built side by side."
          : "",
      line,
      activity: last ? `last activity ${agoText(now - last)}` : "",
      waves,
      agentId: agent ? agent.id : meta.sessionId || "",
      agentName: agent ? agent.name || agent.id : "",
    };
  }

  /// What stands between a wave and its next step, as facts a card can act
  /// on. `hard` ones stop the wave (an unapproved dependency); the rest are
  /// what to settle *before* approving it (PRs not merged, threads waiting on
  /// you, PRs the last refresh could not reach) — approval does not require
  /// them, so they are said, never enforced. `prs` carry `unanswered` and
  /// `error` as `itemPrs` + the item's `prErrors` give them.
  function waveBlockers(item, w, prs) {
    const board = waveBoard(item);
    if (!board) return [];
    const byId = new Map(board.waves.map((x) => [x.id, x]));
    const out = [];
    if (w.state === "blocked") {
      for (const id of w.blockedBy || []) {
        const d = byId.get(id);
        out.push({
          kind: "wave",
          hard: true,
          wave: id,
          text: d ? `Waits for ${waveLabel(d)} — ${waveStateLabel(d.state)}` : `Waits for ${waveName(id)}`,
        });
      }
    }
    const mine = wavePrs(w, prs).prs;
    if (w.state === "review") {
      const open = mine.filter((p) => !settledPr(p));
      if (open.length) {
        const drafts = open.filter((p) => p.draft).length;
        out.push({
          kind: "prs",
          hard: false,
          urls: open.map((p) => p.url),
          text: `${open.length} PR${open.length === 1 ? "" : "s"} not merged yet${drafts ? ` (${drafts} draft${drafts === 1 ? "" : "s"})` : ""}`,
        });
      }
      if ((w.prs || []).length === 0 && (w.tickets || []).length)
        out.push({ kind: "no-prs", hard: false, text: "No PR recorded for this wave yet" });
    }
    if (w.state === "review" || w.state === "active") {
      const threads = mine.reduce((n, p) => n + (p.unanswered > 0 ? p.unanswered : 0), 0);
      if (threads)
        out.push({
          kind: "threads",
          hard: false,
          urls: mine.filter((p) => p.unanswered > 0).map((p) => p.url),
          text: `${threads} PR thread${threads === 1 ? "" : "s"} waiting on you`,
        });
    }
    const errs = mine.filter((p) => p.error);
    if (errs.length)
      out.push({
        kind: "error",
        hard: false,
        text: `${errs.length} PR${errs.length === 1 ? "" : "s"} not reached on the last refresh`,
      });
    return out;
  }

  /// The actions a wave's card offers, as ids app.js binds to the bar's own
  /// handlers (so a card and the bar can never disagree):
  /// `approve-next` / `approve` (the round in review — every wave in it, since
  /// waves are approved by round), `start` (a ready wave, pre-picked in the
  /// wave picker), `review` (a code review scoped to its open PRs), `open-prs`,
  /// `session` (the agent building it). What a wave waits for is a link on
  /// its blocker line, not a button.
  /// Decisions only where the bar would offer them: a parked stage, and a
  /// sign-off only at a code stage.
  function waveActions(item, w, prs) {
    const meta = (item && item.meta) || {};
    const st = meta.status;
    const parked = ["plan-review", "diff-review", "pr-draft", "pr-ready"].includes(st);
    const codeStage = ["diff-review", "pr-draft", "pr-ready"].includes(st);
    const bar = waveBarState(item);
    const mine = wavePrs(w, prs).prs;
    const open = mine.filter((p) => !settledPr(p));
    const acts = [];
    if (codeStage && bar && (bar.kind === "approve" || bar.kind === "approve-last") && bar.review.includes(w.id)) {
      const round = waveList(bar.review);
      if (bar.kind === "approve")
        acts.push({ id: "approve-next", label: `✓ Sign off ${round} → next waves…`, primary: true, title: "Seal this round's work and pick the waves that start next" });
      acts.push({
        id: "approve",
        label: bar.kind === "approve" ? `✓ Sign off ${round} only` : `✓ Sign off ${round}`,
        primary: bar.kind !== "approve",
        title: bar.review.length > 1 ? `Waves are signed off by round: this seals ${round}` : "Seal this wave's work: later rounds leave it alone",
      });
    }
    if (parked && bar && bar.kind === "start" && w.state === "ready") {
      // The plan allows parallel waves; running them in parallel is your call.
      // The set grows from this wave with every ready sibling compatible
      // with all of those already in it — what the picker would accept.
      const board = waveBoard(item);
      const set = [w];
      for (const o of board.waves) {
        if (o.id === w.id || o.state !== "ready") continue;
        if (set.every((x) => (x.compatibleWith || []).includes(o.id) && (o.compatibleWith || []).includes(x.id)))
          set.push(o);
      }
      const verb = st === "plan-review" ? "✓ Approve plan → start" : "▶ Start";
      if (set.length > 1) {
        const ids = set.map((x) => x.id);
        acts.push({
          id: "start",
          ids,
          label: `${verb} ${waveList(ids)} together…`,
          primary: true,
          title: "One round, one agent, built side by side — the plan says they touch different code. Spends tokens.",
        });
        acts.push({
          id: "start",
          ids: [w.id],
          label: `${verb} ${waveName(w)} alone…`,
          title: "One wave at a time — slower, simpler to review",
        });
      } else {
        acts.push({
          id: "start",
          ids: [w.id],
          label: `${verb} ${waveName(w)}…`,
          primary: true,
          title: "Pick the waves of the next round, this one ticked — spends tokens",
        });
      }
    }
    if (codeStage && open.length && (w.state === "review" || w.state === "done"))
      acts.push({
        id: "review",
        label: `⌕ Review ${open.length === 1 ? "its PR" : `its ${open.length} PRs`}…`,
        urls: open.map((p) => p.url),
        title: "A code review round over this wave's open PRs — spends tokens",
      });
    if (open.length)
      acts.push({
        id: "open-prs",
        label: `Open ${open.length === 1 ? "PR" : `${open.length} PRs`}`,
        urls: open.map((p) => p.url),
      });
    if (w.state === "active") acts.push({ id: "session", label: "Open session", title: "The agent building this wave" });
    return acts;
  }

  /// What signing off the round in review does, said before it is done:
  /// `{ title, effects, warnings, drafts }`. A sign-off seals the waves'
  /// work (the commit it ends at is recorded, and every later executor round
  /// is told to leave it alone), unlocks what depended on it, and may hand the
  /// waves' draft PRs to GitHub review — the one effect that leaves clash,
  /// so it is offered (`drafts`), never assumed. Null when nothing waits.
  function signOffModel(item, prs) {
    const bar = waveBarState(item);
    if (!bar || !bar.review.length) return null;
    const board = waveBoard(item);
    const byId = new Map(board.waves.map((w) => [w.id, w]));
    const round = bar.review.map((id) => byId.get(id)).filter(Boolean);
    const names = round.map(waveLabel).join(", ");
    const unlocked = unlockedBy(board, bar.review);
    const effects = [
      `Seals ${round.length > 1 ? "their" : "its"} work at the current commit — later agent rounds are told it is accepted and to leave it alone.`,
      unlocked.length
        ? `Unlocks ${unlocked.map(waveLabel).join(", ")}.`
        : bar.kind === "approve-last"
          ? "These are the last waves: the item's own approval comes next."
          : "Unlocks nothing yet: the next waves wait for other waves too.",
    ];
    const warnings = [];
    const open = (item && item.openAnnotations) || 0;
    if (open) warnings.push(`${open} comment${open === 1 ? " is" : "s are"} still open on the diff.`);
    for (const w of round)
      for (const b of waveBlockers(item, w, prs))
        if (!b.hard) warnings.push(`${waveName(w)}: ${b.text}.`);
    // A PR shared by several waves of the round is offered once, naming them.
    const drafts = [];
    for (const w of round)
      for (const p of wavePrs(w, prs).prs) {
        if (!p.draft || settledPr(p)) continue;
        const had = drafts.find((d) => d.url === p.url);
        if (had) had.waves.push(w.id);
        else drafts.push({ url: p.url, wave: w.id, waves: [w.id] });
      }
    return {
      title: `Sign off ${names}?`,
      effects,
      warnings,
      drafts,
      okLabel: `Sign off ${waveList(bar.review)}`,
    };
  }

  /// The wave "one at a time" opens on: the one that needs you (to approve),
  /// else the one being built, else the next to start, else the first not yet
  /// approved, else the last. `focus` wins when it still names a wave.
  function waveFocus(item, focus) {
    const board = waveBoard(item);
    if (!board || !board.waves.length) return null;
    const ws = board.waves;
    if (focus && ws.some((w) => w.id === focus)) return focus;
    for (const st of ["review", "active", "ready"]) {
      const w = ws.find((x) => x.state === st);
      if (w) return w.id;
    }
    const open = ws.find((x) => x.state !== "done");
    return (open || ws[ws.length - 1]).id;
  }

  const api = {
    signOffModel,
    waveBlockers,
    waveActions,
    waveLive,
    waveFocus,
    waveBoard,
    wavesIssue,
    waveName,
    waveLabel,
    waveList,
    waveSummary,
    waveBarState,
    wavesHoldApproval,
    wavesIssueGuidance,
    wavePickRows,
    defaultWavePick,
    toggleWavePick,
    waveColumns,
    waveStateLabel,
    wavePrs,
    waveNarrative,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = api;
  else Object.assign(window, api);
})();
