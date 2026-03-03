# toki-tui

Terminal UI for Toki time tracking, built with [Ratatui](https://ratatui.rs).

## Running

```bash
# Authenticate (first time or after session expires)
just tui-login

# Run against the real toki-api
just tui

# Run in dev mode (no server needed, fake in-memory data)
just tui-dev

# Clear saved session
just tui-logout

# Print config path and create default config if missing
just tui-config
```

## Configuration

Config file: `~/.config/toki-tui/config.toml`

Run `just tui-config` (or `cargo run -- config-path`) to print the path and create the file with defaults if it does not exist.

All keys are optional. If the file is missing, built-in defaults are used.

### Environment variables

You can override config values with environment variables.

- Prefix: `TOKI_TUI_`
- Key format: uppercase snake case
- Nested keys (if added later): use `__` as separator

Current variables:

```bash
TOKI_TUI_API_URL="http://localhost:8080"
TOKI_TUI_GIT_DEFAULT_PREFIX="Development"
TOKI_TUI_TASK_FILTER="+work project:Toki"
```

Environment variables override values from `config.toml`.

```toml
# URL of the toki-api server. Defaults to the production instance.
api_url = "https://toki-api.spinit.se"

# Prefix used when converting a git branch name to a time entry note,
# when no conventional commit prefix (feat/fix/etc.) or ticket number is found.
# Example: branch "branding/redesign" → "Utveckling: branding/redesign"
git_default_prefix = "Utveckling"

# Taskwarrior filter tokens prepended before `status:pending export`.
# Leave empty to show all pending tasks.
# Example: "+work project:Toki"
task_filter = ""
```

### Example: local dev setup

```toml
api_url = "http://localhost:8080"
git_default_prefix = "Development"
task_filter = "+work"
```

## Releasing

### Branch strategy

```
upstream/master → master (sync only, never tagged)
feature/* / fix/* → next  (integration, disposable)
next → release/v0.3       (stable snapshot, tags live here)
```

- `master` tracks upstream. Never tag it, never develop on it.
- `next` is the integration branch — rebase and force-push freely.
- `release/vMAJOR.MINOR` is a stable snapshot cut from `next`. Tags (`v0.3.0`, `v0.3.1`, ...) go here.

### New minor release (e.g. v0.3.0)

```bash
git checkout next
git checkout -b release/v0.3
git tag v0.3.0
git push origin release/v0.3 --tags
```

### Patch release (e.g. v0.3.1)

```bash
git checkout release/v0.3
# cherry-pick fixes from next if needed
git tag v0.3.1
git push origin release/v0.3 --tags
```

GitHub Actions builds binaries for all 5 platforms and creates a GitHub Release automatically when a `v*` tag is pushed.

> Never tag `next` or `master`. Tags only go on `release/v*` branches.

## Standard key bindings

| Key            | Action             |
| -------------- | ------------------ |
| `Space`          | Start / stop timer |
| `Ctrl+S`         | Save (options)     |
| `Ctrl+X`         | Clear              |
| `Tab / ↑↓ / j/k` | Navigate           |
| `H`              | History view       |
| `P`              | Project            |
| `N`              | Note               |
| `T`              | Toggle timer size  |
| `S`              | Stats              |
| `Esc`            | Exit / cancel      |
| `Q`              | Quit               |
