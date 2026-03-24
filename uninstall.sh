#!/usr/bin/env bash
set -euo pipefail

# Oversight uninstall script
# Reverses everything setup.sh does: stops the daemon, removes managed
# blocks, removes the binary, and optionally deletes the KB and config.

INSTALL_DIR="${HOME}/.local/bin"
OVERSIGHT_BIN="${INSTALL_DIR}/oversight"
OVERSIGHT_DIR="${HOME}/.oversight"
CLAUDE_MD="${HOME}/.claude/CLAUDE.md"
PLIST_LABEL="com.twin-drill.oversight"
PLIST_PATH="${HOME}/Library/LaunchAgents/${PLIST_LABEL}.plist"
SYSTEMD_UNIT="oversight.service"
SYSTEMD_PATH="${HOME}/.config/systemd/user/${SYSTEMD_UNIT}"
AUTO_YES=false

for arg in "$@"; do
    case "$arg" in
        -y|--yes) AUTO_YES=true ;;
    esac
done

# ─── Helpers ──────────────────────────────────────────────────────

info()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m  ✓\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m  !\033[0m %s\n' "$*"; }

is_macos() { [ "$(uname -s)" = "Darwin" ]; }
is_linux() { [ "$(uname -s)" = "Linux" ]; }

# ─── Step 1: Stop and remove daemon ──────────────────────────────

info "Stopping healing loop daemon"

if is_macos && [ -f "$PLIST_PATH" ]; then
    launchctl bootout "gui/$(id -u)" "$PLIST_PATH" 2>/dev/null || true
    rm -f "$PLIST_PATH"
    ok "launchd agent stopped and removed"
elif is_linux && [ -f "$SYSTEMD_PATH" ]; then
    systemctl --user disable --now "$SYSTEMD_UNIT" 2>/dev/null || true
    rm -f "$SYSTEMD_PATH"
    systemctl --user daemon-reload 2>/dev/null || true
    ok "systemd user service stopped and removed"
else
    warn "No daemon service found (may not have been installed)"
fi

# ─── Step 2: Remove managed blocks ───────────────────────────────

info "Removing managed blocks from agent configs"

if [ -x "$OVERSIGHT_BIN" ]; then
    "$OVERSIGHT_BIN" integrate remove --target claude-code 2>/dev/null && \
        ok "Removed managed block from CLAUDE.md" || \
        warn "No managed block in CLAUDE.md (or already removed)"
else
    for f in "$CLAUDE_MD"; do
        if [ -f "$f" ] && grep -q "oversight:begin" "$f" 2>/dev/null; then
            sed -i.bak '/<!-- oversight:begin/,/<!-- oversight:end -->/d' "$f"
            ok "Removed managed block from $f (manual sed)"
            rm -f "$f.bak"
        fi
    done
fi

# ─── Step 3: Remove binary ───────────────────────────────────────

info "Removing oversight binary"
if [ -f "$OVERSIGHT_BIN" ]; then
    rm -f "$OVERSIGHT_BIN"
    ok "Removed ${OVERSIGHT_BIN}"
else
    warn "Binary not found at ${OVERSIGHT_BIN}"
fi

# ─── Step 4: Optionally remove KB and config ─────────────────────

if [ -d "$OVERSIGHT_DIR" ]; then
    if [ "$AUTO_YES" = true ]; then
        rm -rf "$OVERSIGHT_DIR"
        ok "Deleted ${OVERSIGHT_DIR}"
    else
        echo ""
        printf '\033[1;33m  ?\033[0m Delete %s (KB, config, API keys, state)? [y/N] ' "$OVERSIGHT_DIR"
        read -r answer
        if [ "$answer" = "y" ] || [ "$answer" = "Y" ]; then
            rm -rf "$OVERSIGHT_DIR"
            ok "Deleted ${OVERSIGHT_DIR}"
        else
            ok "Kept ${OVERSIGHT_DIR}"
        fi
    fi
fi

# ─── Done ─────────────────────────────────────────────────────────

echo ""
info "Uninstall complete"
echo ""
