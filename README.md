# TmuxCC

**AI Agent Dashboard for tmux** — Monitor and manage multiple AI coding agents from a single terminal interface.

TmuxCC scans your tmux sessions for running AI agents (Claude Code, Codex CLI, Gemini CLI, OpenCode), shows their status in a tree view, and lets you approve requests, jump to panes, and check PR status — all without leaving your terminal.

---

## Quick Start

### 1. Install

```bash
cargo install tmuxcc
```

### 2. Add a tmux keybind

Add this to your `~/.tmux.conf`:

```tmux
bind-key a display-popup -h 80% -w 80% -E "$HOME/.local/bin/tmuxcc --popup"
```

Then reload: `tmux source ~/.tmux.conf`

### 3. Use it

Press `prefix + a` to open the dashboard as a floating popup. From there:

- **Navigate** with `j`/`k` to find your agent
- **Enter** to jump to that agent's tmux pane (closes the popup)
- **Tab** to the input panel and type a message directly to the agent
- **`y`** to approve a pending request, **`N`** to reject
- **`p`** to see PR status, **`o`** to open the PR in your browser, **`c`** to copy the URL
- **Esc** or **`q`** to close

### Without the popup

You can also run `tmuxcc` directly in any terminal pane — it works the same way, just not as a floating overlay.

---

## Features

- **Multi-agent monitoring** — Track agents across all tmux sessions and windows
- **Real-time status** — Idle, Processing, Awaiting Approval, Error at a glance
- **Approval management** — Approve/reject with single keystrokes, batch operations
- **Flash navigation** — Press `g` to show jump labels on every item, press the label to jump instantly
- **GitHub PR integration** — See PR number, CI status dots, review state, and merge readiness inline
- **Popup mode** — Runs as a tmux floating popup with `--popup`, auto-closes on jump
- **Subagent tracking** — Monitor spawned subagents with status and duration
- **Pane preview** — See live content from the selected agent's tmux pane
- **Direct input** — Tab to the input panel and send text directly to an agent

### Supported Agents

| Agent | Detection |
|-------|-----------|
| **Claude Code** | `claude` command, version output, window title |
| **Codex CLI** | `codex` command |
| **Gemini CLI** | `gemini` command |
| **OpenCode** | `opencode` command |

---

## Key Bindings

### Navigation

| Key | Action |
|-----|--------|
| `j` / `k` | Move up/down in the sidebar |
| `Enter` | Jump to pane in tmux (collapse/expand on session headers) |
| `f` / `F` | Focus pane in tmux |
| `/` | Search: type to filter, Up/Down for prev/next match, Enter to go |
| `g` | Flash-focus: show labels, press one to jump cursor |
| `G` | Flash-go: show labels, press one to jump + attach tmux |
| `Tab` | Cycle focus: Sidebar → Preview → Input |
| `Esc` | Quit (or dismiss search/selection first) |
| `q` | Quit |

### Agent Actions

| Key | Action |
|-----|--------|
| `y` / `Y` | Approve pending request(s) |
| `N` | Reject pending request(s) |
| `a` / `A` | Approve ALL pending requests |
| `1`-`9` | Send numbered choice to agent |
| `Space` | Toggle multi-select on current agent |

### Session & Pane Management

| Key | Action |
|-----|--------|
| `[` / `]` | Collapse / expand all sessions |
| `H` | Toggle non-agent sessions (hidden by default) |
| `V` | Toggle non-agent panes like nvim/shells (hidden by default) |
| `n` | Spawn a new agent in the current session |
| `dd` | Kill pane (double-tap for safety) |
| `R` | Rename tmux window |

### View & PR

| Key | Action |
|-----|--------|
| `p` | Toggle PR detail panel |
| `o` | Open PR URL in browser |
| `c` | Copy PR URL to clipboard |
| `s` / `S` | Toggle subagent log panel |
| `t` / `T` | Toggle TODO/Tools summary panel |
| `<` / `>` | Resize sidebar |
| `h` / `?` | Show help overlay |

### Input Panel

| Key | Action |
|-----|--------|
| `Enter` | Send text to selected agent |
| `Shift+Enter` | Insert newline |
| `Esc` | Back to sidebar |

---

## Command Line Options

```
tmuxcc [OPTIONS]

Options:
  -p, --poll-interval <MS>      Polling interval in milliseconds [default: 500]
  -l, --capture-lines <LINES>   Lines to capture from each pane [default: 100]
  -f, --config <FILE>           Path to config file
  -d, --debug                   Enable debug logging (verbose)
      --popup                   Running inside tmux popup (auto-quit on focus)
      --show-config-path        Show config file path and exit
      --init-config             Create default config file and exit
  -h, --help                    Print help
  -V, --version                 Print version
```

---

## Configuration

Config file location: `~/.config/tmuxcc/config.toml` (Linux) or `~/Library/Application Support/tmuxcc/config.toml` (macOS).

```bash
tmuxcc --init-config   # Create default config
tmuxcc --show-config-path  # Show location
```

```toml
poll_interval_ms = 500
capture_lines = 100

# GitHub PR integration (requires `gh` CLI)
pr_enabled = true
pr_poll_interval_ms = 60000
```

---

## Logging

Logs are written to `~/.local/state/tmuxcc/` automatically.

- Default level: `info` (startup, PR polls, focus events)
- `--debug` flag: `debug` level (all tmux commands, path changes)
- `latest.log` symlink always points to the current session's log
- Old logs are cleaned up after 7 days

```bash
# Tail logs in real time
tail -f ~/.local/state/tmuxcc/latest.log
```

---

## Requirements

- **tmux** (must be running)
- **Rust** 1.70+ (for building)
- **gh** CLI (optional, for PR integration — [github.com/cli/cli](https://cli.github.com/))

---

## License

MIT — see [LICENSE](LICENSE).
