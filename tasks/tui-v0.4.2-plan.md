# Toki TUI v0.4.2 scope (draft)

Base: published `v0.4.1` (`de96ad4e`); worktree `feat/tui-aven-version`. No backend changes, production writes, tag or release publication during preparation.

## Capabilities

1. **Header:** Show the running binary's Cargo version in muted gray immediately after “Toki Timer TUI”; give existing stats priority on narrow terminals. This has no auth or API dependency.
2. **Configured note task picker:** Add `task_manager = "none" | "aven" | "taskwarrior"` to `config.toml`; `none` is the default and hides/disables both picker shortcuts. When set to `taskwarrior`, preserve existing Ctrl+T functionality; when set to `aven`, show Ctrl+A. The Aven list displays `[REF] Title` but appends only the title to the note. Read all *open* (inbox, backlog, todo, active, including blocked) tasks from the active Aven workspace, across projects, using `aven list --open --json` in the TUI's selected working directory. Never invoke Aven writes/sync. Execute the CLI off the render/input loop; bound execution time and output size without silently presenting a truncated list. Absent CLI, invalid output, no workspace, or an oversized result must show a safe error and leave time tracking usable. Do not expose task JSON, stderr, or private titles in diagnostics.
3. **Release checks:** Correct the unsupported Ctrl+K stopped-timer hint; preserve browser login, configured Taskwarrior, and uncertain-save recovery; run focused parser/UI/CLI fault tests, TUI tests, offline PTY suite and cross-platform release candidate CI. Bump to 0.4.2 and prepare notes only after behavior is verified. Tag/publish only with separate approval.

## Explicitly deferred

Token login. A read-only bearer probe returned 200 for the needed time-tracking GET routes but 401 for `/me`. The existing TUI needs its same-account identity for safe save recovery. The backend cannot be changed for this release, so do not guess an identity, bypass the guard, or silently substitute token auth. Git/log note removal, empty-row bug, and the unverified hours-long crash are also outside this scope.

## Acceptance

- Header shows `v0.4.2` in muted style where space permits; narrow layouts do not lose stats because of the addition.
- With `task_manager = "aven"`, the note editor displays refs alongside open task titles across projects, allows selection/cancel, and appends only the selected title. With `"taskwarrior"`, Ctrl+T works as before. With default `"none"`, neither shortcut appears or runs; no Aven mutation, network save, or unsolicited CLI execution.
- With Aven absent, stalled, malformed, or oversized, the editor remains responsive and the current note remains unchanged. The stopped-timer status advertises only `Space`, not the unsupported Ctrl+K.
- Existing recovery and redraw checks remain green; no claim about the hours-long Windows crash. Native Windows smoke test and release approval remain outstanding.

## Local candidate evidence and outstanding gates

- Versioned binary reports `0.4.2`. `cargo fmt -p toki-tui --check`, `SQLX_OFFLINE=true cargo test -p toki-tui --locked` (87 tests), and a strict TUI Clippy run excluding the existing `client.rs:293` `too_many_arguments` warning pass.
- Seven offline save/recovery PTY modes, the idle/running/resize redraw check, and a fake-CLI Aven picker PTY test pass locally. After the follow-up requests, the fake picker shows references while the note contains only a title, the default task manager hides/disables both shortcuts, and the stopped-timer hint no longer mentions Ctrl+K. The Aven test uses dev mode, throwaway config, and an executable that only prints fixture JSON. An installed read-only `aven list --open --json` check returned a JSON task array across multiple projects; no real task titles were displayed.
- Independent review found a missing version bump and excessive potential redraw work for large Aven lists. Both were fixed: `Cargo.toml`/`Cargo.lock` now declare 0.4.2, and lists over 500 tasks produce an explicit error rather than truncating silently.
- Still needed before any tag/publish: release-branch cross-platform CI, native-Windows hands-on header/picker/Taskwarrior and ordinary timer smoke test, review of the final diff, and explicit release approval. No production write or backend change is part of this candidate.
