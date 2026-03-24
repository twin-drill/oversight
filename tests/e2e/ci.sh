#!/usr/bin/env bash
set -euo pipefail

# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
# Oversight end-to-end test suite
#
# Runs natively on the host (CI runner or dev machine).
# Expects oversight binary on PATH.
#
# Environment:
#   ANTHROPIC_API_KEY    optional — enables LLM compliance + healing loop tests
#   OPENAI_API_KEY       optional — enables OpenAI compliance test
#   GEMINI_API_KEY       optional — enables Gemini compliance test
#   OVERSIGHT_BIN        path to oversight binary (default: oversight)
#   WORKSPACE            repo root (default: script's grandparent directory)
#
# Usage:
#   # minimal (unit tests + KB CRUD, no LLM keys needed)
#   ./tests/e2e/run-oversight-tests.sh
#
#   # full (all phases including LLM tests)
#   ANTHROPIC_API_KEY=sk-... ./tests/e2e/run-oversight-tests.sh
# ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

PASS=0
FAIL=0
SKIP=0

pass() { printf '\033[1;32m  PASS\033[0m %s\n' "$*"; PASS=$((PASS + 1)); }
fail() { printf '\033[1;31m  FAIL\033[0m %s\n' "$*"; FAIL=$((FAIL + 1)); }
skip() { printf '\033[1;33m  SKIP\033[0m %s\n' "$*"; SKIP=$((SKIP + 1)); }
section() { printf '\n\033[1;36m━━━ %s ━━━\033[0m\n' "$*"; }

WORKSPACE="${WORKSPACE:-$(cd "$(dirname "$0")/../.." && pwd)}"
BIN="${OVERSIGHT_BIN:-oversight}"

if ! command -v "$BIN" >/dev/null 2>&1; then
    printf '\033[1;31mERROR:\033[0m oversight binary not found at "%s"\n' "$BIN" >&2
    printf '  Set OVERSIGHT_BIN or ensure oversight is on PATH.\n' >&2
    exit 1
fi

# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 1: KB CRUD                                                       ║
# ╚═══════════════════════════════════════════════════════════════════════════╝
section "Phase 1: Knowledge Base CRUD"

$BIN init 2>/dev/null || true

echo "When gh auth login fails with 'read-only token', unset GITHUB_TOKEN first." \
  | $BIN add "GitHub CLI Auth" -t cli -t github -a "gh auth" -a "github cli"
FIRST_SLUG=$($BIN topics --json | jq -r '.[0].slug // empty')
if $BIN read "$FIRST_SLUG" >/dev/null 2>&1; then
    pass "add + read by slug ($FIRST_SLUG)"
else
    fail "add + read by slug"
fi

if $BIN read "gh auth" >/dev/null 2>&1; then
    pass "read by alias"
else
    fail "read by alias"
fi

echo "Run 'aws sso login --profile dev' before any AWS CLI commands when tokens expire." \
  | $BIN add "AWS SSO Login" -t aws -t sso -a "aws login"
echo "Use 'docker compose up -d --build' to force image rebuild on source changes." \
  | $BIN add "Docker Compose Rebuild" -t docker -t compose -a "docker rebuild"

TOPIC_COUNT=$($BIN topics --json | jq 'length')
if [ "$TOPIC_COUNT" -eq 3 ]; then
    pass "topics list shows 3 topics"
else
    fail "topics list expected 3, got ${TOPIC_COUNT}"
fi

SEARCH_RESULT=$($BIN search "github token" 2>&1)
if echo "$SEARCH_RESULT" | grep -qi "github-cli-auth"; then
    pass "search finds github-cli-auth for 'github token'"
else
    fail "search for 'github token': ${SEARCH_RESULT}"
fi

echo "Also try: gh auth switch if multiple accounts are configured." \
  | $BIN update "$FIRST_SLUG"
UPDATED=$($BIN read "gh auth" --raw 2>/dev/null || true)
if echo "$UPDATED" | grep -q "gh auth switch"; then
    pass "update topic body"
else
    fail "update topic body"
fi

$BIN delete docker-compose-rebuild 2>/dev/null || true
TOPIC_COUNT_AFTER=$($BIN topics --json | jq 'length')
if [ "$TOPIC_COUNT_AFTER" -eq 2 ]; then
    pass "delete topic"
