# toki-tui Agent Guide

## Overview

`toki-tui` is a standalone terminal UI for Toki time tracking, built with [Ratatui](https://ratatui.rs). It lives as a crate inside the `toki2` monorepo but is independently versioned and released. It communicates with `toki-api` over HTTP and authenticates via browser OAuth, storing the session locally.

This file documents conventions for agents working inside `toki-tui/`.

## Tech Stack

- **Rust** (2021 edition), async via **Tokio**
- **Ratatui** + **crossterm** for terminal rendering
- **reqwest** for HTTP (cookies-based auth against toki-api)
- **clap** for CLI subcommands, **config** crate for configuration
- **anyhow** for error handling, **time** crate for date/time
- No database — all state is in-memory or persisted via toki-api

## Project Structure

```
toki-tui/src/
├── main.rs               # Entry point
├── cli.rs                # CLI subcommand definitions (clap)
├── config.rs             # Config loading (~/.config/toki-tui/config.toml)
├── bootstrap.rs          # App bootstrap / startup sequence
├── terminal.rs           # Terminal setup/teardown
├── session_store.rs      # OAuth session persistence
├── login.rs              # OAuth login flow
├── git.rs                # Git branch → note conversion
├── time_utils.rs         # Date/time helpers
├── types.rs              # Shared domain types
├── api/                  # HTTP client and DTOs
│   ├── client.rs         # ApiClient (reqwest wrapper)
│   ├── dev_backend.rs    # In-memory mock backend
│   └── dto.rs            # API data transfer objects
├── app/                  # Application state machine
│   ├── state.rs          # AppState
│   ├── navigation.rs     # Focus/navigation logic
│   ├── edit.rs           # Text editing helpers
│   └── history.rs        # History view state
├── runtime/              # Event loop and action dispatch
│   ├── event_loop.rs     # Main event loop
│   ├── actions.rs        # Action enum
│   ├── action_queue.rs   # Queued action processing
│   └── views/            # Per-view update handlers
└── ui/                   # Ratatui rendering
    └── ...               # Widget rendering functions
```

## Version Control

Use **git** (not jj). **Never push commits** — the user pushes and tags manually.

Use [Conventional Commits](https://www.conventionalcommits.org/) with the `tui` scope:

```
feat(tui): add word-navigation in note editor
fix(tui): correct cursor rendering for multibyte chars
refactor(tui): eliminate dead code in stop_timer
test(tui): add regression tests for multibyte cursor
chore(tui): bump version to 0.3.2
ci(tui): gate release builds on tests
```

## Branch Strategy

| Branch | Purpose |
|--------|---------|
| `master` | Mirrors `upstream/master` (`ponbac/toki2`). Synced via fast-forward. Do not add toki-tui–only commits here. |
| `feat/*`, `fix/*` | Feature and fix branches. Cut from `upstream/master`. Opened as PRs against `origin/master`. |
| `origin/fork/base` | Fork-specific integration base. Contains fork-only additions (release CI workflow, etc.). Periodically rebased on top of `upstream/master`. |
| `release/vX.Y[.Z]` | Release branches. Cut from `origin/fork/base`. Stabilisation work and version bump happen here. |

### Workflow summary

```
upstream/master  ──────────────────────────────────────────►
                      │                    ▲
                      │  (rebase)          │ (sync)
                      ▼                    │
              origin/fork/base         master
                      │
                      │  (cut release branch)
                      ▼
              release/vX.Y.Z
                      │
                      │  (bump version, tag)
                      ▼
                  vX.Y.Z  ◄── tag here
```

**Feature work:**
```bash
git fetch upstream
git checkout -b feat/tui-my-feature upstream/master
# ... work ...
# Open PR → origin/master
```

**Starting a release branch:**
```bash
git fetch origin
git checkout -b release/vX.Y.Z origin/fork/base
# Bump version in toki-tui/Cargo.toml
# Cherry-pick or merge feature branches as needed
# Tag when stable: git tag vX.Y.Z
```

**Keeping fork/base current:**
```bash
git fetch upstream
git rebase upstream/master <fork-base-branch>
```

### Tags

- Stable releases: `vX.Y.Z`
- Release candidates: `vX.Y.Z-rc.N`

Tags are created by the user manually after stabilisation.

## Verifying Changes

Always verify before declaring work complete. Run from the repo root or `toki-tui/`:

```bash
# From repo root
just check         # cargo check (whole workspace)
just clippy        # cargo clippy (whole workspace)

# Scoped to toki-tui only (faster)
cd toki-tui
cargo check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

Clippy warnings are treated as **errors** in CI (`-D warnings`). Fix all warnings before finishing.

`SQLX_OFFLINE=true` is **not** required for toki-tui (no SQL queries).

## Running the TUI

```bash
just tui-login   # Authenticate (first time or after session expires)
just tui         # Run against the real toki-api
just tui-config  # Print config path, create default config if missing
just tui-status  # Show session and Milltime status
```

Config file: `~/.config/toki-tui/config.toml`
Environment variable prefix: `TOKI_TUI_` (e.g. `TOKI_TUI_API_URL`)

## Development Approach

**Prefer subagent-driven development within the same session.** When a task has independent subtasks (e.g. implement a feature and write tests for it in parallel, or fix two unrelated bugs), dispatch parallel subagents rather than working sequentially. This is the preferred workflow.

## Important Notes

1. **Never push** — all pushes and tag creation are done manually by the user
2. **Clippy is strict** — `-D warnings` in CI; zero warnings expected
3. **No database** — toki-tui has no SQL; SQLX_OFFLINE is irrelevant
4. **Config location** — `~/.config/toki-tui/config.toml`; missing file is fine (built-in defaults apply)
5. **Minimal test coverage** — tests exist for critical logic (e.g. multibyte cursor); be careful with refactors
6. **Conventional commits with `tui` scope** — keeps toki-tui commits identifiable in the shared monorepo history
