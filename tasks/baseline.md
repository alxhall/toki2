# TUI 0.4.0 baseline — 2026-09-25

Worktree: `fix/tui-reliability`, created from `release/v0.4.0` at `447891dddce2dac2c12216f8a12488107d70c1eb`. The original `master` checkout and the release tag were not changed. This is **read-only contract evidence**, not a successful authenticated end-to-end test.

## Released artifact and local build

- Fork release `v0.4.0` is tagged at `447891d`, has Linux (x86_64/aarch64), macOS (Intel/arm) and Windows binaries, and its TUI Release Actions run `25667297196` completed successfully. Release URL: https://github.com/alxhall/toki2/releases/tag/v0.4.0 . Do not assume local source builds are byte-for-byte identical to those artifacts.
- Linux baseline: `SQLX_OFFLINE=true cargo test -p toki-tui` — **50 passed**, 0 failed, 3 unused-import warnings in `toki-tui/src/ui/mod.rs` tests. `SQLX_OFFLINE=true cargo build -p toki-tui --release` — passed. The tagged source has `toki-tui` version 0.4.0 in its manifest but 0.3.0 in `Cargo.lock`; Cargo updated one lockfile line during this run. Keep this lockfile correction with the next fork-owned release. No native Windows build or manual TUI flow has been run in this environment.
- `SQLX_OFFLINE=true cargo clippy -p toki-tui --all-targets -- -D warnings` — **fails on unchanged 0.4.0**: three unused-import groups in `src/ui/mod.rs` tests, and `clippy::too_many_arguments` in `src/api/client.rs:293` (`edit_time_entry`). `cargo fmt --all -- --check` also reports two formatting differences in untouched regions of `src/runtime/actions.rs`. Do not claim a clean strict-lint/format baseline or hide these diagnostics with new suppressions.

## Public deployed contract (unauthenticated GET only)

`GET https://toki-api.spinit.se/openapi.json` returned HTTP 200, OpenAPI **3.1.0**, API document version **1.1.0**, bearer security, and 15 published paths. This does not authenticate the 0.4.0 cookie/session flow and does not prove that all users are connected to Kleer. `/me` is not in this curated catalog; omission does not imply that the route cannot accept bearer auth. No production write or user-specific read was attempted.

| Operation | 0.4.0 behavior | Deployed schema / upstream handler | Consequence / proof still needed |
| --- | --- | --- | --- |
| `GET /time-tracking/timer` | Deserialize optional active timer. | Published with `GetTimerResponse`. | Shape appears compatible; authenticated fixture still needed. |
| `GET /time-tracking/time-info` | Deserialize `WeeklyStats`. | Published `WeeklyStatsResponse` requires worked, scheduled, remaining, absence, covered, period flex fields. | Shape appears compatible; authenticated fixture still needed. |
| `PUT /time-tracking/timer` | Sends `userNote`, `projectId`, `projectName`, `activityId`, `activityName`; discards response body. | Published `SaveTimerPayload` has only `userNote` and optional `restartTimer`, response has `entry` and optional `timer`. Upstream service reads project/activity from the **server-side active timer**. | Extra project/activity fields do not select the saved project/activity on the current handler. Verify immediately changed local fields have been confirmed by `PUT /time-tracking/update-timer` *before* save; lost response is ambiguous. Do not auto-retry. |
| Save and continue | Sends save then a separate start request; optimistic new timer appears first. | Save payload supports optional `restartTimer`, but handler saves before starting replacement. | Even one combined request might commit the entry and then fail starting the next timer. Treat a failed/lost response as potentially committed. Compare both approaches using an approved test server. |
| Auth/session | `toki.sid` cookie, browser OAuth login. | OpenAPI documents bearer security for curated operations; server authentication is composed separately. | Browser session compatibility, `/me` and bearer token behavior require an authenticated, approved test; never add a real token to fixtures. |
| History refresh | Awaits the HTTP refresh inside `handle_save_timer_with_action`, called by `run_action(...).await` in the event loop. | `GET /time-tracking/time-entries` published. | A slow save **or refresh** can stop input/render; local delayed-response reproduction next. |

### Important ambiguity

The upstream `save_timer` service creates the provider entry **before** marking the active timer finished in the repository. Timeout, 500, or loss of the HTTP response cannot safely be treated as proof that no entry was created. The deployed OpenAPI document is not itself evidence of a transaction boundary or idempotency guarantee.

## First verified contract slice

Test-first change `d827326` removes unsupported project/activity fields from the save request and adds a serialization assertion. The new test first failed on the five-field 0.4.0 request, then passed after correction; the whole TUI suite now passes **51 tests**. This makes the request honest but does **not** fix the freeze or guarantee that an unsynced local project/activity edit is saved: those remain in checkpoint A/phase 1.

## Local slow-save reproduction

`python3 tasks/repro_slow_save.py target/release/toki-tui` uses a loopback HTTP stub, a throwaway config/session directory, and a draining PTY; it never contacts production or stores terminal output. With the original 0.4.0 release build, after the stub received `PUT /time-tracking/timer` and held the response, sending `q` **did not exit within 0.5 s**. Releasing the response let the TUI exit normally (status 0). This confirms an event-loop input stall during a slow save on Linux; it does **not** explain which Windows process held 25 GB or prove save idempotency. The script is a manual reproduction fixture, not yet a pass/fail CI regression test.

The optional `--require-responsive` switch turns the PTY harness into an expected-to-fail regression check on 0.4.0: it exited nonzero with `TUI ignored quit input while save was pending`. It is not wired into CI until the off-loop behavior is implemented. For a safe native-Windows starting point, `tasks/windows-memory-sample.ps1` samples the TUI, Windows Terminal, OpenConsole, conhost and wslhost separately every 10 s for up to 15 min; it stops sampling on ≥1 GiB growth or ≥2 GiB private memory and warns the tester to close the TUI manually. Linux cannot validate the PowerShell script or reproduce native Windows behavior.

## Test-environment decision

The repository's `.docs/kleer.md` **does** document Kleer's isolated `https://test-api.kleer.se/v1`: writes there do not appear in the normal `my.kleer.se` admin UI, even for matching company/user IDs. A local Toki API can select this through `TOKI_KLEER__BASE_URL`, but it also needs a local database, server-side Kleer service-account configuration, OAuth and a local user mapping. I found no ready-to-use Toki staging URL or disposable account in this repo. Therefore no production write is necessary for the initial local stub and contract work. If a local end-to-end environment proves impractical, agree on a specific disposable production account/project and cleanup with the user before any live mutation; **do not extract or print credentials**.

## Pending checkpoint A work

1. Extend the local stub for committed/lost response, explicit failure and slow history refresh; keep the responsiveness assertion red until the event-loop change makes it green.
2. Establish an authenticated test environment and verify project/activity sync-before-save and response semantics without unapproved production writes.
3. Have the native-Windows tester run a short idle/running-timer sample and send process-level readings; do not run an unattended multi-hour, multi-GB test.
4. Decide how to address the pre-existing strict Clippy failures as a separate small baseline cleanup before treating strict lint as a gate.
