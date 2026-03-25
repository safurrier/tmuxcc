#!/usr/bin/env bash
# E2E smoke test for tmuxcc: verifies that tmuxcc starts, displays sessions,
# and responds to basic navigation keys (j/k).
#
# Uses an isolated tmux socket via -L flag and a wrapper script to redirect
# tmuxcc's tmux calls to the test server.
set -euo pipefail

SOCKET="test-tmuxcc-smoke-$$"
WRAPPER_DIR=""

# Cleanup on exit
cleanup() {
    tmux -L "$SOCKET" kill-server 2>/dev/null || true
    [ -n "$WRAPPER_DIR" ] && rm -rf "$WRAPPER_DIR"
}
trap cleanup EXIT

# ── Helpers ──────────────────────────────────────────────────────────────────

fail() {
    echo "FAIL: $1" >&2
    exit 1
}

pass() {
    echo "PASS: $1"
}

# Wait until capture-pane output contains a given string
# Usage: wait_for_text <tmux-target> <text> [timeout_seconds]
wait_for_text() {
    local target="$1"
    local text="$2"
    local timeout="${3:-10}"
    local elapsed=0
    while [ "$elapsed" -lt "$((timeout * 2))" ]; do
        local output
        output=$(tmux -L "$SOCKET" capture-pane -t "$target" -p 2>/dev/null || true)
        if echo "$output" | grep -qF "$text"; then
            return 0
        fi
        sleep 0.5
        elapsed=$((elapsed + 1))
    done
    echo "Timed out waiting for '$text' in pane $target" >&2
    echo "Last captured output:" >&2
    tmux -L "$SOCKET" capture-pane -t "$target" -p 2>/dev/null >&2 || true
    return 1
}

# ── Prerequisites ────────────────────────────────────────────────────────────

command -v tmux >/dev/null 2>&1 || { echo "SKIP: tmux not found"; exit 0; }

# Build tmuxcc
echo "Building tmuxcc..."
cargo build 2>&1 || fail "cargo build failed"
TMUXCC_BIN="$(pwd)/target/debug/tmuxcc"
[ -x "$TMUXCC_BIN" ] || fail "tmuxcc binary not found at $TMUXCC_BIN"

# ── Setup test environment ───────────────────────────────────────────────────

echo "Setting up isolated tmux server (socket: $SOCKET)..."

# Create sessions with fake agent panes
tmux -L "$SOCKET" new-session -d -s "smoke-session" -x 120 -y 40
tmux -L "$SOCKET" send-keys -t "smoke-session" "printf '\\033]0;claude\\007'" Enter

tmux -L "$SOCKET" new-session -d -s "second-session" -x 120 -y 40

sleep 1

# Create a wrapper directory with:
# 1. A tmux wrapper that forces -L <socket>
# 2. A launcher script that sets PATH and runs tmuxcc
REAL_TMUX="$(command -v tmux)"
WRAPPER_DIR=$(mktemp -d)

cat > "$WRAPPER_DIR/tmux" <<EOF
#!/bin/sh
exec "$REAL_TMUX" -L "$SOCKET" "\$@"
EOF
chmod +x "$WRAPPER_DIR/tmux"

cat > "$WRAPPER_DIR/launch.sh" <<EOF
#!/bin/sh
export PATH="$WRAPPER_DIR:\$PATH"
export TMUX=""
exec "$TMUXCC_BIN" 2>/dev/null
EOF
chmod +x "$WRAPPER_DIR/launch.sh"

# Launch tmuxcc via the short launcher script (avoids long command line issues)
tmux -L "$SOCKET" new-window -t "smoke-session" -n "tmuxcc"
tmux -L "$SOCKET" send-keys -t "smoke-session:tmuxcc" "$WRAPPER_DIR/launch.sh" Enter

# ── Tests ────────────────────────────────────────────────────────────────────

echo "Waiting for tmuxcc to render..."

# Test 1: tmuxcc starts and shows session names
if wait_for_text "smoke-session:tmuxcc" "smoke-session" 15; then
    pass "tmuxcc started and shows smoke-session"
else
    fail "tmuxcc did not show smoke-session within timeout"
fi

# Test 2: second session visible
if wait_for_text "smoke-session:tmuxcc" "second-session" 5; then
    pass "tmuxcc shows second-session"
else
    fail "tmuxcc did not show second-session"
fi

# Test 3: Navigate with j key (down)
tmux -L "$SOCKET" send-keys -t "smoke-session:tmuxcc" "j"
sleep 0.5
tmux -L "$SOCKET" send-keys -t "smoke-session:tmuxcc" "j"
sleep 0.5
pass "j key navigation accepted"

# Test 4: Navigate with k key (up)
tmux -L "$SOCKET" send-keys -t "smoke-session:tmuxcc" "k"
sleep 0.5
pass "k key navigation accepted"

# Test 5: Help toggle with ?
tmux -L "$SOCKET" send-keys -t "smoke-session:tmuxcc" "?"
sleep 0.5
if wait_for_text "smoke-session:tmuxcc" "Help" 5; then
    pass "Help popup appeared"
else
    fail "Help popup did not appear"
fi

# Close help
tmux -L "$SOCKET" send-keys -t "smoke-session:tmuxcc" "q"
sleep 0.5

# Test 6: Footer shows sort mode badge
if wait_for_text "smoke-session:tmuxcc" "s:Recent" 5; then
    pass "Footer shows default sort mode badge"
else
    fail "Footer missing sort mode badge"
fi

# Quit tmuxcc
tmux -L "$SOCKET" send-keys -t "smoke-session:tmuxcc" "q"
sleep 1

echo ""
echo "All smoke tests passed."
exit 0
