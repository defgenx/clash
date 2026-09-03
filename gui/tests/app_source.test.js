// node --test gui/tests/ — invariants over the frontend source itself.
//
// `app.js` is a plain script against the Tauri globals, so it cannot be
// `require`d in isolation. These are the properties worth pinning anyway: they
// are the ones whose violation is silent at runtime and expensive to debug.
const { test } = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const APP_PATH = path.join(__dirname, "..", "dist", "app.js");
const APP = fs.readFileSync(APP_PATH, "utf8");
const APP_BYTES = fs.readFileSync(APP_PATH);

test("app.js contains no NUL bytes", () => {
  // A single NUL makes grep treat the whole file as binary: `grep -n BROWSE
  // app.js` silently returns nothing without `-a`, so every future reader loses
  // time to a search that appears to prove the symbol doesn't exist.
  //
  // This assertion is only safe now that the repo picker's "Browse…" sentinel is
  // a Symbol. Deleting the byte while the sentinel was still "\x00browse" would
  // have demoted it to the plausible-as-a-path string "browse" — a weaker
  // invariant locked in by a passing test.
  const indexes = [];
  for (let i = 0; i < APP_BYTES.length; i++) {
    if (APP_BYTES[i] === 0) indexes.push(i);
  }
  assert.deepEqual(indexes, [], `NUL byte(s) at offset(s) ${indexes.join(", ")}`);
});