else
    fail "delete topic (expected 2, got ${TOPIC_COUNT_AFTER})"
fi

# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 2: Integrate – Managed Block Injection                           ║
# ╚═══════════════════════════════════════════════════════════════════════════╝
section "Phase 2: Integrate system"

CLAUDE_DIR="${HOME}/.claude"
CLAUDE_MD="${CLAUDE_DIR}/CLAUDE.md"
mkdir -p "$CLAUDE_DIR"

$BIN integrate install --target claude-code
$BIN integrate refresh --target claude-code
if grep -q "github-cli-auth" "$CLAUDE_MD" 2>/dev/null; then
    pass "refresh updates CLAUDE.md with current topics"
else
    fail "refresh did not update CLAUDE.md"
fi

STATUS_OUT=$($BIN integrate status 2>&1)
if echo "$STATUS_OUT" | grep -qiE "healthy|installed"; then
    pass "integrate status reports healthy"
else
    fail "integrate status: ${STATUS_OUT}"
fi

MANAGED_BLOCK=$(cat "$CLAUDE_MD")

# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 3: Cross-Agent Instruction Compliance                            ║
# ╚═══════════════════════════════════════════════════════════════════════════╝
section "Phase 3: Cross-agent instruction compliance"

SCENARIO="Commit my changes and open a pull request for this branch."

check_compliance() {
    local provider="$1"
    local response="$2"

    if echo "$response" | grep -qiE "oversight (topics|search|read)"; then
        pass "${provider}: response includes oversight command"
    elif echo "$response" | grep -qi "oversight"; then
        pass "${provider}: response references oversight"
    else
        fail "${provider}: response does not mention oversight"
        echo "    First 200 chars: $(echo "$response" | head -c 200)"
    fi
}

if [ -n "${ANTHROPIC_API_KEY:-}" ]; then
    RESP=$(curl -sf https://api.anthropic.com/v1/messages \
        -H "x-api-key: ${ANTHROPIC_API_KEY}" \
        -H "anthropic-version: 2023-06-01" \
        -H "content-type: application/json" \
        -d "$(jq -n --arg s "$MANAGED_BLOCK" --arg u "$SCENARIO" \
            '{model:"claude-sonnet-4-20250514",max_tokens:512,system:$s,messages:[{role:"user",content:$u}]}')" \
        2>&1) || true
    TEXT=$(echo "$RESP" | jq -r '.content[]? | select(.type=="text") | .text // empty' 2>/dev/null || echo "")
    if [ -n "$TEXT" ]; then
        check_compliance "Anthropic" "$TEXT"
    else
        fail "Anthropic: API error"; echo "    $(echo "$RESP" | head -c 200)"
    fi
else
    skip "Anthropic: ANTHROPIC_API_KEY not set"
fi

if [ -n "${OPENAI_API_KEY:-}" ]; then
    RESP=$(curl -sf https://api.openai.com/v1/chat/completions \
        -H "Authorization: Bearer ${OPENAI_API_KEY}" \
        -H "content-type: application/json" \
        -d "$(jq -n --arg s "$MANAGED_BLOCK" --arg u "$SCENARIO" \
            '{model:"gpt-4o-mini",max_tokens:512,messages:[{role:"system",content:$s},{role:"user",content:$u}]}')" \
        2>&1) || true
    TEXT=$(echo "$RESP" | jq -r '.choices[0]?.message?.content // empty' 2>/dev/null || echo "")
    if [ -n "$TEXT" ]; then
        check_compliance "OpenAI" "$TEXT"
    else
        fail "OpenAI: API error"; echo "    $(echo "$RESP" | head -c 200)"
    fi
else
    skip "OpenAI: OPENAI_API_KEY not set"
fi

if [ -n "${GEMINI_API_KEY:-}" ]; then
    RESP=$(curl -sf "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent" \
        -H "x-goog-api-key: ${GEMINI_API_KEY}" \
        -H "content-type: application/json" \
        -d "$(jq -n --arg s "$MANAGED_BLOCK" --arg u "$SCENARIO" \
            '{system_instruction:{parts:[{text:$s}]},contents:[{role:"user",parts:[{text:$u}]}],generationConfig:{maxOutputTokens:512}}')" \
        2>&1) || true
    TEXT=$(echo "$RESP" | jq -r '.candidates[0]?.content?.parts[0]?.text // empty' 2>/dev/null || echo "")
    if [ -n "$TEXT" ]; then
        check_compliance "Gemini" "$TEXT"
    else
        fail "Gemini: API error"; echo "    $(echo "$RESP" | head -c 200)"
    fi
