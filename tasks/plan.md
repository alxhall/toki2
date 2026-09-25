# Toki TUI recovery and next-release plan

**Status:** approved for execution; baseline underway (see [`tasks/baseline.md`](baseline.md)); no save behavior changed yet. **Implementation base:** `release/v0.4.0` (not the current checkout's `master`). **Delivery:** fork-owned TUI releases; no upstream TUI PRs required. **Audience:** the three daily TUI users, including a native Windows tester. **Task details:** [`tasks/todo.md`](todo.md). The existing TUI bugs are recorded in the local Aven project; this plan does not create, reprioritize, or close Aven tasks.

## Goal and boundaries

Make it safe and pleasant to record time every day: the UI stays responsive during slow operations, save outcomes are trustworthy, and native Windows memory use is bounded. After that, simplify authentication and the note workflow. Preserve working Taskwarrior support when adding optional Aven support. Do not turn the TUI into a general-purpose workflow dashboard.

**Not doing in the reliability release:** Aven integration, removal of Git/log notes, a wholesale TUI rewrite, automatic retries of uncertain writes, or requiring API tokens. Do not change the backend merely to paper over an unmeasured client problem.

## Evidence and assumptions to test

- `release/v0.4.0` has Kleer migration, `toki.sid`, optimistic timer mutations, and the release build workflow. Local `master` is 30 commits behind `upstream/master`; the release branch has 25 additional commits beyond local `master`. The earlier branch convention, documented on `release/v0.3.2` in `toki-tui/AGENTS.md`, was `upstream/master` → `fork/base` (fork-only release CI) → `release/vX.Y.Z`. **0.4.0 diverged from that convention:** it does not descend from `fork/base`; its final commit re-added `.github/workflows/tui-release.yml` directly. `fork/base` still points to old commits. The user has since separated TUI release work from upstream PRs, so the old branch guide is historical rather than the process to reinstate. Start in an isolated worktree from the released 0.4.0 code, preserve its workflow, and integrate **selected** upstream API changes; do not merge the stale base into it blindly.
- In 0.4.0, `toki-tui/src/runtime/event_loop.rs` calls `run_action(...).await` in the input/render loop. Saving also awaits history refresh (`runtime/actions.rs`). This explains an apparent freeze on slow I/O; it does **not** establish the root cause of Windows memory growth.
- Upstream's `PUT /time-tracking/timer` returns a saved entry and optionally a replacement timer. The 0.4.0 client discards that response and sends project/activity fields on save, while current upstream's save payload uses `userNote` and optional `restartTimer`, taking project/activity from the *server-side active timer*. Confirm actual deployed-server behavior before relying on any field, especially edits made just before saving.
- Upstream exposes `/openapi.json` and bearer tokens; its published OpenAPI operations are curated documentation, **not** token permissions. Confirm deployment and mapping requirements with a disposable personal token before designing token login.
- Aven reports a native-Windows long-running-timer crash and login/reauthentication bugs. A colleague reports that a **Windows Terminal tab** reached ~25 GB; after restarting, it reportedly grew by ~7 GB in one hour while the TUI did not respond to input, then stopped growing once input resumed. This is a severe, repeatable-looking symptom, but the report does not identify whether the memory belongs to `toki-tui.exe`, Windows Terminal, a child process, or a tab-associated process tree. Nor does it establish that the long-timer crash shares a cause.

## Dependency graph and delivery checkpoints

```text
0.1 baseline/branch inventory ── 0.2 deployed API contract + fixture ── 0.3 reproduce slow save
                                       │                              │
                                       └───────── 1.1 single-flight UI state ── 1.2 off-loop I/O
                                                                       │                 │
                                                                       └──── 1.3 reconcile uncertain saves
                                                                                         │
                                                                                   1.4 restart/refresh
                                                                                         │
                                                                                  checkpoint: save

0.1 ── 2.1 Windows measurement ── 2.2 profile/fix only measured growth ── checkpoint: Windows

0.2 + save checkpoint ── 3.1 bearer/token feasibility ── 3.2 optional auth + storage

save + Windows checkpoints ── reliability release
reliability release ── 4.1 Aven picker ── 4.2 optional Git/log retirement ── next feature release
```

### Phase 0 — Establish the correct baseline (tasks 0.1–0.3)

Work from an isolated release-branch worktree; do not overwrite the existing `master` checkout. Record the tagged/released binary versus source revision, build on Linux and Windows, run `SQLX_OFFLINE=true cargo test -p toki-tui`, and test the user flow against the deployed API **without writing to production test data unless explicitly approved**. Inspect the deployed `/openapi.json` if available and compare its authentication, endpoints and JSON bodies with 0.4.0. If the deployed spec is absent/older, capture sanitized contract fixtures from an approved staging/test instance instead. Map server-save behavior, including changes to project/activity/note, locked periods, save-and-continue, and what happens when the provider succeeds but the API response is delayed/lost. Prefer a local stub for fault injection rather than inducing failures in live Kleer.

**Gate:** write down a small compatibility matrix (released TUI × deployed API × current upstream API), representative sanitized response fixtures, and a reproducible slow-save script. If the deployed API is incompatible, fix the smallest contract break first; do not attribute an auth or schema failure to a performance problem.

### Phase 1 — Save without freezing or duplicating entries (tasks 1.1–1.4)

Model save as **idle → saving → confirmed / uncertain / failed**. **Before the network write**, persist minimal protected pending metadata (no note text, cookie or token) so crash/quit can be recognized on relaunch; if persisting fails, refuse to send. Establish the single-flight guard synchronously, reject duplicate saves and serialize writes affecting the same timer (save, edit, stop, restart); reads must not overwrite newer local state. Run I/O without holding the UI event loop or terminal hostage. Keep a bounded queue/coalesced polling so a slow provider does not accumulate background work. Render and relevant navigation must stay usable; quitting while a write is in flight must either wait with a visible explanation or leave a durable pending marker to reconcile on next launch. Use explicit deadlines for individual requests, not a hidden infinite wait. Do not ship a responsiveness-only fix that treats timeouts/500s as proof the entry was not created.

On success, use the server-confirmed response and refresh history asynchronously. The local pending record is only a crash/retry guard; **the API/server remains authoritative for whether an entry exists**, with no new client-to-API protocol required for this first slice. On timeout/disconnect, **never issue a second save automatically**: first fetch active timer and recent entries and compare the expected timer/time window and returned registration ID when available. If reconciliation cannot decide (for example offline), display “outcome unknown”, retain enough non-secret local context to reconcile on reconnect/restart, and disable unsafe retry until the user resolves it. Distinguish “entry saved, restart failed” from “save failed”; evaluate upstream's `restartTimer` contract before choosing one request over two. Avoid optimistic resets that discard the previous note/project or fabricate a new start time on rollback.

**Gate:** with a stubbed 10-second save and a separate 10-second history refresh, redraw/keypress feedback remains perceptibly prompt (target <250 ms for local UI-only handling); repeated save keys produce one write. Test success, explicit failure before write, server-committed/lost-response, refresh failure, restart failure and application restart. No duplicate entry or silent loss in these cases. If the server does not expose enough state to distinguish committed from uncommitted writes, retain the honest “unknown” state and make an idempotency-key/lookup backend change a separate decision, not a guess.

### Phase 2 — Measure and address native Windows resource growth (tasks 2.1–2.2)

First ask whether the reported tab was a native executable, WSL, or a shell launching other processes, and whether the stalled period followed save, refresh, login, or idle operation. On native Windows, run the **actual release artifact** and a diagnostic build under a controlled terminal/account/server or stub. Sample **separate process IDs** for `toki-tui.exe`, `WindowsTerminal.exe`/terminal renderer and any child processes: private bytes, working set, handles, CPU, and timestamps at startup and every 5–10 seconds through the first hour, including the onset and end of input unresponsiveness. Compare idle, running timer, slow save, slow refresh and terminal choice; only extend to multi-hour runs after short runs demonstrate bounded usage. **Stop/capture diagnostics at a conservative safety threshold well below 7 GB** rather than reproducing 25 GB and impairing a work machine. Record versions, build flags, environment, workload and whether growth stops when input resumes or a process is terminated. Compare with Linux as a control. Capture a heap/profile or allocation evidence **before** choosing a fix; separately test whether the recorded long-running-timer crash correlates. Agree on a numeric release memory budget after establishing a stable baseline.

**Gate:** either demonstrably bounded memory on a repeatable Windows run plus a regression guard, or a documented blocker with evidence and an explicit user decision not to release. “It did not crash on Linux” is not a pass.

### Phase 3 — Optional personal-token authentication (tasks 3.1–3.2; after reliability)

Verify the deployed API accepts bearer authentication on all TUI endpoints and the Toki user is mapped to Kleer. Verify token issuance/revocation, 401 vs disconnected-provider behavior, and whether the current `/me` route works with tokens. A token is shown once and has broader authority than the OpenAPI catalog implies. Choose a secure OS credential store on supported platforms; no plaintext config, CLI argument, debug log, or telemetry containing the secret. Keep browser login as fallback initially. If token mode proves reliable, the native-Windows two-port login bug and session expiry can be addressed by deprecating the old path later—not by silently deleting it. Independently add a visible bind error and finite timeout for legacy login if it remains.

**Gate:** three-user opt-in trial, revoke/expired-token recovery, existing session login unchanged, no token leaks, and documented setup. A token is an authentication improvement, **not** a fix for slow Kleer requests.

### Phase 4 — Focused note workflow, then cleanup (tasks 4.1–4.3; separate release)

Add Aven as an *optional* source in the note picker while retaining Taskwarrior. Start with read-only lookup: current workspace/project inference, bounded open/ready task list, chosen title inserted into the note, missing-CLI and malformed-output handling. Decide together whether to include the local task reference in notes: notes may be sent to Kleer and references from a local Aven workspace may be inappropriate outside it. Move CLI execution off the render loop. Later, inventory existing Git copy/parse use and linked log-file data; remove Git UI only after users confirm, and keep a way to access/export old linked logs before retiring creation/editing. Re-triage the recorded Windows external-editor issue if that feature is retired. Minor edit/history/stats backlog follows only after these decisions.

**Gate:** Taskwarrior behavior remains intact, Aven absence does not block time tracking, legacy notes remain readable, and removal is communicated to colleagues.

## Release and verification policy

- **Reliability release first** (version decided after checking deployed compatibility and user-visible changes), **feature/cleanup release later**. Releases belong to the user's fork; no upstream TUI PR is a release prerequisite. For the next release, branch from 0.4.0 in an isolated worktree and retain its TUI Actions workflow. Later choose whether a fork-owned `tui/main` (or refreshed integration branch) is worth maintaining; do not rewrite stale `fork/base` or force-push it just to reproduce the old process. Build Windows, Linux and macOS artifacts with the release workflow; the Windows tester and at least one other user perform start/save/continue/edit/restart smoke tests. Do not change the release workflow or publish tags as part of planning.
- Every behavioral slice: focused failing test before implementation, `SQLX_OFFLINE=true cargo test -p toki-tui`, targeted local stub integration test, and a diff review. Where backend contract changes are necessary, add backend tests and run `SQLX_OFFLINE=true just check`/relevant backend tests. Manual native-Windows gates are recorded with measured output.
- Sanitize fixtures, diagnostics and screenshots: no token, session cookie, Kleer personal data or actual time-entry note in the repository. Make an explicit fallback/rollback path: keep the last working 0.4.0 artifact and restore it if a candidate fails a user flow; never automatically replay an uncertain write after rollback.

## Open decisions (do not silently choose)

1. What exact API version is deployed at the configured production URL, and is `/openapi.json` already there? Use an approved test/staging account for any write probe.
2. Does saving with immediately changed project/activity persist the desired values on the deployed server? If not, is a confirmed update of the server-side active timer required before save?
3. Is there enough information in the API to reconcile a timed-out write conclusively? If not, should the backend add idempotency keys or a stronger lookup contract?
4. Which process actually held the reported 25 GB, and was the tab native Windows, WSL, or a shell with children? What preceded the ~7 GB/hour growth, and what caused input to resume? What Windows memory ceiling should the team agree on after measurement?
5. For Aven, should notes contain title only or a human-readable local reference? Which workspace/project should the picker show when launched outside a mapped repository?
6. Who depends on Git note insertion or stored log files, and how will existing logs be retained when editing is removed?

**Approval checkpoint:** Review this plan and the first baseline findings with the user before changing behavior or creating/updating Aven tasks.