test("the repo picker sentinel is a Symbol, not a string", () => {
  // A string sentinel can collide with something a user types or pastes; the
  // picker compares by identity, so a Symbol cannot.
  assert.match(APP, /const BROWSE = Symbol\("browse"\)/);
  assert.doesNotMatch(APP, /const BROWSE = "/);
});

test("cross-frontend settings are not persisted in the GUI blob", () => {
  // The 7 shared settings live in config.toml from the migration onward. If the
  // blob kept a copy, a stale gui-state.json — or WKWebView's localStorage,
  // which resolves the real user home even under an isolated HOME — would
  // overwrite a hand-edited config.toml on the next boot.
  assert.match(APP, /settings: guiLocalSettings\(\)/);
  assert.doesNotMatch(APP, /^\s+settings: state\.settings,$/m);
  assert.match(APP, /function guiLocalSettings\(\)/);
});

test("only a disk blob may seed the settings migration", () => {
  // localStorage is a same-session fallback and is not HOME-isolated, so it must
  // never be a migration source.
  assert.match(APP, /applyWorkspacesData\(JSON\.parse\(raw\), \{ migratable: true \}\)/);
  assert.match(APP, /applyWorkspacesData\(JSON\.parse\(raw\), \{ migratable: false \}\)/);
});

test("settings are resolved from the backend before the first paint", () => {
  // applyTheme and syncSettingsUi both read state.settings; loading settings
  // after them would flash the default palette.
  const loadSettings = APP.indexOf("await loadSettings();");
  const applyTheme = APP.indexOf("applyTheme(state.settings.theme);\n  applyMonoFont();");
  assert.ok(loadSettings > 0, "boot must await loadSettings()");
  assert.ok(applyTheme > 0, "boot must apply the theme");
  assert.ok(loadSettings < applyTheme, "loadSettings() must precede applyTheme()");
});

test("a config reload coalesces refits to one animation frame", () => {
  // A reload can push xterm options to every open terminal and refit each one.
  // At the default 200ms watcher debounce, an editor saving per keystroke would
  // otherwise become a burst of refits across every pane.
  assert.match(APP, /function queueRefit\(\)/);
  assert.match(APP, /requestAnimationFrame\(\(\) => \{\s*refitQueued = false;/);
  // And the refit path is entered only for the keys the backend flagged.
  assert.match(APP, /payload\.refit && payload\.refit\.length\) queueRefit\(\)/);
});

test("shared settings persist through config.toml, GUI-local ones through the blob", () => {
  assert.match(APP, /function persistSetting\(key\)/);
  assert.match(APP, /if \(!sharedSettingKeys\.has\(key\)\) \{\s*saveWorkspaces\(\);/);
  assert.match(APP, /invoke\("config_set", \{ key, value: state\.settings\[key\] \}\)/);
  // The generic binders route through it rather than writing the blob directly.
  assert.match(APP, /else persistSetting\(key\);/);
});

test("the hand-written per-key settings validation is gone", () => {
  // One Rust validator now covers both frontends; these were the JS duplicates
  // of the schema's own range checks.
  assert.doesNotMatch(APP, /function numSetting\(/);
  assert.doesNotMatch(APP, /typeof s\.confirmKill === "boolean"/);
  assert.doesNotMatch(APP, /s\.embedLinks \? "embedded" : "external"/);
});

test("an early flush cannot persist unresolved defaults", () => {
  // blur/pagehide fire on any click away, including before boot finishes. At
  // that point state.settings still holds the in-code defaults, so persisting
  // them would let the next boot migrate defaults over the user's real values.
  assert.match(APP, /if \(!settingsResolved\) return loadedSettingsBlob \|\| \{\};/);
  assert.match(APP, /settingsResolved = true;/);
});

test("approving a diff never requires a draft PR", () => {
  // The only primary action at diff-review used to be "Approve & create draft
  // PR", which is a dead end for any repo that merges to its default branch
  // without one. `→ done` must always be offered, and the PR path must be
  // reachable only as a separate, optional action.
  const diffReview = APP.slice(
    APP.indexOf('case "diff-review": {'),
    APP.indexOf('case "pr-draft": {')
  );
  assert.ok(diffReview.length > 0, "the diff-review case must exist");
  assert.match(diffReview, /✓ Approve → done/);
  assert.match(diffReview, /wfTransition\(item, root, "done"\)/);
  // No approve action creates a PR as part of approving.
  assert.doesNotMatch(diffReview, /Approve & create draft PR/);
  // Creating a PR is its own opt-in button, offered only when none exists.
  assert.match(diffReview, /add\("Create draft PR…"/);
});

test("requesting changes uses the composer, not a one-line prompt", () => {
  // The note is appended verbatim to review.md and read by the agent as its
  // instructions for the next round — a single-line <input> could not hold it,
  // and could not contain a newline at all.
  assert.match(APP, /function wfComposeChangeRequest\(/);
  // Both the plan and the diff path go through the one requestChanges flow
  // (composer + workflow_request_changes, which snapshots the iteration);
  // they used to be two divergent inline prompts, and the plan path used to
  // skip the snapshot entirely.
  assert.match(APP, /wfComposeChangeRequest\(\{\s*item,\s*target,\s*annotations,\s*onJump[\s\S]{0,80}\}\)/);
  assert.match(APP, /requestChanges\("plan"\)/);
  assert.doesNotMatch(APP, /uiPrompt\("What should change in the plan\?"\)/);
  assert.doesNotMatch(APP, /Request changes — describe what to change/);
});

test("every dialog backdrop attaches its box", () => {
  // A dialog builder that forgets `backdrop.appendChild(box)` still "works":
  // the backdrop dims the screen, the handlers run, nothing throws — but the
  // dialog itself is invisible and the app looks frozen. This shipped once
  // (the change-request composer), so pin the pairing: one box attach per
  // backdrop creation.
  const backdrops = APP.match(/className = "dialog-backdrop"/g) || [];
  const attaches = APP.match(/backdrop\.appendChild\(box\)/g) || [];
  assert.ok(backdrops.length > 0, "dialog builders must exist");
  assert.equal(
    attaches.length,
    backdrops.length,
    "every `dialog-backdrop` needs a matching `backdrop.appendChild(box)`"
  );
});

test("PR-identity errors recover in place instead of dead-ending", () => {
  // The backend prefixes identity-shaped failures (no-pr: / pr-number-unknown:)
  // and the frontend must answer them by asking for the PR URL, attaching it
  // and retrying — never by a bare alert the user can't act on.
  assert.match(APP, /async function wfPrRecovery\(item, err, retry\)/);
  assert.match(APP, /msg\.startsWith\("pr-number-unknown:"\)/);
  // Every surface that demands a PR identity goes through the recovery:
  // Mark ready, Post round to PR, and the review-round launcher.
  const uses = APP.match(/await wfPrRecovery\(/g) || [];
  assert.ok(uses.length >= 3, `expected ≥3 wfPrRecovery call sites, got ${uses.length}`);
  // The launcher's no-pr path offers the local downgrade, not just attach.
  assert.match(APP, /Run the round locally instead/);
});

test("a dismissed composer keeps the draft", () => {
  // Losing a paragraph to a stray Esc or backdrop click is the failure this
  // guards; submitting clears it so the next round starts empty.
  assert.match(APP, /const wfDrafts = new Map\(\)/);
  assert.match(APP, /const dismiss = \(\) => \{\s*keepDraft\(\);/);
  // Submitting clears the draft and resolves the composer's result object.
  assert.match(APP, /wfDrafts\.delete\(key\);\s*done\(\{\s*note,/);
});

test("no dialog dismisses on a bare backdrop click", () => {
  // `click` fires on the nearest common ancestor of the mousedown and mouseup
  // targets, so drag-selecting text in a dialog field and releasing past the
  // box edge targets the backdrop — a bare `e.target === backdrop` handler read
  // that as an outside click and cancelled the dialog mid-edit (renaming a
  // session, selecting the old name). Every scrim goes through the helper,
  // which requires the press and the release to both land on the scrim.
  assert.match(APP, /function wireBackdropDismiss\(backdrop, dismiss, hits =/);
  assert.doesNotMatch(APP, /e\.target === backdrop/);
  assert.doesNotMatch(APP, /e\.target === \$\("modal-backdrop"\)/);
  // One wiring per dialog builder (plus the new-session modal and the tour
  // scrim, whose backdrops are not `.dialog-backdrop`).
  const backdrops = (APP.match(/className = "dialog-backdrop"/g) || []).length;
  const wired = (APP.match(/^\s*(?:if \(cancelable\) )?wireBackdropDismiss\(/gm) || []).length;
  assert.equal(wired, backdrops + 2, "every backdrop needs one wireBackdropDismiss");
});

test("a click inside the already-focused pane repaints nothing", () => {
  // renderPanes detaches every pane element before re-appending it, and
  // detaching an ancestor of the focused node blurs it. A pane-level click
  // handler that repaints unconditionally therefore drops the caret on every
  // click, which makes a text field in a view tab (the workflow ⚙ Settings
  // knobs) impossible to type into — it looks like a broken input, not a
  // layout repaint.
  assert.match(APP, /pane\.onclick = \(\) => \{\s*if \(w\.focused === i\) \{/);
  // And the repaints that do happen hand focus back, scoped to the pane that
  // still holds it so a pane switch doesn't drag focus to the pane we left.
  assert.match(APP, /const wasFocused = document\.activeElement;/);
  assert.match(APP, /focusedEl\?\.contains\(wasFocused\)/);
});

test("a workflow tab rebuild never lands on a field being edited", () => {
  // The watcher fires on every meta write, including clash's own. A rebuild
  // replaces the tab's DOM, so it must defer while anything on the tab is
  // being typed into — not just the comment composer, which was the only
  // protected surface while ⚙ Settings shipped its first text fields.
  const guard = APP.slice(APP.indexOf("function rebuildOpenWorkflowTabs()"));
  assert.match(guard, /active\.tagName === "TEXTAREA"/);
  assert.match(guard, /composer\.contains\(active\)/);
  // Our own save echoing back is not "changed on disk".
  assert.match(guard, /wfSelfWriteAt < 3000/);

  // And the per-item settings save does not rebuild the view at all: nothing
  // on that tab is derived from the values being edited, and the rebuild
  // landed after `change` had already moved focus to the next field.
  const settings = APP.slice(
    APP.indexOf('if (ts.subView === "settings") {'),
    APP.indexOf('// diff sub-view')
  );
  assert.ok(settings.length > 0, "the settings sub-view must exist");
  assert.doesNotMatch(settings, /buildWorkflowView\(/);
  assert.match(settings, /Object\.assign\(committed, patch\)/);
});

test("the queued-follow-up UI mirrors the backend rather than guessing", () => {
  // The queue lives in the backend, which delivers on its own schedule. If the
  // frontend tracked it locally, a delivery would leave the row claiming a
  // prompt is still pending — so every change re-reads the whole map.
  assert.match(APP, /async function refreshQueuedPrompts\(/);
  assert.match(APP, /state\.queued = await invoke\("list_queued_prompts"\)/);
  // Queued, cancelled, delivered: all three paths end in a re-read.
  const reads = APP.match(/refreshQueuedPrompts\(/g) || [];
  assert.ok(reads.length >= 5, `expected ≥5 refreshQueuedPrompts sites, got ${reads.length}`);
  assert.match(APP, /listen\("prompt-delivered"/);
  // The composer is multi-line: the text is a message to Claude, and a
  // single-line field cannot hold a prompt with a list in it.
  assert.match(APP, /await uiTextPrompt\(\s*`Follow-up for/);
});

test("applying a review round goes through the change-round flow", () => {
  // A path that revised the plan without recording a round would leave the
  // revision untraceable and unversioned — which is exactly what the
  // snapshotting flow exists to prevent. So "Apply review" must compose a note
  // and call the same command the composer does, never a shortcut of its own.
  assert.match(APP, /const applyReview = async \(\) => \{/);
  // Keyed on "the latest section", not on a round number: numbers restart per
  // phase, so a by-number lookup can name two different rounds.
  assert.match(APP, /applyReviewNote\(round, latestAgentRoundFindings\(md\), target\)/);
  assert.doesNotMatch(APP, /roundFindings\(md, round\.round\)/);
  const apply = APP.slice(
    APP.indexOf("const applyReview = async () => {"),
    APP.indexOf("const applyReviewButton = ")
  );
  // The note and the recording both come from the shared helpers, so the click
  // path and the pre-authorized path cannot diverge.
  assert.match(apply, /await wfApplyReviewNoteFor\(item, round, target\)/);
  assert.match(apply, /await wfRecordAndRevise\(item, root, note\)/);
  // And the second way out is the composer, pre-filled with the same note.
  assert.match(apply, /requestChanges\(target, note\)/);
  assert.match(APP, /function wfComposeChangeRequest\(\{ item, target, annotations, onJump, prefill = "" \}\)/);
  // A kept draft outranks the prefill: one is the human's unfinished sentence.
  assert.match(APP, /const seeded = \(wfDrafts\.get\(key\) \|\| ""\)\.trim\(\) \? "" : prefill;/);
});

test("every workflow sub-view renderer is actually reachable", () => {
  // This is the test that was missing: `renderWfPlanView` shipped defined but
  // never called, so the Plan tab still rendered the plain document and the
  // whole version reader was dead code — while a test asserting the function
  // *existed* passed, and a smoke test calling it directly passed too.
  // Existence is not reachability.
  const defined = [...APP.matchAll(/^async function (renderWf\w*View)\(/gm)].map((m) => m[1]);
  assert.ok(defined.length >= 4, `expected several sub-view renderers, got ${defined}`);
  for (const name of defined) {
    const calls = (APP.match(new RegExp(`${name}\\(`, "g")) || []).length;
    // Definition + at least one call that is not its own recursion.
    assert.ok(calls >= 2, `${name} is defined but never called`);
    const dispatch = new RegExp(`return ${name}\\(|${name}\\(body`);
    assert.match(APP, dispatch, `${name} is never dispatched from a sub-view router`);
  }
  // And the two plan readers are wired to their own sub-views.
  assert.match(APP, /if \(ts\.subView === "plan"\) return renderWfPlanView\(/);
  assert.match(APP, /if \(ts\.subView === "revisions"\) return renderWfRevisionsView\(/);
  // The generic doc branch must not claim the plan any more, or it would win.
  const generic = APP.slice(APP.indexOf('ts.subView === "review" ||'));
  assert.doesNotMatch(generic.slice(0, 400), /ts\.subView === "plan"/);
  // The tab exists, so the view is reachable by a human and not just by code.
  assert.match(APP, /\["revisions", "◫ Revisions"\]/);
});

test("the plan has one version reader, not three views", () => {
  // The Timeline's drill-downs and the Plan tab used to render the same data
  // three ways; two of them are retired and the diff colouring lives once.
  assert.match(APP, /async function renderWfPlanView\(/);
  assert.match(APP, /function renderUnifiedDiff\(container, text\)/);
  assert.doesNotMatch(APP, /ts\.subView === "planAt"\)? \{/);
  // The plan is versioned continuously, so the reader asks the revision store
  // rather than the round snapshots — a plan written between rounds, or edited
  // by hand, has no round to be found under.
  assert.match(APP, /invoke\("list_workflow_plan_versions"/);
  assert.match(APP, /invoke\("get_workflow_plan_version", \{ project, slug, n: sel\.n \}\)/);
  assert.doesNotMatch(APP, /get_workflow_history_plan/);
  // A Timeline plan link resolves iteration → revision when clicked, because
  // the answer changes while the card is on screen.
  assert.match(APP, /planVersionForIteration\(versions, iteration\)/);
  assert.doesNotMatch(APP, /if \(ts\.subView === "planDiff"\)/);
  // A tab persisted before the consolidation still lands on the plan.
  assert.match(APP, /if \(ts\.subView === "planAt" \|\| ts\.subView === "planDiff"\)/);
  // One `pd-` renderer: the class assignments appear in exactly one function.
  const hunks = APP.match(/span\.className = "pd-hunk"/g) || [];
  assert.equal(hunks.length, 1, "the unified-diff colouring must exist once");
});

test("a pre-authorized round applies itself through the same one mechanism", () => {
  // Two entry points — the button and the hand-back — must not become two
  // implementations, or one of them ends up skipping the snapshot that
  // versions the plan.
  assert.match(APP, /async function wfRecordAndRevise\(item, root, note\)/);
  const mech = APP.slice(
    APP.indexOf("async function wfRecordAndRevise("),
    APP.indexOf("// Items whose auto-apply is in flight")
  );
  assert.match(mech, /invoke\("workflow_request_changes"/);
  assert.match(mech, /launchWfAgent\(fresh, "revise"/);
  // Exactly one caller composes the note, and both paths use it.
  assert.match(APP, /async function wfApplyReviewNoteFor\(item, round, target\)/);
  assert.equal((APP.match(/wfRecordAndRevise\(/g) || []).length, 3); // def + 2 callers
  // The auto path is guarded against a doubled hand-back event, and gated by
  // the pure rule rather than an inline condition.
  assert.match(APP, /const wfAutoApplying = new Set\(\)/);
  assert.match(APP, /if \(!item \|\| !shouldAutoApply\(item, review\)\) return;/);
  assert.match(APP, /wfMaybeAutoApplyReview\(project, slug, review\);/);
  // And the launch surface passes the checkbox through.
  assert.match(APP, /autoApply: picked\.autoApply,/);
  assert.match(APP, /autoApply,\n\s+cols: 120,/);
});

test("a session share hands off without leaking credentials or the payload's shape", () => {
  // The third route exists to reach destinations clash has no client for —
  // with a named skill, or with whatever the session has connected. Three
  // properties matter: it goes through a session (so clash holds no
  // credentials for that route), an unnamed skill is sent as null rather than
  // an empty string the backend would have to interpret, and the human can
  // watch it.
  assert.match(APP, /invoke\("share_workflow_via_agent", \{/);
  const send = APP.slice(
    APP.indexOf('} else if (d.route === "agent") {'),
    APP.indexOf('} else if (d.id === "jira") {')
  );
  assert.match(send, /destination: d\.id/);
  assert.match(send, /skill: d\.skill \|\| null/);
  // It spends tokens and carries none of clash's credentials, so it asks —
  // naming whichever of the two will do the posting.
  assert.match(send, /await uiConfirm\(/);
  assert.match(send, /It will use the \$\{d\.skill\} skill/);
  assert.match(send, /tools that session has connected/);
  assert.match(send, /await openSession\(sid/, "the session opens so the post can be checked");
  // The Jira API token is write-only: nothing reads it back into the webview.
  assert.doesNotMatch(APP, /\.value = s\.jiraApiToken/);
  assert.match(APP, /s\.jiraTokenSet \? "•••••• \(stored\)" : "API token"/);
});

test("explain is offered from every state that is not mid-agent", () => {
  // It judges nothing and writes only its own document, so the question a
  // *review* has to ask — "is this artifact parked on my decision" — is the
  // wrong gate for it. Three of the eight status cases used to call it.
  assert.match(APP, /const wfCanExplain = \(item\) =>\s*!WF_WORKING\.has\(item\.meta\.status\)/);
  assert.match(APP, /if \(!wfCanExplain\(item\)\) return;/);
  // Called once, outside the switch — not remembered per case.
  assert.equal((APP.match(/^\s*explainButton\(\);$/gm) || []).length, 1);
  assert.doesNotMatch(APP, /wfCanReview\(item\) \|\| item\.meta\.status === "plan-review"/);
});

test("each workflow document says what it is", () => {
  // "Review" and "Agent reviews" as bare tab labels left the difference to be
  // inferred from the contents.
  assert.match(APP, /\["review", `Change requests\$\{/);
  assert.match(APP, /className = "wf-doc-caption dim"/);
  const cap = APP.slice(APP.indexOf('caption.textContent ='), APP.indexOf('body.appendChild(caption)'));
  assert.match(cap, /Your change requests/);
  assert.match(cap, /What the agent review rounds found/);
  assert.match(cap, /It judges nothing/);
});

test("the blueprint carries a decision, and none of the three is a fallback", () => {
  // A blueprint is read before the implementation exists, so agreeing on the
  // shape of the work is the point — and each answer does a different thing.
  assert.match(APP, /function wfBlueprintDecisionBar\(root, item\)/);
  const bar = APP.slice(
    APP.indexOf("function wfBlueprintDecisionBar("),
    APP.indexOf("/// Render a unified diff as coloured lines")
  );
  assert.match(bar, /invoke\("set_workflow_blueprint_decision"/);
  for (const d of ['decide\\("accepted"\\)', 'decide\\("rejected"', 'decide\\("stale"\\)']) {
    assert.match(bar, new RegExp(d), `missing the ${d} answer`);
  }
  // Rejecting means the *plan* needs a round, through the same composer as
  // every other change request — not a second mechanism.
  assert.match(bar, /await wfRequestChanges\(\s*fresh,\s*root,\s*"plan",/);
  // Accepting does not move the pipeline; approving is still its own click.
  assert.doesNotMatch(bar, /wfTransition/);
  // A pending blueprint demotes the stage's approve, like a pending review.
  assert.match(APP, /const blueprintPending = blueprintState\(item\) === "pending";/);
  assert.match(APP, /reviewPending \|\| blueprintPending \? "" : "primary"/);
  // Before the work exists the explainer draws a blueprint instead of a
  // structure of a diff that does not exist yet.
  assert.match(APP, /const forward = item\.meta\.status === "plan-review" && wfHasPlanPhase\(item\)/);
  assert.match(APP, /\{ target: "blueprint" \}/);
});
