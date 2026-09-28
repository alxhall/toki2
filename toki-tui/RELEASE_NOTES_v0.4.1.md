# Toki TUI v0.4.1

Reliability update to v0.4.0.

## Improvements

- Saving and history refresh no longer block TUI input while waiting for the API. Duplicate save keys do not send duplicate requests.
- A durable, local guard prevents replaying an uncertain save after a timeout, error or restart. Press `r` for an in-app, read-only review of the server timer and recent entries. After independently verifying the outcome, press `c`, then `y` to clear **only** the local guard; quit and relaunch afterward. The `resolve-save` command remains a fallback. The TUI never retries an uncertain save automatically.
- Saves attempted before a timer reaches 60 seconds are declined without contacting the server, leaving the timer and note intact. This is a conservative TUI rule; the provider's exact minimum has not been verified.
- The save request now matches the current API contract (`userNote` only). Select and verify the project/activity on the active timer before saving.
- Unchanged idle screens no longer redraw continuously; running clocks refresh about once a second. In short, visible native-Windows comparisons, this eliminated the rapid Windows Terminal memory growth reproduced with v0.4.0.

## Verification and limitations

Release CI runs TUI tests, seven offline save/recovery scenarios, a redraw regression and builds for Windows, Linux and macOS. One normal save-and-stop and one normal same-project save-and-continue were checked against the web app by a Windows tester, each yielding one intended entry; the latter restarted the timer on the same project/activity. No deliberate response loss was tested on production.

The separately reported **hours-long timer crash remains unverified** and must not be described as fixed. Recent server history may be incomplete or ambiguous after a lost response; if you cannot establish whether an entry was saved, leave the guard in place and do not retry. Token authentication, Aven support, feature removal and the low-priority empty-row issue are deferred.

## Rollback safety

Keep a copy of v0.4.0 for rollback. If v0.4.1 shows an uncertain save, first check the server/web app and use its recovery flow; **do not use v0.4.0 to retry while a pending guard exists**, because v0.4.0 cannot recognize that guard and could duplicate a committed entry. Stop any test if terminal memory rises unexpectedly rather than allowing multi-GB growth. Once no save remains unresolved, the previous binary can be used while investigating a regression.
