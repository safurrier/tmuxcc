#!/usr/bin/env bash
# E2E test for sort modes: verifies that pressing 's' cycles between
# "Recent" and "Status" sort modes, as shown in the footer badge.
#
# Uses an isolated tmux socket via -L flag.
set -euo pipefail

SOCKET="test-tmuxcc-sort-$$"
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

# Check that capture-pane output does NOT contain a given string
assert_no_text() {
    local target="$1"
    local text="$2"
    local output
    output=$(tmux -L "$SOCKET" capture-pane -t "$target" -p 2>/dev/null || true)
    if echo "$output" | grep -qF "$text"; then
        return 1
    fi
    return 0
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

# Create multiple sessions with fake agent panes
tmux -L "$SOCKET" new-session -d -s "alpha-project" -x 120 -y 40
tmux -L "$SOCKET" send-keys -t "alpha-project" "printf '\\033]0;claude\\007'" Enter

tmux -L "$SOCKET" new-session -d -s "beta-project" -x 120 -y 40
tmux -L "$SOCKET" send-keys -t "beta-project" "printf '\\033]0;claude\\007'" Enter

tmux -L "$SOCKET" new-session -d -s "gamma-project" -x 120 -y 40
tmux -L "$SOCKET" send-keys -t "gamma-project" "printf '\\033]0;claude\\007'" Enter

sleep 1

# Create wrapper directory with tmux wrapper and launcher script
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

# Launch tmuxcc in a new window via the short launcher script
tmux -L "$SOCKET" new-window -t "alpha-project" -n "tmuxcc"
tmux -L "$SOCKET" send-keys -t "alpha-project:tmuxcc" "$WRAPPER_DIR/launch.sh" Enter

# ── Tests ────────────────────────────────────────────────────────────────────

echo "Waiting for tmuxcc to render..."

# Test 1: Sessions are visible in the initial view
if wait_for_text "alpha-project:tmuxcc" "alpha-project" 15; then
    pass "alpha-project session visible"
else
    fail "alpha-project session not visible"
fi

if wait_for_text "alpha-project:tmuxcc" "beta-project" 5; then
    pass "beta-project session visible"
else
    fail "beta-project session not visible"
fi

# Test 2: Default mode shows "Recent" in footer
if wait_for_text "alpha-project:tmuxcc" "s:Recent" 5; then
    pass "default sort mode shows 's:Recent' in footer"
else
    fail "footer does not show 's:Recent' (default sort mode)"
fi

# Test 3: Press 's' to cycle to Status mode
tmux -L "$SOCKET" send-keys -t "alpha-project:tmuxcc" "s"
sleep 1

if wait_for_text "alpha-project:tmuxcc" "s:Status" 5; then
    pass "pressing 's' cycles to Status mode (footer shows 's:Status')"
else
    fail "footer does not show 's:Status' after pressing 's'"
fi

# Test 4: Press 's' again to cycle back to Recent mode
tmux -L "$SOCKET" send-keys -t "alpha-project:tmuxcc" "s"
sleep 1

if wait_for_text "alpha-project:tmuxcc" "s:Recent" 5; then
    pass "pressing 's' again cycles back to Recent mode"
else
    fail "footer does not show 's:Recent' after second 's' press"
fi

# Test 5: Verify Status mode doesn't show when in Recent mode
if assert_no_text "alpha-project:tmuxcc" "s:Status"; then
    pass "Recent mode footer does not contain 's:Status'"
else
    fail "footer unexpectedly shows 's:Status' in Recent mode"
fi

# Cleanup: quit tmuxcc
tmux -L "$SOCKET" send-keys -t "alpha-project:tmuxcc" "q"
sleep 0.5

echo ""
echo "All sort mode e2e tests passed."
exit 0
