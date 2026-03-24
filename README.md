# Oversight

AI coding agents are generic. Your development environment isn't. Oversight bridges that gap — it watches your agents work, extracts what they learn about *your* setup, and makes that knowledge available to every future session.

## What it does

Every dev environment has friction that agents have to rediscover from scratch: your project needs `--legacy-peer-deps` for npm install, your CI database requires a specific migration order, your staging deploy has an undocumented auth step. Agents burn time and tokens figuring this out, and the knowledge dies when the session ends.

Oversight fixes this with two components:

**Knowledge Base** — A local collection of Markdown topic files (`~/.oversight/kb/topics/`) that agents can query before running tools. Topics have titles, tags, aliases, and a confidence level. Oversight injects a topic index into your agent config file (`~/.claude/CLAUDE.md`) so agents know what's available.

**Healing Loop** — A pipeline that reads conversation transcripts from local agent session logs (Claude Code, Codex, Gemini CLI), sends them to an LLM for analysis, deduplicates the extracted learnings against the existing KB, and writes new topics or appends to existing ones. It runs as a single pass or a background daemon.

### Why not just add notes to CLAUDE.md?

You can, and for a handful of tips it works fine. But it stops scaling:

- **Agent config files have a soft attention budget.** As `CLAUDE.md` grows, directives at the bottom get progressively less attention from the model. By the time you have 30+ tips inlined, the early ones are effectively ignored.
- **Oversight keeps the config file small.** It injects only a compact topic index (names and aliases), not the full content. The agent reads the detail for a specific topic on demand, right before it needs it — when attention is focused.
- **Manual notes don't deduplicate.** Three agents in three projects will each discover the same workaround independently. You'd need to notice, write it up, and copy it to every project's config. Oversight's healing loop does this automatically.
- **Manual notes don't update.** When a workaround becomes obsolete or a better approach is found, stale notes linger. Oversight appends new insights to existing topics and tracks confidence levels, so knowledge evolves.

## Install

### Prerequisites