else
    skip "Gemini: GEMINI_API_KEY not set"
fi

# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 4: Setup script                                                  ║
# ╚═══════════════════════════════════════════════════════════════════════════╝
section "Phase 4: Setup script"

# setup.sh requires an API key — provide a dummy one so it doesn't fail
# (the daemon won't actually call the API in this test)
if [ -z "${ANTHROPIC_API_KEY:-}${OPENAI_API_KEY:-}${GEMINI_API_KEY:-}" ]; then
    export OPENAI_API_KEY="sk-test-dummy-key-for-ci"
    DUMMY_KEY=true
else
    DUMMY_KEY=false
fi

if bash "${WORKSPACE}/setup.sh" 2>&1; then
    pass "setup.sh completed"
else
    fail "setup.sh exited with error"
fi

if [ -f "$HOME/.local/bin/oversight" ]; then
    pass "binary installed by setup.sh"
else
    fail "binary not installed by setup.sh"
fi

if [ -f "$HOME/.oversight/config.toml" ]; then
    pass "config.toml written"
    # Verify config has the llm section
    if grep -q "provider" "$HOME/.oversight/config.toml" 2>/dev/null; then
        pass "config.toml has llm provider"
    else
        fail "config.toml missing llm provider"
    fi
else
    fail "config.toml not written"
fi

if grep -q "oversight:begin" "$CLAUDE_MD" 2>/dev/null; then
    pass "setup.sh installed managed block in CLAUDE.md"
else
    fail "setup.sh did not install managed block"
fi

# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 5: Healing loop dry-run                                          ║
# ╚═══════════════════════════════════════════════════════════════════════════╝
section "Phase 5: Healing loop"

DRY_OUTPUT=$($BIN loop run --dry-run 2>&1) || true
if echo "$DRY_OUTPUT" | grep -qE "Discovering contexts from"; then
    pass "loop dry-run discovers from a source"
else
    fail "loop dry-run did not report source"
    echo "    $(echo "$DRY_OUTPUT" | head -5)"
fi

STATUS_OUTPUT=$($BIN loop status 2>&1) || true
if echo "$STATUS_OUTPUT" | grep -qiE "last poll|contexts processed"; then
    pass "loop status reports state"
else
    fail "loop status output unexpected"
fi

# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 6: Uninstall                                                     ║
# ╚═══════════════════════════════════════════════════════════════════════════╝
section "Phase 6: Uninstall"

if bash "${WORKSPACE}/uninstall.sh" --yes 2>&1; then
    pass "uninstall.sh completed"
else
    fail "uninstall.sh exited with error"
fi

if [ -f "$HOME/.local/bin/oversight" ]; then
    fail "binary still exists after uninstall"
else
    pass "binary removed"
fi

if grep -q "oversight:begin" "$CLAUDE_MD" 2>/dev/null; then
    fail "managed block still in CLAUDE.md after uninstall"
else
    pass "managed block removed from CLAUDE.md"
fi

if [ -d "$HOME/.oversight" ]; then
    fail "~/.oversight still exists after uninstall --yes"
else
    pass "~/.oversight deleted"
fi

# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  PHASE 7: Cargo test suite                                              ║
# ╚═══════════════════════════════════════════════════════════════════════════╝
section "Phase 7: Cargo test suite"

cd "$WORKSPACE"
if cargo test --release 2>&1; then
    pass "cargo test (all unit + integration tests)"
else
    fail "cargo test had failures"
fi

# ╔═══════════════════════════════════════════════════════════════════════════╗
# ║  Summary                                                                ║
# ╚═══════════════════════════════════════════════════════════════════════════╝
section "Results"
TOTAL=$((PASS + FAIL + SKIP))
printf '  \033[1;32m%d passed\033[0m, \033[1;31m%d failed\033[0m, \033[1;33m%d skipped\033[0m (of %d)\n' \
    "$PASS" "$FAIL" "$SKIP" "$TOTAL"
echo ""

if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
