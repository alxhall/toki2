# Toki TUI task breakdown

Plan and decision gates: [`tasks/plan.md`](plan.md). **Approved; Phase 0 underway.** Baseline evidence: [`tasks/baseline.md`](baseline.md). The code work starts from `release/v0.4.0` in an isolated worktree; TUI releases are fork-owned and require no upstream PR. This checklist is a planning artifact, not a second issue tracker: existing bug state remains in Aven. Do not bulk-create or change Aven items until requested. Check off items only with recorded evidence; conditional tasks may be split after their spikes.

## Phase 0 — Baseline and evidence

### 0.1 Inventory released source and reproducible builds
- [ ] Identify the 0.4.0 source/tag/artifact revisions and an isolated branch/worktree rooted at `release/v0.4.0`; record differences from local and upstream `master` relevant to the TUI. Preserve release CI added directly on 0.4.0; decide separately whether a fork-owned integration branch is useful, without rewriting stale `fork/base`. Leave existing branches untouched.
- [ ] Run `SQLX_OFFLINE=true cargo test -p toki-tui` and the release-build path where available; record compiler/toolchain and Windows build results or an explicit access blocker.
- [ ] Document existing start/save/continue/edit behavior with a dev or approved test account, not unapproved production writes.
**Depends on:** none. **Likely files:** plan/evidence docs only. **Size:** S.

### 0.2 Pin down deployed API and contract
- [ ] Compare deployed `/openapi.json` (if available), upstream contract, and the 0.4.0 client for auth, project/activity updates, timer save/restart, error codes and time-entry response shape.
- [ ] Capture only sanitized read response fixtures and approved test-environment write fixtures. Record which compatibility differences block the daily workflow and the smallest necessary adapter changes.
- [ ] Confirm whether a save takes project/activity from the server-side timer rather than the save request; flag potential lost updates before any release.
**Depends on:** 0.1. **Verification:** schema/fixture deserialization tests or a contract comparison with explicit absent-spec caveat; no secrets in fixtures. **Likely files:** `toki-tui/src/api/{client,dto}.rs`, test fixtures (only when implementation begins). **Size:** M.

### 0.3 Reproduce slow and uncertain saves
- [ ] Build/use a local HTTP test double with controlled 10-second response, pre-commit failure, committed/lost-response, and slow history fetch.
- [ ] Establish a baseline of UI responsiveness, number of writes per key sequence, and state after a forced restart of the TUI.
**Depends on:** 0.2. **Verification:** repeatable scripted/manual scenario and a failing focused test before behavior changes. **Likely files:** TUI runtime tests and local fixture/test support. **Size:** M.

**Checkpoint A — Agree with user:** compatibility matrix, reproducible slow-save case, and any backend contract blocker are reviewed. No blind upstream merge.

## Phase 1 — Save reliability

### 1.1 Model one in-flight save and explicit outcome states
- [ ] Represent idle, saving, confirmed, uncertain and failed states; make duplicate save keys single-flight without discarding edit context.
- [ ] Before sending a write, persist minimal, restricted pending metadata without note text, cookie or token. Refuse to write if persistence fails; recognize unfinished attempts after restart. Keep status and relevant keys usable, and make quit while saving explicit/safe.
**Depends on:** 0.3. **Verification:** state-transition tests for repeated save, cancel/quit and explicit failure; `SQLX_OFFLINE=true cargo test -p toki-tui`. **Likely files:** `toki-tui/src/app/{mod,state}.rs`, `toki-tui/src/runtime/views/*`, focused UI tests. **Size:** M.

### 1.2 Keep I/O off the event loop
- [ ] Only after the durable guard exists, dispatch save and its result through bounded/coalesced work without awaiting HTTP in the render/input loop; prevent polling from queuing unlimited history refreshes.
- [ ] Serialize timer mutations; ignore stale read completions rather than letting them overwrite a newer timer. Add visible progress and finite request deadlines.
**Depends on:** 1.1. **Verification:** 10-second fixture yields responsive redraw/keypress feedback (target <250 ms for local UI handling), one save request per intended action, no growing queue; `SQLX_OFFLINE=true cargo test -p toki-tui`. **Likely files:** `toki-tui/src/runtime/{event_loop,action_queue,actions}.rs`, `toki-tui/src/api/client.rs`, UI status. **Size:** M; split implementation into separate dispatch and bounded-poll changes if it exceeds one focused session.

