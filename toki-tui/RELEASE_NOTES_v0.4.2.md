## Toki Timer TUI 0.4.2

- Show the running TUI version in gray beside the title when the terminal is wide enough.
- Choose one note-editor task picker in `config.toml`: `task_manager = "aven"` enables `Ctrl+A`, `"taskwarrior"` enables the existing `Ctrl+T`, and the default `"none"` hides/disables both. **Existing Taskwarrior users must set `task_manager = "taskwarrior"` to keep its shortcut.** Browser sign-in is unchanged.
- The Aven picker reads open tasks across the active workspace, displays `[REF] Title`, but inserts **only the title** into the note. It does not change or sync tasks, and Aven is not required for time tracking.
- Corrected the stopped-timer hint: `Space` starts a timer; `Ctrl+K` is not a start shortcut. Personal-token login is **not** included: the deployed API currently rejects bearer tokens on `/me`, which the TUI uses for account-aware save recovery.

The uncertain-save guard introduced in 0.4.1 remains in effect. If a save outcome is unknown, do **not** retry automatically: review the server state and use the in-app `r` review or `toki-tui resolve-save` as documented in the README. The previously reported hours-long Windows timer crash has **not** been reproduced or declared fixed. An earlier rapid Windows Terminal memory-growth case was addressed in 0.4.1; short tests do not establish multi-hour stability.
