#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd "$(dirname "$0")" && pwd)"
INSTALL_DIR="${HOME}/.local/bin"
OVERSIGHT_BIN="${INSTALL_DIR}/oversight"
OVERSIGHT_DIR="${HOME}/.oversight"
ENV_FILE="${OVERSIGHT_DIR}/env"
PLIST_LABEL="com.twin-drill.oversight"
PLIST_PATH="${HOME}/Library/LaunchAgents/${PLIST_LABEL}.plist"
SYSTEMD_UNIT="oversight.service"
SYSTEMD_PATH="${HOME}/.config/systemd/user/${SYSTEMD_UNIT}"

# ─── Helpers ──────────────────────────────────────────────────────

info()  { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()    { printf '\033[1;32m  ✓\033[0m %s\n' "$*"; }
warn()  { printf '\033[1;33m  !\033[0m %s\n' "$*"; }
fail()  { printf '\033[1;31m  ✗\033[0m %s\n' "$*"; exit 1; }

require() {
    command -v "$1" >/dev/null 2>&1 || fail "$1 is required but not found. Please install it first."
}

is_macos() { [ "$(uname -s)" = "Darwin" ]; }
is_linux() { [ "$(uname -s)" = "Linux" ]; }

# ─── Preflight ────────────────────────────────────────────────────

info "Checking prerequisites"
require cargo
ok "cargo available"

# ─── Step 1: Build oversight ─────────────────────────────────────

info "Building oversight (release)"
cd "$REPO_DIR"
cargo build --release --quiet
ok "Built target/release/oversight"

# ─── Step 2: Install binary ──────────────────────────────────────

info "Installing oversight to ${INSTALL_DIR}"
mkdir -p "$INSTALL_DIR"
cp -f "${REPO_DIR}/target/release/oversight" "$OVERSIGHT_BIN"
chmod +x "$OVERSIGHT_BIN"
ok "Installed ${OVERSIGHT_BIN}"

if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
    warn "${INSTALL_DIR} is not in your PATH"
    warn "Add this to your shell profile:"
    warn "  export PATH=\"${INSTALL_DIR}:\$PATH\""
fi

# ─── Step 3: Detect LLM API key ──────────────────────────────────

info "Detecting LLM API keys"
LLM_PROVIDER=""
LLM_MODEL=""
FOUND_KEYS=""

[ -n "${ANTHROPIC_API_KEY:-}" ] && FOUND_KEYS="${FOUND_KEYS} ANTHROPIC_API_KEY"
[ -n "${OPENAI_API_KEY:-}" ]    && FOUND_KEYS="${FOUND_KEYS} OPENAI_API_KEY"
[ -n "${GEMINI_API_KEY:-}" ]    && FOUND_KEYS="${FOUND_KEYS} GEMINI_API_KEY"

if [ -z "$FOUND_KEYS" ]; then
    fail "No LLM API key found. Set one of: ANTHROPIC_API_KEY, OPENAI_API_KEY, GEMINI_API_KEY"
fi

ok "Keys found:${FOUND_KEYS}"

if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
    LLM_PROVIDER="anthropic"
    LLM_MODEL="claude-sonnet-4-6"
elif [ -n "${OPENAI_API_KEY:-}" ]; then
    LLM_PROVIDER="openai"
    LLM_MODEL="gpt-4o-mini"
elif [ -n "${GEMINI_API_KEY:-}" ]; then
    LLM_PROVIDER="gemini"
    LLM_MODEL="gemini-2.0-flash"
fi

ok "Selected provider: ${LLM_PROVIDER} (${LLM_MODEL})"

# ─── Step 4: Initialize KB, write config, and store keys ─────────

info "Initializing knowledge base"
"$OVERSIGHT_BIN" init
ok "KB initialized at ~/.oversight/kb/"

CONFIG_FILE="${OVERSIGHT_DIR}/config.toml"
mkdir -p "$OVERSIGHT_DIR"
cat > "$CONFIG_FILE" <<EOF
kb_path = "${OVERSIGHT_DIR}/kb"

[llm]
provider = "${LLM_PROVIDER}"
model = "${LLM_MODEL}"
max_tokens = 4096

[loop]
interval_secs = 300
confidence_threshold = 0.7
EOF
ok "Config written to ${CONFIG_FILE} (provider: ${LLM_PROVIDER})"

# Write API keys to a private env file (mode 600)
: > "$ENV_FILE"
chmod 600 "$ENV_FILE"
[ -n "${ANTHROPIC_API_KEY:-}" ] && echo "ANTHROPIC_API_KEY=${ANTHROPIC_API_KEY}" >> "$ENV_FILE"
[ -n "${OPENAI_API_KEY:-}" ]    && echo "OPENAI_API_KEY=${OPENAI_API_KEY}" >> "$ENV_FILE"
[ -n "${GEMINI_API_KEY:-}" ]    && echo "GEMINI_API_KEY=${GEMINI_API_KEY}" >> "$ENV_FILE"
ok "API keys stored in ${ENV_FILE} (mode 600)"

# ─── Step 5: Integrate with Claude Code ──────────────────────────

info "Injecting oversight into ~/.claude/CLAUDE.md"
"$OVERSIGHT_BIN" integrate install --target claude-code
ok "Managed block installed in CLAUDE.md"

# ─── Step 6: Install and start healing loop daemon ───────────────

info "Installing healing loop daemon"

# Wrapper script that sources the env file before starting the daemon
WRAPPER="${OVERSIGHT_DIR}/start-daemon.sh"
cat > "$WRAPPER" <<'WEOF'
#!/usr/bin/env bash
set -euo pipefail
ENV_FILE="${HOME}/.oversight/env"
[ -f "$ENV_FILE" ] && set -a && . "$ENV_FILE" && set +a
exec oversight loop start
WEOF
chmod 700 "$WRAPPER"

if is_macos; then
    mkdir -p "$(dirname "$PLIST_PATH")"
    cat > "$PLIST_PATH" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>${PLIST_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>${WRAPPER}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>${OVERSIGHT_DIR}/daemon.log</string>
    <key>StandardErrorPath</key>
    <string>${OVERSIGHT_DIR}/daemon.log</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>PATH</key>
        <string>${INSTALL_DIR}:/usr/local/bin:/usr/bin:/bin</string>
    </dict>
</dict>
</plist>
EOF
    launchctl bootout "gui/$(id -u)" "$PLIST_PATH" 2>/dev/null || true
    launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH"
    ok "launchd agent installed and started (${PLIST_LABEL})"

elif is_linux && command -v systemctl >/dev/null 2>&1; then
    mkdir -p "$(dirname "$SYSTEMD_PATH")"
    cat > "$SYSTEMD_PATH" <<EOF
[Unit]
Description=Oversight healing loop daemon
After=default.target

[Service]
Type=simple
ExecStart=${WRAPPER}
Restart=on-failure
RestartSec=30
Environment=PATH=${INSTALL_DIR}:/usr/local/bin:/usr/bin:/bin

[Install]
WantedBy=default.target
EOF
    systemctl --user daemon-reload
    systemctl --user enable --now "$SYSTEMD_UNIT"
    ok "systemd user service installed and started (${SYSTEMD_UNIT})"

else
    warn "Could not detect launchd or systemd. Start the daemon manually:"
    warn "  oversight loop start &"
fi

# ─── Done ─────────────────────────────────────────────────────────

echo ""
info "Setup complete"
echo ""
echo "  oversight binary:  ${OVERSIGHT_BIN}"
echo "  knowledge base:    ~/.oversight/kb/"
echo "  API keys:          ~/.oversight/env (mode 600)"
echo "  healing loop:      running as background service"
echo "  daemon log:        ~/.oversight/daemon.log"
echo "  CLAUDE.md:         ~/.claude/CLAUDE.md (managed block injected)"
echo ""
echo "  The daemon will process your existing sessions over the next few hours."
echo ""