- [Rust](https://rustup.rs/) (stable)
- An API key for at least one supported LLM provider (for the healing loop):
  - [Anthropic](https://console.anthropic.com/) (`ANTHROPIC_API_KEY`)
  - [OpenAI](https://platform.openai.com/) (`OPENAI_API_KEY`)
  - [Google Gemini](https://aistudio.google.com/) (`GEMINI_API_KEY`)

### Quick setup

```bash
git clone https://github.com/twin-drill/oversight.git
cd oversight
./setup.sh
```

This builds the binary, detects which API keys you have set, initializes the KB with the appropriate provider, and installs managed blocks into your Claude Code config.

### Manual setup

```bash
git clone https://github.com/twin-drill/oversight.git
cd oversight

# Build
cargo build --release
cp target/release/oversight ~/.local/bin/

# Initialize
oversight init

# Install agent integrations
oversight integrate install --target claude-code
```

## Usage

### Knowledge Base

```bash
# Add a topic (body from stdin)
echo "Always pass --legacy-peer-deps when running npm install in this repo." \
  | oversight add "npm install workaround" -t npm -t node -a "npm install"

# List topics
oversight topics
oversight topics --json

# Read a topic by slug or alias
oversight read gh-cli-auth
oversight read "gh auth"

# Search
oversight search "docker compose"

# Update a topic (body from stdin)
echo "Updated instructions here." | oversight update gh-cli-auth

# Delete
oversight delete gh-cli-auth
```

### Healing Loop

The healing loop reads agent conversation transcripts from local session logs, extracts learnings via LLM, and writes them to the KB.

```bash
# Preview what would be extracted (no writes)
oversight loop run --dry-run

# Run a single pass
oversight loop run

# Run with a specific deduplication regime
oversight loop run --regime aggressive

# Start as a daemon (foreground, polls every 5 minutes)
oversight loop start --interval 300

# Check processing state
oversight loop status
```

The healing loop auto-detects your LLM provider from whichever API key is set. To use a specific provider, set it in `~/.oversight/config.toml` (see [Configuration](#configuration)).

Three deduplication regimes control how aggressively new topics are created:

| Regime | Behavior |
|---|---|
| `aggressive` | Almost never merges into existing topics; creates new ones freely |
| `balanced` (default) | Creates topics when clearly novel; appends to existing when related |
| `conservative` | Prefers merging into existing topics; only creates when no match exists |

### Agent Integration

Oversight injects a managed block into agent config files listing all KB topics, so agents know to check the KB before running tools.

```bash
# Install into ~/.claude/CLAUDE.md
oversight integrate install

# Install into a custom file
oversight integrate install --target generic-agents-md --path ~/my-agents.md

# Refresh topic list in all installed targets
oversight integrate refresh

# Check status
oversight integrate status

# Remove
oversight integrate remove
```

## Configuration

`~/.oversight/config.toml`:

```toml
kb_path = "~/.oversight/kb"

[loop]
interval_secs = 300
confidence_threshold = 0.7
max_transcript_len = 30000
regime = "balanced"
context_limit = 20

[llm]
provider = "anthropic"   # "anthropic", "openai", or "gemini"
model = "claude-sonnet-4-latest"
max_tokens = 4096

[integrate]
topic_preview_limit = 20
```

If you set a `provider` without specifying a `model`, the provider's default model is used automatically.

### LLM providers

| Provider | Config value | Env variable | Default model |
|---|---|---|---|
| Anthropic | `anthropic` | `ANTHROPIC_API_KEY` | `claude-sonnet-4-latest` |
| OpenAI | `openai` | `OPENAI_API_KEY` | `gpt-4o-mini` |
| Google Gemini | `gemini` | `GEMINI_API_KEY` | `gemini-2.0-flash` |

Provider detection order: if no provider is set in config, `setup.sh` checks for API keys in the order above and selects the first one found.

### Environment variables

| Variable | Purpose | Default |
|---|---|---|
| `ANTHROPIC_API_KEY` | Anthropic API key for healing loop | — |
| `OPENAI_API_KEY` | OpenAI API key for healing loop | — |
| `GEMINI_API_KEY` | Gemini API key for healing loop | — |
| `OVERSIGHT_KB_PATH` | Override KB root directory | `~/.oversight/kb` |
| `OVERSIGHT_SOURCE` | Transcript source (`claude-code`, `codex`, `gemini`) | `claude-code` |

## Architecture

```
                    ┌──────────────────┐
                    │  Agent Sessions   │  ~/.claude/projects/
                    │  (local logs)     │  ~/.codex/sessions/
                    └────────┬─────────┘  ~/.gemini/tmp/
                             │ read JSONL/JSON
                             ▼
┌─────────────────────────────────────────────────────┐
│                   Healing Loop                       │
│                                                      │
│  discover → fetch → extract (LLM) → dedupe → merge  │
└──────────────────────────┬──────────────────────────┘
                           │ write topics
                           ▼
                  ┌─────────────────┐
                  │  Knowledge Base  │  ~/.oversight/kb/topics/
                  │  (Markdown+YAML) │
                  └────────┬────────┘
                           │ integrate refresh
                           ▼
                  ┌─────────────────┐
                  │  Agent Config    │  ~/.claude/CLAUDE.md
                  │  (managed block) │
                  └─────────────────┘
```

## Development

```bash
cargo test                          # all tests
cargo test --test loop_pipeline     # specific test file
cargo clippy -- -D warnings         # lint
make e2e                            # full e2e suite (isolated $HOME)
```

### Running the e2e suite

```bash
make e2e
```

The e2e suite tests KB CRUD, agent integration, setup/uninstall scripts, and healing loop dry-run. Phases requiring API keys are skipped gracefully when unavailable.

### CI

GitHub Actions (`.github/workflows/ci.yml`) runs build + unit tests on every push, with e2e tests when API keys are configured as secrets.

## Contributing

### Getting started

```bash
git clone https://github.com/twin-drill/oversight.git
cd oversight
cargo build
cargo test
```

The project uses standard Rust tooling. No formatters or linters are enforced beyond `cargo clippy -- -D warnings`.

### Adding a new LLM provider

The LLM layer is in `src/llm/client.rs`. Each provider needs four things:

**1. Add the variant to `LlmProvider`**

```rust
// src/llm/client.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    #[default]
    Anthropic,
    OpenAI,
    Gemini,
    MyProvider,   // add here
}
```

**2. Implement the required trait arms**

Fill in the `match` arms in the existing `impl` blocks:

```rust
// Display
LlmProvider::MyProvider => write!(f, "MyProvider"),

// env_var() — which env variable holds the API key
LlmProvider::MyProvider => "MY_PROVIDER_API_KEY",

// default_model()
LlmProvider::MyProvider => "my-model-v1",
```

**3. Write the completion method**

Add an `async fn complete_myprovider(...)` method to `LlmClient` following the pattern of the existing providers. The method receives an optional system prompt and a user prompt, calls the provider's HTTP API, and returns the extracted text. Then wire it into the `complete()` dispatcher:

```rust
pub async fn complete(&self, system: Option<&str>, user_prompt: &str) -> Result<String> {
    match self.provider {
        // ...existing arms...
        LlmProvider::MyProvider => self.complete_myprovider(system, user_prompt).await,
    }
}
```

Each provider method should:
- Send API keys via headers, **never** in URL query parameters
- Check `resp.status().is_success()` and return `Error::LlmApi` with the response body on failure
- Join all text parts from multi-part responses (don't silently drop content)

**4. Update `detect_from_env()`**

Add the provider to the detection loop in `LlmProvider::detect_from_env()`. Order matters — providers are checked first-to-last, so place yours after the existing ones unless there's a reason to prefer it.

**5. Add the e2e compliance test** (optional)

In `tests/e2e/ci.sh`, add a block in Phase 3 that calls the provider's API with the managed block as a system prompt and checks that the response mentions `oversight`. See the existing Anthropic/OpenAI/Gemini blocks for the pattern.

### Adding a new transcript source

Transcript sources live in `src/source/providers/`. Each provider is a struct that implements two methods:

- `discover_candidates(&self, state: &LoopState, limit: u32) -> Result<Vec<Candidate>>` — scan the local filesystem for session files, return candidates not yet processed.
- `get_turns(&self, candidate: &Candidate) -> Result<Vec<TypedTurn>>` — parse a session file into normalized turns.

After creating your provider module:

1. Add it to `src/source/providers/mod.rs`
2. Add a variant to `TranscriptSource` in `src/source/mod.rs` and wire it into the `discover_candidates`, `get_turns`, and `source_name` dispatch methods
3. Add a match arm in `Config::build_source()` (`src/config.rs`) keyed on an `OVERSIGHT_SOURCE` env value

### Adding a new integration target

Integration targets are defined in `src/integrate/targets.rs`. To add one:

1. Add a constructor method on `IntegrationTarget` (see `claude_code()` and `generic_agents_md()` for the pattern)
2. Add a match arm in `resolve_target()`
3. If needed, add a new `InstructionStyle` variant in the same file and handle it in `src/integrate/render.rs`

## License

Apache-2.0. See [LICENSE](LICENSE) for details.