### 1.3 Reconcile ambiguous save outcomes
- [ ] On deadline/network loss **or 500 after a possible provider commit**, retain the pending marker and compare the server's active timer and recent entries with the attempted save; do not automatically retry a non-idempotent write.
- [ ] If still ambiguous/offline, retain non-secret pending context, show an explicit unknown state, and reconcile at reconnect/startup before permitting unsafe replay.
**Depends on:** 1.2. **Verification:** tests cover successful response, explicit no-write failure, committed/lost-response, offline-after-timeout and app restart; no duplicated entries. If conclusive reconciliation is impossible, capture a backend idempotency/lookup decision for review instead of faking certainty. **Likely files:** `toki-tui/src/runtime/actions.rs`, `toki-tui/src/api/client.rs`, `toki-tui/src/session_store.rs` (or a separate non-secret pending-state store). **Size:** M per sub-slice; split if backend support is required.

### 1.4 Make save-and-continue and history refresh trustworthy
- [ ] Decide, from the deployed contract, whether optional `restartTimer` or two calls give the safest semantics; distinguish saved-but-not-restarted from save failure. **Normal-path evidence:** user verified one Windows diagnostic save-and-stop (exactly one correct entry; no active timer) and later separately approved/verified one save-and-continue on the same project (**option 3**; exactly one new entry and a running replacement timer on the same project/activity). Both were genuine work kept by the user. Uncertain production outcomes remain untested and must not be deliberately induced.
- [ ] Apply confirmed server state and refresh history asynchronously; preserve the previous project/note when rollback is necessary and ensure refresh errors do not mask a successful save.
**Depends on:** 1.3. **Verification:** stub tests for save-and-stop, both continue modes, restart failure and stale/failed refresh; native-Windows and Linux manual save smoke test; `SQLX_OFFLINE=true cargo test -p toki-tui`. **Likely files:** `toki-tui/src/api/{client,dto}.rs`, `toki-tui/src/runtime/actions.rs`, app tests. **Size:** M.

**Checkpoint B — Save gate:** input remains responsive under 10-second delay; no duplicate, lost or falsely confirmed save in the test matrix. Review with users before any release candidate.

## Phase 2 — Native-Windows diagnosis (can run alongside phase 1 after 0.1)

### 2.1 Measure the reported memory growth and long-timer crash
- [ ] Clarify the colleague's report: a Windows Terminal tab reached ~25 GB; after restart it reportedly gained ~7 GB in an hour while input was unresponsive, then stopped growing when input resumed. Determine native executable vs WSL, which **process** consumed memory, process-tree/terminal details and the immediately preceding action.
- [ ] Measure separate PIDs for TUI, Windows Terminal/renderer and children (private bytes, working set, handles and CPU) every 5–10 seconds through startup, idle, running timer, slow save and slow refresh with the 0.4.0 release artifact and a comparable diagnostic build. Record exactly when input stalls/resumes. Stop early at a safe limit **well below 7 GB**; extend to multi-hour runs only after short runs prove bounded. Compare another terminal and Linux as controls.
- [ ] Separately attempt to reproduce the long-running-timer crash; establish whether it correlates before combining diagnoses.
**Depends on:** 0.1. **Verification:** reproducible per-process measurements and sanitized timeline showing growth/stall correlation; agree on a numeric ceiling after establishing a stable baseline. **Likely files:** test protocol/evidence docs; optional diagnostic harness. **Size:** M.

### 2.2 Fix the measured resource or timer defect
- [ ] Profile the specific reproducing path; isolate allocation/retention or overflow before editing production logic. If memory and crash have different causes, track them separately. **Idle evidence:** unchanged v0.4.0 rendered at 10 Hz (~1,225 ANSI bytes/5 s); the focused redraw build emitted zero and kept native Windows Terminal private memory ~86 MiB for 110 seconds (versus +245.5 MiB for v0.4.0 at 110 seconds). A separate 110-second native Windows `dev` run with an advancing visible timer kept Terminal at 88.65 → 88.23 MiB; this does not prove the long-running-timer crash fixed.
- [ ] Add a focused regression guard and repeat the same Windows protocol under the candidate binary.
**Depends on:** 2.1. **Verification:** Windows memory stays within the agreed budget/plateaus in the repeatable workload, crash no longer reproduces, and `SQLX_OFFLINE=true cargo test -p toki-tui` passes. **Likely files:** determined by profiling, not preselected. **Size:** conditional; split by cause.

