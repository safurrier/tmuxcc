# AGENTS.md

## What is this?

tmuxcc is a Rust TUI that monitors AI coding agents running in tmux panes. It supports Claude Code, Codex CLI, Gemini CLI, and OpenCode.

## Validation commands

```bash
cargo test --lib          # Run all tests
cargo fmt --all -- --check  # Check formatting
cargo clippy -- -D warnings  # Lint (requires mise-managed toolchain)
mise run check            # Build + test + clippy
mise run install          # Build release + install to ~/.local/bin
```

## Directory map

| Directory | Purpose |
|-----------|---------|
| `src/app/` | AppState, Action enum, Config. Core state management. |
| `src/ui/` | Main event loop (`app.rs`), layout, and all ratatui components. |
| `src/ui/components/` | Individual widgets: agent tree, footer, input, preview, PR panels. |
| `src/tmux/` | TmuxClient (subprocess calls to tmux), PaneInfo parsing. |
| `src/git/` | GitHub PR integration: GhClient, PrMonitorTask, PR types. |
| `src/parsers/` | Agent-specific output parsers (claude_code, codex_cli, gemini_cli, opencode). |
| `src/monitor/` | Background polling task that scans tmux panes. |
| `src/logging.rs` | Always-on file logging to `~/.local/state/tmuxcc/`. |

## Common pitfalls

- **PR monitor rate limits**: The monitor debounces and backs off, but if you see rate limit errors in logs, increase `pr_poll_interval_ms` in config.
- **Popup mode**: `--popup` flag changes behavior (auto-quit on focus). Don't set it when running standalone.
- **macOS code signing**: After `cargo build`, the binary may need `codesign -s -` to avoid SIGKILL in tmux popups.
- **Cargo.lock v4**: Requires recent Rust. Use `mise` to manage the toolchain.
