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

## Pending checkpoint A work

1. Extend the local stub for committed/lost response, explicit failure and slow history refresh; convert the expected behavior into a failing regression test before modifying the event loop.
2. Ask for an approved staging/test account before authenticated reads or writes; establish whether the deployed server preserves project/activity edits made immediately before save.
3. Obtain native-Windows build/reproduction: per-process samples for TUI vs terminal, using an early stop limit. Do not run an unattended multi-hour, multi-GB test.
4. Decide how to address the pre-existing strict Clippy failures as a separate small baseline cleanup before treating strict lint as a gate.