**Checkpoint C — Windows gate:** Windows tester confirms per-process attribution and a measured, bounded result. Given the reported ~7 GB/hour and unresponsiveness, an unexplained repeat of this symptom blocks release unless the user explicitly accepts the risk.

## Phase 3 — Authentication (after save gate)

### 3.1 Trial personal-token authentication against the deployed API
- [ ] With a disposable, revocable test token, confirm bearer access to the TUI's `/me`, connection, timer, projects, history and time-info routes; distinguish 401 from unmapped Kleer user.
- [ ] Inventory token issuance/revocation and secure storage options per OS; document why OpenAPI publication does not constrain token authority.
**Depends on:** 0.2, checkpoint B. **Verification:** sanitized contract results and a threat/UX decision, without storing a real token in a file or logs. **Likely files:** docs only for spike. **Size:** S.

### 3.2 Add opt-in token mode without regressing session users
- [ ] Choose secure OS credential storage and implement opt-in login/status/logout; keep browser login as fallback and never print credentials.
- [ ] Handle revocation, expired token and disconnected time-tracking provider; add finite timeout/visible error for legacy localhost login if retained.
**Depends on:** 3.1. **Verification:** token/session tests, native-Windows login failure test, three-user opt-in smoke test, `SQLX_OFFLINE=true cargo test -p toki-tui`. **Likely files:** `toki-tui/src/{main,login,session_store,cli}.rs`, `toki-tui/src/api/client.rs`, README. **Size:** split into storage, client auth and UX tasks before implementation.

**Checkpoint D — Reliability release:** B and C pass; contract compatibility and basic login are proven; token mode may be deferred if its storage/security gate is not met. Windows/Linux/macOS release artifacts and rollback instructions are ready. Version/tag only after approval.

## Phase 4 — Separate feature/cleanup release

### 4.1 Add a read-only Aven note source alongside Taskwarrior
- [ ] Agree on workspace/project selection and whether Kleer notes contain title only or a local task reference. Implement bounded CLI output with missing/malformed-output messages and off-loop execution.
- [ ] Preserve current Taskwarrior behavior and keep both optional; no Aven writes or requirement for colleagues to install it.
**Depends on:** checkpoint D, note-format decision. **Verification:** picker tests for both sources and absent executable, manual tests in/outside a mapped repo, `SQLX_OFFLINE=true cargo test -p toki-tui`. **Likely files:** `toki-tui/src/app/mod.rs`, `toki-tui/src/runtime/views/edit_description.rs`, `toki-tui/src/ui/description_editor.rs`, config/tests. **Size:** split CLI adapter and picker UI into two slices.

### 4.2 Assess and retire Git note insertion only if unused
- [ ] Ask all three users whether branch/commit insertion or changing directory is used. If not, remove those controls/config and associated code with tests and release notes.
**Depends on:** checkpoint D, user confirmation. **Verification:** note editing remains functional; `SQLX_OFFLINE=true cargo test -p toki-tui`. **Likely files:** `toki-tui/src/{git,config}.rs`, app/editor UI/tests. **Size:** M.

### 4.3 Decide future of log notes without losing old data
- [ ] Inventory linked log tags/files, define access/export for existing logs, then separately remove new-log creation/editor UI if approved. Reassess the recorded native-Windows external-editor defect in light of the decision.
**Depends on:** checkpoint D, user confirmation and retention plan. **Verification:** old tagged notes still display correctly and retained logs are accessible; no silent orphaning. **Likely files:** `toki-tui/src/{log_notes,editor}.rs`, app/runtime/UI/tests/README. **Size:** split retention support from removal.

### 4.4 Triage remaining low-severity bugs
- [ ] After the reliability release, reproduce/fix the running-entry empty-row indexing bug if it is still relevant, then choose among date editing, clearing fields, Delete key support and stats sorting using user impact—not original low-priority labels alone. The user explicitly skipped the empty-row bug for the next release; the existing backlog task remains open.
**Depends on:** checkpoint D. **Verification:** one regression test per fixed bug, Windows/manual UI checks as applicable, `SQLX_OFFLINE=true cargo test -p toki-tui`. **Likely files:** app history/edit and UI tests. **Size:** one small task per chosen bug; do not bundle them.

**Checkpoint E — Feature release:** Taskwarrior still works, optional Aven works, existing note/log data is safe, and all removed controls are documented. Run cross-platform release builds and user smoke tests.
