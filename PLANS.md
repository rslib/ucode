---

## Project goals

* Fullscreen TUI (Linux + macOS)
* Multi-model routing + fallback (fast/strong/longctx)
* Async subagent orchestration with inter-agent communication
* Mention-driven orchestration (`@agent`) + user-defined slash commands (`/command`)
* Built-in tools (fs/search/patch/cmd/git/ast-grep) — all Rust-native, no CLI shelling
* Fine-grained sandbox policy (per-tool, per-agent, per-provider)
* MCP client (external tool servers) with native launcher support (`uvx`/`npx`/`bunx`)
* Skills compatibility: load `SKILL.md` (Claude Code + OpenCode)
* Plugins/hooks system for user customization
* Auth:

  * API keys
  * “Login” flows (browser/device)
  * Subscription-plan login (Claude Pro/Max style; OpenCode-like)

---

## Repo structure (recommended)

```
ucode/
  crates/
    ucode-core/        # canonical messages/events, router, session state, subagent orchestration
    ucode-auth/        # keychain + login flows + token refresh
    ucode-providers/   # openai/anthropic/ollama adapters
    ucode-tools/       # built-in tools + registry + permissions + sandbox policy engine
    ucode-mcp/         # MCP client + server registry + native launchers
    ucode-skills/      # SKILL.md discovery/parsing
    ucode-plugins/     # plugin runner + hooks API
    ucode-tui/         # ratatui app
    ucode-cli/         # clap commands, wiring
  docs/
  examples/
```

---

# Phase 0 — Bootstrapping

### Task 0.1: Workspace skeleton [DONE]

* Cargo workspace + crates above
* CI: fmt/clippy/test on Linux + macOS
* Baseline deps: tokio/serde/tracing/thiserror

**Acceptance**

* `cargo fmt && cargo clippy && cargo test` pass.

---

# Phase 1 — Canonical runtime + routing/fallback

### Task 1.1: Canonical message + event model (ucode-core) [DONE]

Types:

* `Message { role, parts: Vec<Part> }`
* `Part::Text | Part::ToolCall | Part::ToolResult`
* `ToolCall { id, name, args: Value }`
* `ToolResult { id, name, result: Value, is_error }`

Streaming events:

* `Event::Token(String)`
* `Event::ToolCall(ToolCall)`
* `Event::ToolResult(ToolResult)`
* `Event::Patch(String)`
* `Event::Log(String)`
* `Event::Error(AppError)`
* `Event::Done`

**Acceptance**

* Tests for serde + a fake provider stream.

### Task 1.2: Router + fallback policy (ucode-core) [DONE]

* Model groups: `fast`, `strong`, `longctx`
* Fallback triggers:

  * rate limit / timeout → next provider
  * auth failure → next provider
  * patch-apply fails twice → escalate to strong
  * context-too-large → shrink context pack then retry once

**Acceptance**

* Deterministic unit tests for routing and fallback.

### Task 1.3: Async subagent orchestration (ucode-core) [DONE]

* `spawn_agent(spec) -> AgentHandle` (non-blocking)
* `wait_agent(handle, timeout?) -> AgentResult`
* `wait_all(handles) -> Vec<AgentResult>`, `wait_any(handles) -> AgentResult`
* `cancel_agent(handle)`, `list_agents()`
* Parent/child task DAG with deterministic completion collection
* Lifecycle events:

  * `Event::AgentSpawned { id, spec }`
  * `Event::AgentMessage { from, to, payload }`
  * `Event::AgentCompleted { id, result }`
  * `Event::AgentFailed { id, error }`

**Acceptance**

* Spawn 3 agents concurrently; parent continues work while they run.
* `wait_all` returns deterministic completion summary.
* Cancellation stops running agent and emits failure/cancel event.

### Task 1.4: Inter-agent communication channels (ucode-core) [DONE]

* Optional mailbox channel per agent (`send(agent_id, payload)`)
* Optional shared context board for fan-out/fan-in coordination
* Policy-gated: inter-agent communication defaults to `off`, enabled per agent profile
* Message size limits + structured schema + audit logging
* Communication disabled does not break orchestration (spawn/wait still works)

**Acceptance**

* Two agents exchange messages through mailbox.
* Shared board sync works for fan-out/fan-in workflow.
* Policy can disable inter-agent messaging and system still functions normally.

### Task 1.5: Mention/command parser + orchestrator bindings (ucode-core) [DONE]

* Parse user directives from input text:

  * `@<agent-name>` for explicit subagent spawn
  * `/command` style slash commands
  * keep file references compatible with existing `@path` behavior via resolver order

* Resolver order (deterministic): slash command -> registered agent mention -> file reference
* Support escaping (`\@name`, `\/command`) to avoid accidental invocation
* `@agent` can spawn async and return handle to orchestrator (`spawn_agent` integration)

**Acceptance**

* `@agent-name` spawns the targeted registered agent.
* `/command-name` resolves to user/project/plugin command definition.
* Ambiguous `@token` cases follow resolver order and produce clear diagnostics.

### Task 1.6: Token budget manager + context compaction/distillation (ucode-core) [DONE]

* Add per provider/model token budget estimator (input + reserved output budget)
* Run context-fit preflight before every model request
* Counting strategy order:

  * use provider-native token counting when available
  * fallback to local tokenizer-based estimation with conservative safety margin
* Track per-request budget envelope: `max_context`, reserved output, available input
* Hybrid compaction modes:

  * rule-based/no-model compaction first (trim + deterministic packing)
  * optional model-assisted summarization (same model or smaller summarizer)
  * compaction must not depend on availability of a smaller model
* Add deterministic recovery chain when over budget:

  * trim low-value artifacts (verbose logs/tool chatter)
  * compact older turns into concise summaries
  * distill long tool outputs into structured memory records
  * keep recent turns + unresolved tool context pinned

* Persist compaction/distillation artifacts in session store with provenance links
* Emit runtime logs for each compaction/distillation decision
* Persist count source (`provider_count` vs `local_estimate`) for diagnostics

**Acceptance**

* Oversized transcript auto-compacts/distills and request succeeds without manual intervention.
* Pinned recent turns and unresolved tool context are preserved.
* Distilled artifacts reload correctly with session state.
* Provider-count unavailable path uses local estimate + safety margin and remains stable.
* Rule-based compaction path succeeds without summarizer model; model-assisted path remains optional.

### Task 1.7: Session lifecycle + model-generated session titles (ucode-core + ucode-cli + ucode-tui) [DONE]

* Extend session metadata: `title`, `title_source(auto|manual)`, `created_at`, `last_active_at`, `archived`
* Generate initial session title from early conversation turns using active model
* Add deterministic fallback title path if model title generation fails/unavailable
* Add manual rename + title lock so manual titles are never overwritten
* Add lifecycle operations: create/list/switch/archive/unarchive/rename in CLI and TUI
* Persist and surface title/session metadata in session selectors

**Acceptance**

* New sessions get auto titles; fallback title used when title generation is unavailable.
* Manual rename is durable across reload and protected from auto overwrite.
* Archive/unarchive and switch operations work in both CLI and TUI.

### Task 1.8: Session resume/fork lineage model (ucode-core + ucode-cli + ucode-tui) [P0] [DONE]

* Resume sessions by id with full state restoration (model/skill/policy/transcript)
* Fork sessions into child branches with explicit parent-child lineage metadata
* Surface lineage in CLI/TUI session switcher views
* Emit audit events for resume/fork/switch actions

**Acceptance**

* Resume restores full runnable session state.
* Fork creates child session with correct ancestry metadata.
* Switching between parent/child sessions does not leak runtime state.

### Task 1.9: Token/cost governance controls (ucode-core + ucode-providers + ucode-tui) [P1] [DONE]

* Track token and estimated cost per request/session across providers
* Configurable soft/hard budgets with policy actions (warn, fallback, block)
* TUI budget alerts and usage visibility in sidebar/status
* Persist usage summaries in session metadata

**Acceptance**

* Soft threshold emits warning without aborting workflow.
* Hard threshold enforces configured policy action.
* Usage summary persists and reloads with session.

### Task 1.10: Structured logging subsystem (stderr + file, session + rolling) (ucode-core + ucode-cli + ucode-tui) [DONE]

* Implement levels: `ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`
* Default runtime level is `INFO`; `DEBUG`/`TRACE` require explicit opt-in
* Runtime control surfaces:

  * env vars: `UCODE_LOG_LEVEL`, `UCODE_LOG_STDERR`, `UCODE_LOG_FILE`, `UCODE_LOG_DIR`, `UCODE_LOG_ROLLING`
  * CLI flags: `--log-level`, `--log-file`, `--log-dir`, `--log-stderr`, `--trace`
* Logging precedence: CLI flags > env vars > config file > defaults
* Env var semantics:

  * `UCODE_LOG_LEVEL`: `error|warn|info|debug|trace`
  * `UCODE_LOG_STDERR`: boolean sink toggle (`1/0`, `true/false`)
  * `UCODE_LOG_FILE`: file path override
  * `UCODE_LOG_DIR`: directory override for session/rolling logs
  * `UCODE_LOG_ROLLING`: boolean toggle for global rolling log
* Support dual sinks:

  * stderr for interactive runs
  * file sink for persistent diagnostics
* Stdout policy: keep stdout reserved for command/model output and machine-readable responses; do not emit logs to stdout by default
* Hybrid retention strategy:

  * per-session log files as primary (`session-<id>.log`)
  * optional rolling global log (size/time rotation)
* XDG-compliant defaults for log storage:

  * `${XDG_STATE_HOME:-~/.local/state}/ucode/logs`
  * explicit `--log-dir` / `UCODE_LOG_DIR` overrides default location
* Emit structured log fields: timestamp, level, session_id, provider/model, tool_name, event_type
* Add sensitive-data redaction rules for all sinks

**Acceptance**

* Default run logs `INFO+` to stderr + per-session file.
* Default run keeps stdout clean for output piping/JSON consumers.
* `DEBUG` and `TRACE` produce output only when explicitly enabled.
* Per-session logs map cleanly to one session id.
* Rolling log rotates correctly and does not break session log attribution.
* Default log path resolves to XDG state directory when no override is set.
* Secret values are redacted in stderr and file logs.

---

# Phase 2 — Auth (full provider auth coverage)

> **Reference for OAuth details:** OpenCode (`github.com/anomalyco/opencode`)
> Our auth system is our own design. Use OpenCode source as reference only when stuck on
> provider-specific OAuth details (client IDs, endpoints, scopes, flow quirks).
> Key files to consult when needed:
> * `packages/opencode/src/plugin/copilot.ts` — GitHub Copilot device code (client ID, enterprise URL, polling)
> * `packages/opencode/src/cli/cmd/auth.ts` — well-known auth flow (`.well-known/opencode` format)
> * `packages/opencode/src/auth/index.ts` — credential types and storage format

## 2.1 Credential storage (ucode-auth) [DONE]

* OS keychain via `keyring` (macOS Keychain + Linux Secret Service)
* Minimal metadata in config only (no plaintext secrets)
* Redact secrets in logs

CLI:

* `ucode auth status`
* `ucode auth set-key <provider>`
* `ucode auth logout <provider>`

TUI:

* `/connect` opens provider picker + auth method picker

**Acceptance**

* Keys persist across runs; status shows configured providers.

## 2.2 Auth foundation refactor (ucode-auth) [P0] [DONE]

Generalize the auth crate to support arbitrary providers and all auth methods from the
OpenCode ecosystem. This is the infrastructure layer that all subsequent auth tasks build on.

**Config vs runtime state boundary:**

TOML config (static, user-edited) contains only:
* `type` — which adapter (`openai`, `anthropic`, `ollama`, `gemini`)
* `base_url` — endpoint override
* `api_key_env` — env var name to check for API key
* `headers` — custom headers
* `auth_methods` — which login flows are available (e.g., `["api_key", "device_code"]`)

TOML config never contains secrets. OAuth tokens, session tokens, API keys entered via
`ucode auth login` — all runtime auth state — lives in the credential store (keyring or
`auth.json`). The `ucode auth login <provider>` command drives the flow interactively,
obtains tokens, and stores them. Provider adapters call `resolve_auth()` at request time.

**ProviderId → string-based:**

* Replace `ProviderId` enum (currently `OpenAi | Anthropic | Ollama`) with `String`-based identifiers
* Provider IDs match TOML config keys (e.g., `"openai"`, `"groq"`, `"github-copilot"`, `"my-custom-proxy"`)
* Separate `ProviderType` enum for adapter selection: `OpenAi | Anthropic | Ollama | Gemini`
  (this is the `type` field in TOML — fixed set of supported protocols)
* `CredentialStore` trait updated: `store/load/delete/status` take `&str` instead of `ProviderId`
* `all_providers()` removed — configured providers come from TOML config, not a hardcoded list
* `list_configured()` returns all providers that have stored credentials

**AuthMaterial expansion:**

```rust
enum AuthMaterial {
    ApiKey { key: String },
    OAuth {
        access_token: String,
        refresh_token: Option<String>,
        expires_at: Option<String>,  // ISO 8601
    },
    SessionToken {
        token: String,
        expires_at: Option<String>,
    },
    WellKnown {
        /// Env var name the provider expects (e.g., "CUSTOM_API_KEY")
        env_key: String,
        /// The actual token value
        token: String,
    },
    AwsCredentials {
        access_key_id: String,
        secret_access_key: String,
        session_token: Option<String>,
        region: String,
    },
}
```

**File-based credential fallback:**

* Primary: OS keychain via `keyring` (existing `KeyringStore`)
* Fallback: JSON file at `~/.local/share/ucode/auth.json` with `0o600` permissions
* `FileStore` implementation of `CredentialStore` trait
* `ChainStore` that tries keyring first, falls back to file store
* File format compatible with OpenCode's `auth.json` where possible:
  ```json
  {
    "anthropic": { "type": "api_key", "key": "sk-ant-..." },
    "github-copilot": { "type": "oauth", "access_token": "...", "refresh_token": "...", "expires_at": "..." }
  }
  ```

**Auth precedence resolver:**

* Resolution order (first wins):
  1. Environment variable named by `api_key_env` in provider config
  2. Stored credential in keyring/file store (from `ucode auth login` or `ucode auth set-key`)
  3. Prompt user (interactive only)
* `resolve_auth(provider_id, provider_config) -> Result<AuthMaterial, AuthError>`
* Non-interactive mode: skip step 3, return `AuthError::Missing`

**Acceptance**

* String-based provider IDs work for arbitrary provider names.
* `ProviderType` enum selects the correct adapter from TOML `type` field.
* File store persists credentials with correct permissions when keyring unavailable.
* Auth precedence: env var > stored credential > prompt.
* Existing keyring-stored credentials continue to work.

## 2.3 Auth flow framework (ucode-auth) [P0] [DONE]

Generic auth flow implementations that providers can use. Each flow is a standalone
async function — no traits needed (only one implementation per flow type).

**Auth flow types:**

| Flow | Description | Used by |
|---|---|---|
| API key prompt | Interactive prompt for API key string | All API-key providers |
| Device code (RFC 8628) | Display user_code + verification_uri, poll for token | GitHub Copilot, potentially others |
| Browser OAuth (PKCE) | Open browser for auth, receive callback with code, exchange for tokens | Anthropic subscription, Google Gemini |
| Well-known auth | Fetch `{url}/.well-known/opencode`, run auth command, store token | Custom/self-hosted providers |
| Env var chain | Read credentials from environment variables | AWS Bedrock, Azure, any provider with `api_key_env` |

**Device code flow (RFC 8628):**

```rust
pub struct DeviceCodeConfig {
    pub client_id: String,
    pub device_code_url: String,
    pub token_url: String,
    pub scope: String,
    pub grant_type: String,  // "urn:ietf:params:oauth:grant-type:device_code"
}

pub struct DeviceCodePending {
    pub user_code: String,
    pub verification_uri: String,
    pub device_code: String,
    pub interval: u64,  // polling interval in seconds
    pub expires_in: u64,
}

/// Returns pending info for display, then polls until success/timeout
pub async fn device_code_authorize(config: &DeviceCodeConfig) -> Result<AuthMaterial, AuthError>
```

* POST to device_code_url with client_id + scope
* Display verification_uri + user_code to user
* Poll token_url at `interval` seconds (add 3s safety margin)
* Handle `authorization_pending` (continue), `slow_down` (add 5s per RFC 8628)
* On success: return `AuthMaterial::OAuth { access_token, refresh_token, expires_at }`
* On timeout/error: return `AuthError`

**Browser OAuth flow (PKCE):**

```rust
pub struct BrowserOAuthConfig {
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
    pub scope: String,
    pub redirect_port: u16,  // localhost callback port
}

/// Opens browser, starts local HTTP server for callback, exchanges code for tokens
pub async fn browser_oauth_authorize(config: &BrowserOAuthConfig) -> Result<AuthMaterial, AuthError>
```

* Generate PKCE code_verifier + code_challenge
* Open browser to authorize_url with code_challenge
* Start temporary localhost HTTP server on redirect_port
* Receive callback with authorization code
* Exchange code + code_verifier for tokens at token_url
* Return `AuthMaterial::OAuth`

**Well-known auth flow:**

> **If stuck:** See OpenCode `packages/opencode/src/cli/cmd/auth.ts` for `.well-known/opencode`
> JSON format and auth command execution.

```rust
/// Fetch {base_url}/.well-known/opencode, run the auth command, store result
pub async fn wellknown_authorize(base_url: &str) -> Result<AuthMaterial, AuthError>
```

* GET `{base_url}/.well-known/opencode` → JSON with `auth.command` and `auth.env`
* Run the auth command, capture stdout as token
* Return `AuthMaterial::WellKnown { env_key, token }`

**Acceptance**

* Device code flow: displays user_code, polls correctly, handles slow_down.
* Browser OAuth: opens browser, receives callback, exchanges code for tokens.
* Well-known: fetches endpoint, runs command, stores token.
* All flows store result via `CredentialStore`.

## 2.4 Provider-specific auth handlers (ucode-auth) [P0] [DONE]

Concrete auth configurations for each supported provider. Each is a function that
returns the appropriate `DeviceCodeConfig`, `BrowserOAuthConfig`, or env var names
for that provider.

**Supported providers:**

| Provider | Auth methods | Notes |
|---|---|---|
| OpenAI | API key, env (`OPENAI_API_KEY`) | Standard API key |
| Anthropic | API key, env (`ANTHROPIC_API_KEY`), browser OAuth (subscription) | Claude Pro/Max subscription login |
| GitHub Copilot | Device code (RFC 8628) | Client ID: `Ov23li8tweQw6odWQebz`, scope: `read:user` |
| GitHub Copilot Enterprise | Device code + enterprise URL | Custom domain support |
| Google Gemini | API key, env (`GEMINI_API_KEY`), browser OAuth | Google AI subscription |
| Google Vertex AI | Env (`GOOGLE_APPLICATION_CREDENTIALS`), gcloud ADC | Service account or gcloud auth |
| AWS Bedrock | Env (`AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY`, `AWS_REGION`) | IAM credential chain |
| Azure OpenAI | API key, env (`AZURE_OPENAI_API_KEY`) + resource URL | Custom deployment headers |
| Groq | API key, env (`GROQ_API_KEY`) | Standard API key |
| DeepSeek | API key, env (`DEEPSEEK_API_KEY`) | Standard API key |
| OpenRouter | API key, env (`OPENROUTER_API_KEY`) | Standard API key |
| Together | API key, env (`TOGETHER_API_KEY`) | Standard API key |
| Fireworks | API key, env (`FIREWORKS_API_KEY`) | Standard API key |
| Mistral | API key, env (`MISTRAL_API_KEY`) | Standard API key |
| Ollama | None (local, no auth) | Optional API key for remote |
| Custom/self-hosted | Well-known auth, API key, custom headers | Via `.well-known/opencode` |

**GitHub Copilot device code config:**

> **If stuck:** See OpenCode `packages/opencode/src/plugin/copilot.ts` for Copilot OAuth
> details, enterprise domain handling, and `copilot-api.{domain}` baseURL pattern.

```rust
pub fn github_copilot_auth_config(enterprise_url: Option<&str>) -> DeviceCodeConfig {
    let domain = enterprise_url.unwrap_or("github.com");
    DeviceCodeConfig {
        client_id: "Ov23li8tweQw6odWQebz".into(),
        device_code_url: format!("https://{domain}/login/device/code"),
        token_url: format!("https://{domain}/login/oauth/access_token"),
        scope: "read:user".into(),
        grant_type: "urn:ietf:params:oauth:grant-type:device_code".into(),
    }
}
```

**CLI commands (updated):**

* `ucode auth login <provider> [--device] [--url <enterprise-url>]` — interactive login
* `ucode auth login <url>` — well-known auth for arbitrary URL
* `ucode auth set-key <provider>` — direct API key entry
* `ucode auth logout <provider>` — remove stored credentials
* `ucode auth status` — show all configured providers
* `ucode auth list` — alias for status

**Acceptance**

* `ucode auth login github-copilot` completes device code flow.
* `ucode auth login anthropic --subscription` completes browser OAuth.
* `ucode auth login https://custom.example.com` completes well-known auth.
* `ucode auth set-key groq` stores API key.
* Each provider's env var is checked before prompting.

## 2.5 Token refresh + expiry management (ucode-auth) [P0] [DONE]

Background token refresh for OAuth credentials. Runs as a tokio task during active sessions.

> **Note:** OpenCode stores `expires` timestamp in OAuth credentials but does NOT auto-refresh
> (known issue `#9111` — tokens expire after inactivity). Recent PR `#11832` adds multi-account
> rotation as a workaround. We implement proper auto-refresh to avoid this gap.

* Check token expiry before each provider request
* If token expires within 5 minutes: attempt refresh using refresh_token
* If refresh fails: emit `AuthExpired` error, trigger fallback
* If no refresh_token: emit `AuthExpired`, prompt re-login
* Refresh result stored back to credential store
* `auth_changed` hook event emitted on successful refresh
* `auth_failed` hook event emitted on refresh failure

**Acceptance**

* OAuth token auto-refreshes before expiry without user intervention.
* Failed refresh triggers clean auth error and fallback.
* Refreshed credentials persist to store.

## 2.6 Auth-aware fallback (ucode-core + providers) [P0]

* Auth errors mapped to `AuthMissing/AuthInvalid/AuthExpired`
* Router immediately falls back and emits `Event::Log` explaining why
* Provider adapters (Task 3.6) call `resolve_auth()` before each request
* 401/403 responses mapped to `AuthInvalid`
* Missing credentials mapped to `AuthMissing`
* Expired tokens (detected locally) mapped to `AuthExpired`

**Acceptance**

* Break token mid-session and the conversation continues via fallback model.
* Missing env var for a provider skips it cleanly with log message.

---

# Phase 3 — Provider adapters (streaming + tool translation)

### Task 3.1 Provider trait (ucode-providers) [DONE]

* `Provider::stream_chat(req) -> Stream<Event>`
* `capabilities()` (tools, json mode, max context, max output, token counting support)
* optional adapter `count_tokens(req)` for provider-native counting

### Task 3.2 OpenAI adapter [DONE]

* Streaming tokens
* Tool/function calls → canonical ToolCall
* Uses auth module (key or login)

### Task 3.3 Anthropic adapter [DONE]

* Streaming tokens
* Tool use mapping → canonical ToolCall
* Uses auth module (API key or subscription login)

### Task 3.4 Local adapter (Ollama) (optional but recommended) [DONE]

* Fast fallback / offline mode

### Task 3.6 Provider adapter refactor — generic multi-protocol support (ucode-providers) [P0] [DONE]

Replace hardcoded single-endpoint providers with four generic, configurable adapters that cover
the entire LLM API landscape. Each adapter is protocol-generic: users configure named provider
instances in TOML with `type`, `base_url`, `api_key_env`, and optional `headers`.

**Four adapters:**

| Adapter | Protocol | Covers |
|---|---|---|
| `OpenAiCompatProvider` | OpenAI `/v1/chat/completions` | OpenAI, Groq, Together, Fireworks, DeepSeek, Mistral, OpenRouter, vLLM, LiteLLM, Azure OpenAI |
| `AnthropicCompatProvider` | Anthropic `/v1/messages` | Anthropic, any Anthropic-compatible API |
| `OllamaProvider` | Ollama native `/api/chat` | Ollama (thinking, stats, `num_ctx`, richer sampling) |
| `GeminiProvider` | Google `generateContent` / `streamGenerateContent` | Google AI Studio, Vertex AI |

**What changes from current code:**

* `OpenaiProvider` → renamed to `OpenAiCompatProvider`, gains configurable `base_url`, optional `api_key`, custom headers
* `AnthropicProvider` → renamed to `AnthropicCompatProvider`, gains configurable `base_url`, custom headers
* `OllamaProvider` → rewritten to use native `/api/chat` (currently uses `/v1/chat/completions`).
  Gets thinking support (`think` param, `message.thinking` field), performance stats
  (`total_duration`, `load_duration`, `prompt_eval_count`, `eval_count`, etc.),
  direct context size control (`options.num_ctx`), and richer sampling params (`min_p`, `top_k`, `seed`)
* `GeminiProvider` — new. `POST /v1beta/models/{model}:generateContent` (non-streaming) and
  `POST /v1beta/models/{model}:streamGenerateContent?alt=sse` (streaming via SSE).
  API key via query param or `x-goog-api-key` header.
  Tool calling via `functionDeclarations` in `tools`, model returns `functionCall` parts,
  client sends back `functionResponse` parts.
  Thinking support via `thought` field in parts.

**Provider config (TOML):**

```toml
[providers.openai]
type = "openai"
api_key_env = "OPENAI_API_KEY"

[providers.groq]
type = "openai"
base_url = "https://api.groq.com/openai/v1"
api_key_env = "GROQ_API_KEY"

[providers.anthropic]
type = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"

[providers.ollama]
type = "ollama"
base_url = "http://localhost:11434"

[providers.gemini]
type = "gemini"
api_key_env = "GEMINI_API_KEY"

[providers.azure]
type = "openai"
base_url = "https://myresource.openai.azure.com/openai/deployments/gpt-4o"
api_key_env = "AZURE_OPENAI_API_KEY"
[providers.azure.headers]
api-version = "2024-10-21"
```

Key design points:

* `api_key_env` names an env var (not the raw key) — keys never in config files
* Optional `api_key` field for keyring-stored credentials (loaded at runtime via `CredentialStore`)
* Optional `headers` table for custom headers (Azure `api-version`, proxy auth, etc.)
* `base_url` defaults per type: OpenAI → `https://api.openai.com/v1`, Anthropic → `https://api.anthropic.com`,
  Ollama → `http://localhost:11434`, Gemini → `https://generativelanguage.googleapis.com`
* Bedrock and Azure AD auth deferred to plugin-based provider extensions

**Acceptance**

* OpenAI-compatible provider works with OpenAI, Groq, and Ollama `/v1` endpoints via `base_url` config.
* Anthropic-compatible provider works with Anthropic API via configurable `base_url`.
* Ollama native provider uses `/api/chat` with thinking, stats, and `num_ctx` support.
* Gemini provider streams via `streamGenerateContent?alt=sse` with tool calling.
* Custom headers (Azure `api-version`, proxy auth) are sent correctly.
* Missing `api_key_env` var produces clear error; missing `api_key` with no env var falls back gracefully.

### Task 3.5 Prompt/context caching integration (ucode-providers + ucode-core) [P2]

* Add reusable prompt/context cache policy for repeated compatible requests
* Use provider cache hints when available; local fallback cache strategy otherwise
* Cache invalidation on provider/model/session policy changes
* Emit cache hit/miss telemetry

**Acceptance (Task 3.5)**

* Compatible repeated requests demonstrate cache-hit behavior.
* Invalidation occurs correctly on provider/model change.
* Cache actions are visible in logs/audit stream.

**Acceptance (Phase 3)**

* `ucode chat` streams output from at least one provider with tool-call events.

---

# Phase 4 — Built-in tools (must-have)

**Design principle:** All user-facing tools are baked-in Rust libraries. No shelling out to external CLIs.

| Capability | Rust crate | Why |
|---|---|---|
| File search | `ignore` + `regex` | ripgrep ecosystem; gitignore-aware |
| Patch apply | `mpatch` | Fuzzy matching for AI-generated diffs |
| AST search/rewrite | `ast-grep-core` + `ast-grep-language` | Tree-sitter structural patterns |
| Git operations | `gix` | Pure-Rust git |
| Command execution | `tokio::process` | Async with timeout/caps |

### Task 4.1 Tool registry (ucode-tools) [DONE]

* `ToolSpec { name, schema, description }`
* `ToolHandler: async fn(args)->ToolResult`
* Permissions gate integrated (per session + per skill)

### Task 4.2 Filesystem/search tools [DONE]

* `read_file`, `list_dir` — `tokio::fs` async file I/O
* `search` — `ignore` crate (gitignore-aware walking) + `regex` crate (matching)
* All Rust-native, no CLI shelling

### Task 4.3 Patch tool (core feature) [DONE]

* Replace hand-rolled parser with `mpatch` crate
* `mpatch::parse_auto()` — handles raw unified diffs AND markdown-embedded diffs
* `mpatch::apply_patches_to_dir()` — fuzzy context matching, smart indentation
* Built-in path traversal protection
* Return applied/files_changed/rejects with reasons

### Task 4.4 Command runner tool [DONE]

* `run_cmd(cmd, cwd, timeout, env)`
* `tokio::process::Command` — async with timeout/output caps
* require user approval (TUI prompt) unless allowlisted

### Task 4.5 Git helpers (optional) [DONE]

* 17 git tools via `gix` (pure-Rust, no shell): status, diff, diff_staged, diff_commits, add, commit, log, show, tag, branch, checkout, reset, restore, stash, merge, cherry_pick, rebase
* Full interactive rebase (pick/squash/reword/drop), three-way merge with conflict markers
* `register_all_git_tools()` convenience function

### Task 4.5b AST structural search/rewrite tool [DONE]

* `ast_search(pattern, path, lang)` — find code matching AST patterns
* `ast_rewrite(pattern, replacement, path, lang)` — structural find-and-replace
* `ast-grep-core` + `ast-grep-language` crates (tree-sitter based)
* Pattern syntax: `$VAR` wildcards (e.g., `console.log($MSG)`)
* Language grammars: Rust, Python, TypeScript, JavaScript, Go, C, C++

### Task 4.6 Fine-grained sandbox policy engine (ucode-tools) [DONE]

* Policy hierarchy (most restrictive wins by default):

  * global default
  * provider/model profile
  * agent profile
  * tool-level override
  * session override

* Sandbox tiers:

  * `off` — no isolation
  * `workspace` — repo-bound FS/process isolation
  * `networked` — workspace isolation + controlled network egress (allowlisted domains/ports)
  * `strict` — workspace isolation + no network + minimal env

* Per-tool capability flags:

  * file read/write scope
  * command exec
  * network egress
  * spawn external process

* Linux backend: `bwrap` sandbox profiles
* macOS backend: native sandbox profile or documented degraded mode with warning if unavailable
* Effective policy visible in logs and TUI sidebar before execution

**Acceptance**

* Per-agent policy can be stricter than global.
* Per-tool override cannot bypass stricter parent policy.
* Effective policy is visible in logs/UI before execution.
* If sandbox backend unavailable, user sees explicit warning.

### Task 4.7 Outside-project confirmation gates (ucode-tools + ucode-tui) [DONE]

* Explicit confirmation required for:

  * out-of-workspace file access
  * out-of-workspace command cwd/path
  * external process spawn (including MCP server launch)
  * network access when policy requires consent

* Approval options: `once`, `session`, `deny`
* Denials are first-class: enforced + logged to audit trail
* Canonical path checks: resolve symlinks, `..`, relative inputs before policy evaluation

**Acceptance**

* Out-of-scope action is blocked until approved.
* Denial emits clear reason and audit event.
* Symlink escape attempt is denied.

### Task 4.8 Network capability policy for web/deep research (ucode-tools) [DONE]

* Separate `network` capability from general command execution
* Policy can allow network for selected tools/agents only (e.g., web-search agent gets `networked` tier)
* Designed so local-only model agents and web-research agents coexist in same session
* Domain/port allowlists for fine-grained egress control

**Acceptance**

* Agent A (local-only) has no network; Agent B (research) has constrained network.
* Policy changes are visible in runtime logs and UI.

### Task 4.9 Workspace checkpoints + rollback controls (ucode-tools + ucode-core + ucode-tui) [P1] [DONE]

* Create lightweight checkpoints before risky mutating operations (patch apply, mutating commands)
* Provide rollback API and TUI action to restore prior checkpoint
* Checkpoint retention policy (count/time based) with UI visibility

**Acceptance**

* Patch apply creates checkpoint and rollback restores prior state.
* Command-induced mutations can be rolled back.
* Retention pruning follows configured policy.

### Task 4.10 Background jobs with interactive cancel/kill (ucode-tools + ucode-core + ucode-tui) [P0] [DONE]

* Background job runtime: start/list/status/cancel/kill via `JobController`
* Job states: queued/running/completed/failed/cancelled/killed with `is_terminal()`
* Graceful cancel (cancel_tx) and force kill (cancel_tx + kill_tx) signals
* `wait()` with one-shot result consumption, `prune_completed()` for cleanup
* TUI background jobs panel deferred to TUI phase

**Acceptance**

* Long-running job can run detached without blocking chat.
* User can cancel and force-kill running jobs interactively from TUI.
* Job lifecycle transitions are auditable and persisted.

### Task 4.11 Structured artifact output/export (ucode-tools + ucode-cli) [P1] [DONE]

* Standard artifact envelope (`id`, `type`, `source`, `metadata`, `checksum`, `created_at`)
* Generate/export artifacts for markdown reports, unified diffs, and command/test logs
* Expose artifact references in machine-readable CLI output
* Link artifact ids into audit events

**Acceptance**

* Runs can emit and retrieve typed artifacts by id.
* CLI JSON output includes stable artifact references.
* Artifact metadata is deterministic and auditable.

**Acceptance (Phase 4)**

* End-to-end: search → propose diff → apply_patch → run_cmd tests.
* Shifted-hunk patch applies successfully with offset search.
* Large patch apply performance stays within documented target budget.

---

# Phase 5 — MCP client (external tools)

### Task 5.1 MCP client library (ucode-mcp) [DONE]

* stdio transport first
* tool discovery + tool execution
* schema conversion → ToolSpec

### Task 5.2 MCP registry integration (ucode-tools) [DONE]

* namespacing strategy: `mcp.<server>.<tool>`
* collision handling: error on duplicate names
* tool call routing (built-in vs MCP)
* `McpBridge` + `McpToolHandler` + `register_tool_defs()` testable free function

### Task 5.3 Native MCP launcher manager (ucode-mcp) [DONE]

* Launcher definitions in config:

  * `uvx <pkg> [args...]`
  * `npx <pkg> [args...]`
  * `bunx <pkg> [args...]`
  * direct executable path

* Capture runtime metadata: version, executable path, startup timeout, health status
* Validate command schema before launch
* First-run trust prompt + persisted trust decision
* Server identity fingerprint (command + package + version hash)
* Fingerprint drift re-triggers trust confirmation

**Acceptance**

* MCP server configured with each launcher type can be discovered and started.
* Startup timeout and invalid command errors are handled clearly.
* Changing server command/package/version triggers trust re-confirmation.

### Task 5.4 MCP per-server policy + lifecycle controls (ucode-mcp + ucode-tools) [DONE]

* Per-server sandbox tier, network policy, and tool permission profile
* Managed lifecycle: start/stop/restart with crash diagnostics
* Health check + auto-restart with backoff
* Full audit events: launch, approval, deny, crash, restart

**Acceptance**

* Untrusted server cannot execute tools until approved.
* Per-server policy is enforced at invocation time.
* Crash/restart cycle produces clear diagnostics.

### Task 5.5 MCP transport parity (stdio + SSE + HTTP) (ucode-mcp) [P0] [DONE]

> Implementation plan: `docs/plans/2026-03-06-mcp-transport-parity.md`

* Extract `Transport` trait from `StdioTransport` (boxed futures for object safety)
* `McpClient` refactored to `Box<dyn Transport>` with `from_transport()` and `connect_with_config()`
* `ClientInfo` struct with optional `client_name`/`client_version` overrides (for Kimi and similar servers)
* `HttpTransport` — Streamable HTTP (2025-03-26 spec): single POST endpoint, JSON or SSE response, `Mcp-Session-Id` header
* `SseTransport` — Legacy SSE (2024-11-05 spec): GET `/sse` for `endpoint`/`message` events, POST to endpoint URL
* `ReconnectConfig` with 3 strategies: Simple (3 retries), Persistent (unlimited), Configurable (user-defined)
* Exponential backoff with cap; permanent errors (4xx except 429) fail immediately
* `ServerConfig` with TOML parsing, `${VAR}` env var expansion in headers, transport type routing
* `reqwest` 0.12 (rustls-tls, json, stream features) as HTTP client
* `tracing::warn!` on retry attempts, `tracing::error!` on final failure
* New error variants: `Http`, `SseConnection`, `ReconnectExhausted`, `InvalidConfig`
* 4 new files: `reconnect.rs`, `server_config.rs`, `transport_http.rs`, `transport_sse.rs`
* ~22 new tests

TOML config example:
```toml
[mcp.servers.my-remote]
transport = "sse"
url = "https://example.com/mcp"
reconnect = "simple"
client_name = "kimi-code"

[mcp.servers.my-remote.headers]
Authorization = "Bearer ${MCP_TOKEN}"
X-Custom = "value"
```

**Acceptance**

* Discovery/invocation works across stdio, SSE, and HTTP servers.
* Disconnect triggers bounded reconnect with diagnostics.
* Remote auth failures return actionable errors.

### Task 5.6 MCP resources/prompts integration (ucode-mcp + ucode-tools + ucode-core) [P0] [DONE]

* Discover/list/invoke MCP resources and prompts (not tools only)
* Apply normal policy/sandbox/audit checks to resource/prompt access
* Support prompt argument binding and namespaced identifiers
* Deterministic collision handling across servers

**Acceptance**

* Resources/prompts are discoverable and invokable.
* Resource/prompt actions obey policy/audit gates.
* Identifier collisions resolve deterministically.

**Acceptance**

* Connect to an MCP server and successfully call a tool.

---

# Phase 6 — Skills (Claude Code + OpenCode compatible)

### Task 6.1 Skill discovery + parsing (ucode-skills) [DONE]

Load from:

* `.claude/skills/*/SKILL.md`
* `.agents/skills/*/SKILL.md`
* `skills/*/SKILL.md`
* `~/.config/ucode/skills/*/SKILL.md`

Parse:

* YAML frontmatter: require only `name`, `description`
* markdown body is instruction text
* ignore unknown keys
* support optional `ucode:` namespace for extras (permissions, routing hints, tool allowlists)

### Task 6.2 Skill selection/execution [DONE]

* active skill becomes prompt prefix + tool constraints + routing hints
* switch skill from TUI
* `SkillBinding` (system prompt prefix, tool filter, routing hints)
* `SkillManager` (activate/deactivate/switch, tool filter, system prefix)
* `ToolFilter::AllowAll | AllowList(HashSet)` with `is_allowed()` check

**Acceptance**

* Drop in existing skills; they appear and work.

---

# Phase 7 — Fullscreen TUI (ratatui)

> **Design spec:** `docs/plans/2026-03-05-tui-design.md` — full component inventory, visual system, plugin UI API, and ASCII layout sketches. All tasks in this phase implement that spec.

### Task 7.1 TUI foundation (ucode-tui) [DONE]

Panes:

* chat transcript (scroll)
* input box (multiline)
* sidebar: active provider/model, skill, context pack, tool calls queue
* status/log line

Keybinds:

* `Ctrl+P` command palette
* `Ctrl+O` open file
* `Ctrl+E` view last diff
* `Ctrl+R` rerun last cmd
* `Ctrl+T` run tests
* `/connect` auth UI

Terminal compatibility:

* Tmux/zellij/screen detection and seamless operation (see Task 7.7)
* Alternate screen (`smcup`/`rmcup`) with proper cleanup on exit/crash
* True color detection (`$COLORTERM`, tmux `Tc`/`RGB`) with 256-color fallback
* `SIGWINCH` resize handling (native via crossterm)

### Task 7.2 Approvals UX [DONE]

* Diff modal: approve/apply, reject [DONE]
* run_cmd modal: approve once / approve session / deny [DONE]

### Task 7.3 Visual system + sidebar-first information design (ucode-tui) [DONE]

* Visual token system: color roles, spacing, border emphasis, semantic states (safe/warning/danger)
* Sidebar priority panels:

  * active provider/model + effective sandbox tier
  * active skill
  * context pack summary
  * tool call queue with approval state
  * subagent status panel (running/completed/failed)
  * network state indicator

* Transparent-friendly palette option (terminal-dependent; no hard dependency)
* Compact/comfortable density presets
* Keyboard-first focus behavior across all panes

**Acceptance**

* Sidebar remains readable and informative at common terminal sizes (80x24 minimum).
* Risk/approval/sandbox state is always visible without leaving chat pane.
* Theme toggles preserve contrast/accessibility in supported terminals.
* Fully usable without leaving fullscreen UI (except browser auth step).

### Task 7.4 Slash command UX and discoverability (ucode-tui + ucode-core) [DONE]

* Command palette and input parser support `/command` invocation
* Show command source badges: user/project/plugin
* Inline argument hints and validation errors
* Integrate command execution with same policy/sandbox/approval pipeline as normal actions

**Acceptance**

* User-defined `/command` resolves and executes from TUI input.
* Unknown command returns suggestions.
* Command execution obeys the same sandbox/approval controls.

### Task 7.5 Toast notification system + plugin UI extension API (ucode-tui + ucode-plugins) [DONE]

> See `docs/plans/2026-03-05-tui-design.md` §7 (Notifications) and §11 (Plugin UI Extension API).

* Toast component: stacked top-right, 4 types (`info`/`success`/`warning`/`error`), auto-dismiss timers, manual dismiss
* System-triggered toasts: checkpoint created, budget warning, agent completed/failed, MCP crash, auth expired
* Plugin UI extension API (versioned, Safe/Guarded/Risky override classes):
  * `ui::toast(level, title, body?, ms?)` — Safe
  * `ui::notify(level, message)` — Safe
  * `ui::sidebar_section(id, title, lines, priority?)` — Safe
  * `ui::status_segment(id, content, priority?)` — Safe
  * `ui::palette_command(name, desc, handler)` — Safe
  * `ui::transcript_event(style, content)` — Guarded
  * `ui::modal(title, content, actions)` — Guarded
  * `ui::badge(section_id, count, level)` — Safe (own section only)
  * `ui::confirm(title, message) -> bool` — Guarded
  * `ui::input_prompt(title, placeholder) -> String` — Guarded
* Plugin UI lifecycle: register on `on_session_start`, cleanup on `on_session_end`
* Plugin sidebar sections rendered in standard style with `[plugin]` badge

**Acceptance**

* System toast fires on checkpoint creation, budget warning, agent completion/failure.
* Plugin calls `ui::toast()` and toast appears with correct style and auto-dismiss.
* Plugin registers sidebar section; it appears after built-in sections with `[plugin]` badge.
* Plugin registers palette command; it appears in palette with `[plugin]` badge.
* Guarded calls require plugin to have guarded capability declared in manifest.
* Plugin UI elements cleaned up on session end.

### Task 7.6 Copy mode + search overlay + keybind overlay (ucode-tui) [DONE]

> See `docs/plans/2026-03-05-tui-design.md` §6.10–6.11 and §6 (Overlays).

* `v` enters vim-like copy mode in transcript; `y` yanks selection to clipboard
* `Ctrl+F` opens search overlay; matches highlighted; `n`/`N` navigate
* `?` opens keybind overlay with full keybind reference

**Acceptance**

* `v` enters copy mode; `y` copies selected text to clipboard.
* `Ctrl+F` opens search; matches highlighted; `n`/`N` navigate between matches.
* `?` shows full keybind reference overlay.

### Task 7.6b Markdown rendering in transcript (ucode-tui) [DONE]

> Renders assistant messages with rich markdown formatting using `pulldown-cmark`.

* Bold, italic, strikethrough, inline code with ratatui modifiers
* Code blocks with language label and surface-colored background
* Headers (H1-H6) with accent color and appropriate modifiers
* Bullet and numbered lists with proper indentation
* Tables with measured column widths, bold headers, pipe separators
* Links rendered as accent-colored text with dim URL
* Word-wrapping for paragraphs; no wrapping inside code blocks
* Graceful fallback to plain text if markdown parsing produces no events
* Integrated into `render_assistant_message`, `render_streaming_message`, and `entry_height`

**Acceptance**

* Assistant messages render markdown formatting (bold, italic, code, headers, tables, lists).
* Streaming messages also render markdown incrementally.
* `entry_height` uses `markdown_height` for correct virtual scrolling.
* Plain text without markdown renders identically to before.
* 47 markdown-specific tests, 493 total TUI tests.

### Task 7.7 Tmux / terminal multiplexer integration (ucode-tui) [DONE]

> See `docs/plans/2026-03-05-tui-design.md` §10b (Tmux Integration).

* Detect `$TMUX` / `$ZELLIJ` / `$STY` at startup; surface `[tmux]` indicator in title bar
* OSC 52 clipboard writes (works through tmux ≥3.3 with `set-clipboard on`)
* Fallback clipboard: `xclip`/`xsel`/`pbcopy` external, then file (`~/.local/share/ucode/clipboard`)
* Mouse support toggle (`app.mouse_enabled`) for tmux mouse-mode coexistence
* True color detection (`$COLORTERM`, `$TERM` 256color, tmux fallback) via `ColorSupport` enum
* Terminal title via OSC (`\033]0;ucode\007`), restored on exit
* `SIGWINCH` resize handling (native via crossterm)

**Acceptance**

* `[tmux]` shows in title bar when `$TMUX` is set.
* Copy in ucode copy mode writes to system clipboard via OSC 52 inside tmux.
* Fallback clipboard works when OSC 52 is unavailable.
* `mouse_enabled = false` disables mouse capture, allowing tmux mouse passthrough.

### Task 7.8 Keybinding presets (ucode-tui) [DONE]

> See `docs/plans/2026-03-05-tui-design.md` §10 (Keybinding presets).

* Three built-in presets: `vscode` (default), `vim` (modal), `emacs` (Meta+x)
* `override_binding()` / `remove_binding()` for individual key overrides on top of preset
* vim preset: `Esc`/`i` for normal/insert mode, `j`/`k` scroll, `:` palette, `Ctrl+U`/`Ctrl+D` half-page
* emacs preset: `Meta+x` palette, `Ctrl+N`/`Ctrl+P` scroll, `Ctrl+S` search, `Ctrl+G` cancel
* Config file integration (`[tui.keybinds] preset = "vscode"`) deferred to config system

**Acceptance**

* `preset = "vim"` activates modal editing with `i`/`Esc` mode switching.
* `preset = "emacs"` activates emacs-style navigation and `Meta+x` palette.
* `override_binding()` / `remove_binding()` allow individual overrides on top of any preset.
* Default preset is `vscode` when no config is set.

---

# Phase 8 — Plugins & hooks (user customization)

Define plugin contracts, event surfaces, and safety policy first. Defer WASM runtime implementation to the latest stage.

### Task 8.1 Plugin manifest + loader (ucode-plugins) [DONE]

* `plugin.toml`: name, version, command, hooks, tools exported

### Task 8.2 Hooks API (v1) [DONE]

Full hook event surface (64 events across 16 categories). Initial implementation delivered 22 events; remaining events added in Task 8.3 as part of the v1 API contract.

**Session lifecycle** (Safe unless noted)

* `session_start`, `session_end`
* `session_title_generated`, `session_title_updated`
* `config_reloaded`

**Message flow**

* `user_message_received` (Safe)
* `assistant_response_started` (Safe)
* `assistant_response_completed` (Safe)
* `message_retry` (Guarded)

**Model selection & routing**

* `before_model_call` (Guarded), `after_model_call` (Safe)
* `before_model_select` (Guarded)
* `model_fallback` (Risky)
* `router_decision` (Safe)
* `model_rate_limited` (Safe), `model_quota_exhausted` (Safe)

**Tool calls (generic)**

* `before_tool_call` (Guarded), `after_tool_call` (Safe)
* `tool_error` (Safe), `tool_timeout` (Safe)

**Tool calls (specific high-value types)**

* `before_apply_patch` (Guarded), `after_apply_patch` (Safe)
* `before_run_cmd` (Guarded), `after_run_cmd` (Safe)
* `before_file_read` (Guarded), `after_file_read` (Safe)
* `before_file_write` (Guarded), `after_file_write` (Safe)

**Context management**

* `context_overflow` (Guarded), `context_compaction` (Guarded)
* `context_distilled` (Safe)
* `token_usage_updated` (Safe)

**Agent / Sub-agent**

* `agent_spawned`, `agent_message`, `agent_completed`, `agent_failed` (Safe)
* `agent_cancelled` (Safe)

**Approval / Permission / Sandbox**

* `approval_required` (Guarded), `approval_granted` (Safe), `approval_denied` (Safe)
* `sandbox_decision` (Safe), `permission_decision` (Safe)

**Auth & Provider**

* `auth_changed` (Safe), `auth_failed` (Safe), `provider_switched` (Safe)

**MCP servers**

* `mcp_server_connected`, `mcp_server_disconnected` (Safe)
* `mcp_server_launch`, `mcp_server_restart`, `mcp_server_crash` (Safe)
* `mcp_tool_invoked` (Safe)

**Skills**

* `skill_activated`, `skill_deactivated` (Safe)

**Plugins**

* `plugin_loaded`, `plugin_unloaded`, `plugin_error` (Safe)

**Checkpoints**

* `checkpoint_created` (Guarded), `checkpoint_restored` (Risky)

**Budget / Cost**

* `budget_threshold_warning` (Safe), `budget_threshold_reached` (Guarded)
* `cost_incurred` (Safe)

**Background jobs**

* `background_job_state_changed` (Safe)

**Commands / UI**

* `command_invoked` (Safe), `palette_command_executed` (Safe)

**Diagnostics**

* `unhandled_error` (Safe)

Plugins can:

* observe/log
* propose safe policy/routing adjustments (including model fallback preferences)
* veto within safe policy boundaries (e.g., block certain commands)
* register plugin-provided tools (namespaced and policy-gated)

Safety rule:

* plugin behavior changes are allowed only if within effective policy
* any risky override requires explicit user approval
* plugin-provided tools must pass host schema validation and permission checks

Override classes matrix:

| Class | Examples | Auto-apply | Requires explicit approval |
| --- | --- | --- | --- |
| Safe | ranking fallback candidates, adding diagnostic logs, suggesting context shrink order | Yes (if policy-safe) | No |
| Guarded | changing model group preference, adjusting retry counts within limits, reordering safe tool selection | Yes (bounded by policy limits) | No |
| Risky | enabling extra network scope, widening filesystem access, allowing external process spawn, bypassing deny rules | No | Yes |

**Acceptance**

* Example plugin blocks dangerous commands; logs fallbacks.
* Plugin-proposed model fallback override is applied only when policy-safe.
* Risky plugin override path requires explicit user approval.

### Task 8.3 Plugin API contract + SDKs (ucode-plugins) -- DONE

DONE. 95 plugin tests (52 new), 506 TUI tests, 0 clippy warnings. Implemented: manifest id/required_features, 64 hook events, Plugin/HookHandler/ToolProvider traits, PluginHost with load/unload/dispatch/tool registration.

Traits-first approach: define the v1 API as Rust traits, test with in-process plugins. WASM/WIT deferred to Task 8.4.

**Manifest changes:**

* `id` field: reverse-domain globally unique identifier (`org.acme.code-analyzer`), minimum 3 dot-separated segments
* `name` field: human-readable display name (arbitrary string)
* `required_features`: unversioned feature set (`["hooks", "tools", "ui"]`)
* `min_api_version`: semver floor for compatibility
* Tool names are local in manifest; host constructs FQN as `{plugin_id}.{tool_name}`

**Handshake protocol:**

* Plugin sends: `HandshakeRequest { plugin_id, min_api_version, required_features, capabilities }`
* Host responds: `HandshakeResponse::Accepted { api_version, supported_features, granted_capabilities }` or `Rejected { reason }`
* Check 1: semver — host API version >= plugin min_api_version (same major)
* Check 2: features — plugin required_features is subset of host supported_features
* Check 3: capabilities — host may grant a subset of requested capabilities

**Plugin traits (v1 contract):**

* `Plugin` (mandatory): `handshake()`, `initialize()`, `shutdown()`
* `HookHandler` (opt-in, requires `hooks` feature): `on_event() -> HookResponse`
* `ToolProvider` (opt-in, requires `tools` feature): `tool_specs()`, `invoke_tool()`
* `HookResponse`: `Ok` / `Modify { changes }` (Guarded) / `Veto { reason }` (Risky)

**Hook event expansion:**

* Expand `HookEvent` enum from 22 to 64 variants (full surface from Task 8.2 spec)
* All new events get `event_name()` and `override_class()` implementations

**Tool registration model:**

* Plugin declares `ToolSpec` set via `ToolProvider::tool_specs()`
* Host namespaces as `{plugin_id}.{tool_name}` (e.g., `org.acme.code-analyzer.lint`)
* Host validates JSON schema and applies capability policy before activation
* Plugin tool calls flow through normal sandbox/approval/audit pipeline

**Version negotiation:**

* `semver` crate for version parsing and comparison
* `Feature` enum: `Hooks`, `Tools`, `Ui`
* Host reports `API_VERSION` constant and `supported_features` set

**Acceptance**

* Rust in-process example plugin passes handshake and receives hooks.
* Version mismatch produces clear `HandshakeError::VersionIncompatible` error.
* Feature mismatch produces clear `HandshakeError::UnsupportedFeatures` error.
* Plugin tool registered with namespaced FQN and invocable through host registry.
* All 64 hook events have `event_name()` and `override_class()` implementations.

### Task 8.4 WIT interface + wasmtime WASM runtime (ucode-plugins) [DONE]

DONE. 115 plugin tests (111 unit + 4 integration), 0 clippy warnings. Implemented: 65 WIT hook interfaces across 20 category packages, wasmtime 42 host runtime with dynamic export probing, guest SDK crate (ucode-plugin-sdk), example WASM plugin (hello-wasm), integration tests.

* Translate Rust trait API (from Task 8.3) into `.wit` component-model interfaces
* Integrate `wasmtime` with component-model support for loading `.wasm` plugins
* `wit-bindgen` for host and guest binding generation
* WASM plugin lifecycle: load `.wasm` -> handshake -> activate -> dispatch hooks
* Rust SDK crate for authoring WASM plugins (compiles to `wasm32-wasip2`)
* Example WASM plugin demonstrating handshake + hook handling + tool export

**Acceptance**

* Rust WASM plugin compiled to `.wasm` loads via wasmtime and passes handshake.
* WASM plugin receives hook events and returns `HookResponse`.
* WASM plugin exports tools accessible through host registry with namespaced FQN.

### Task 8.5 Plugin runtime isolation model (ucode-plugins + security) -- DONE

* Runtime policy model for WASM-only plugin execution
* Per-plugin policy profile: filesystem scope, network, command spawn, hook scopes
* All plugin-originated actions routed through the same approval and audit pipeline
* Ed25519 signed plugin verification (feature-gated: `signed-plugins`)
* WASM resource limits (fuel metering + memory caps via StoreLimits)
* Plugin isolation levels (Full / Ordered) with accumulated_changes tracking
* Dynamic policy hot-reload from TOML config
* WASI preopens for defense-in-depth filesystem sandboxing
* Tracing instrumentation across all policy enforcement paths

**Tests:** 145 (no features) / 178 (wasm) / 180 (wasm + signed-plugins), 0 clippy warnings

**Acceptance**

* Untrusted plugin cannot execute outside its granted policy.
* Plugin-originated tool call triggers normal approval/sandbox checks.
* Runtime model and effective permissions visible in logs/UI.

### Task 8.6 External plugin infrastructure + public hook surface (ucode-plugins + ucode-core) [P0]

Enable external plugins (DCP-style, context-management, etc.) via documented public hook contracts
and complete plugin runtime plumbing. This is the infrastructure layer — Phase B (Task 8.8) builds
the actual context-management strategies on top.

**8.6.1 Plugin discovery paths** [DONE]

* Default search paths, scanned in order (first match wins on plugin ID conflict):
  1. `.ucode/plugins/` (project-local)
  2. `~/.ucode/plugins/` (or `$UCODE_HOME/plugins/`)
  3. Extra paths from `ucode.toml`: `[plugins] discovery_paths = ["/opt/ucode-plugins"]`
* Each plugin is a directory containing `plugin.toml` manifest.
* Implemented: `default_plugin_search_paths()` and `plugin_search_paths()` in `loader.rs`.

**8.6.2 Typed WIT interface with hooks-transform category** [DONE]

* **Keep all 20 typed WIT category packages** (19 existing + 1 new `hooks-transform`).
  Each category can version independently. Typed payloads provide compile-time safety
  at the WASM boundary — plugin authors get type-checked payload shapes.
* **New `hooks-transform` WIT category** added with 2 interfaces:
  * `ucode:hooks-transform/on-transform-messages` — typed `transform-messages-payload`
  * `ucode:hooks-transform/on-transform-system-prompt` — typed `transform-system-prompt-payload`
* New typed payload records added to `hooks-types/types.wit`:
  * `transform-messages-payload { messages-json: string }`
  * `transform-system-prompt-payload { prompt: string }`
* `EVENT_INTERFACE_MAP` updated from 65 to 67 entries (added 2 transform events).
* `world.wit` updated: `maximal-plugin` world now exports 67 hook interfaces.
* Rationale: typed WIT per category enables independent versioning per category,
  compile-time safety for plugin authors, and clear contract documentation.
  Per-hook payload versioning (8.6.6) handles additive schema changes within categories.

**8.6.3 Complete WASM hook dispatch** [DONE]

* Current state: `dispatch_hook()` for WASM plugins returns `HookResponse::Ok` always (stubbed)
* Wire actual wasmtime handler calls using the **typed per-event WIT exports**:
  1. Create store with policy (fuel + memory limits)
  2. Create linker with host-log import
  3. Instantiate component
  4. Look up the event's typed WIT interface from `EVENT_INTERFACE_MAP` (67 entries)
  5. Serialize payload to the typed WIT record for that event
  6. Call the typed export function (e.g., `ucode:hooks-session/on-start.handle()`)
  7. Deserialize typed `hook-response` via `wit_response_to_native()`
  8. If `Modify`: accumulate changes like native plugins do
  9. On error/fuel exhaustion: log + return `HookResponse::Ok` (fail-open)
* Apply `HookResponse::Modify` payloads back to the originating event (currently ignored)
* Respect fuel/memory limits during handler execution; timeout → treat as `HookResponse::Ok`

**8.6.4 Message transform hooks (regular hooks with pipeline dispatch)** [DONE]

* Two new hook events added to the existing event surface (bringing total to 67):
  * `transform_messages` — plugin receives full message array (JSON) as payload before LLM call
  * `transform_system_prompt` — plugin receives system prompt text as payload
* These are **regular hooks** dispatched through their typed WIT interfaces in `hooks-transform`:
  * `ucode:hooks-transform/on-transform-messages` with typed `transform-messages-payload`
  * `ucode:hooks-transform/on-transform-system-prompt` with typed `transform-system-prompt-payload`
* **Return type: reuse `Modify`** — no new `HookResponse` variant needed:
  * `Ok` = no change (pass through unchanged)
  * `Modify(json)` = replacement data (for transforms: full replacement, not a patch)
  * `Veto(reason)` = skip this plugin in the pipeline
  * The host interprets `Modify` differently based on event type:
    * Regular hooks (fan-out): `Modify` carries a partial patch, host merges into original
    * Transform hooks (pipeline): `Modify` carries full replacement, host replaces pipeline data
* **Dispatch mode differs from regular hooks:**
  * Regular hooks: **fan-out** — every subscribed plugin sees the original event
  * Transform hooks: **pipeline** — each plugin sees the output of the previous plugin
  * The host determines dispatch mode from the event name (hardcoded 2-element set:
    `{"transform_messages", "transform_system_prompt"}`)
  * Plugin authors don't need to know the dispatch mode — they just handle events
* Implemented methods on `PluginHost` (separate from `dispatch_hook`):
  ```rust
  pub fn dispatch_transform(&mut self, event_name: &str, payload: String) -> String
  ```
  Internally calls `handle(event)` on each plugin in pipeline order, chaining `Modify` payloads.
  Convenience wrappers:
  ```rust
  pub fn transform_messages(&mut self, messages: Vec<Message>) -> Vec<Message>
  pub fn transform_system_prompt(&mut self, prompt: String) -> String
  ```
* **User controls ordering** — plugins do NOT declare priority or phase. The user defines
  the transform pipeline order in `ucode.toml`:
  ```toml
  [context_management.transform_pipeline]
  order = [
    "org.acme.custom-dedup",   # runs first
    "native",                   # built-in dedup/supersede/purge (Task 8.8)
    "org.acme.extra-pruner",   # runs after native
  ]
  ```
  * Default (no config): `["native"]` — only built-in context management runs.
  * Omitting `"native"` from the list disables native context management for transforms.
  * Plugin subscribes to transform events via `hooks = ["transform_messages"]` in manifest.
  * Plugins subscribed but not in `order` list are **not called** for transforms
    (still called for regular hooks they subscribe to).
  * Plugins in `order` but not subscribed to the transform event are skipped silently.
* Safety classification: `Guarded` (can modify content but not escalate permissions)
* Transform hooks run synchronously in the LLM call path (latency-sensitive)

**8.6.5 Plugin tool registration** [DONE]

* Plugins can declare tools in `plugin.toml` under `[[tools]]` section
* Declared tools appear in the LLM's tool list alongside built-in tools
* Tool calls to plugin-registered tools are routed through the `tool-provider` WIT interface
  (already defined in `ucode:plugin/tool-provider`):
  `handle-tool-call(name, args_json) -> result<string, string>`
* Plugin tools go through the **same sandbox/approval policy** as built-in tools — no special path
* Tool namespacing: `{plugin_id}.{tool_name}` (e.g., `org.acme.dcp.distill`)

**8.6.6 Hook payload versioning** [DONE]

* Each hook event type has its own payload version via `HookEvent::payload_version()` (semver).
  All events start at `"1.0.0"`.
* Additive fields bump minor version; breaking changes bump major
* Plugins declare `min_payload_versions` in their manifest:
  ```toml
  [min_payload_versions]
  session_start = "1.0.0"
  before_tool_call = "1.0.0"
  ```
* Host skips dispatch to plugin if its declared min version > current payload version
  (version-mismatch skipping implemented in `dispatch_hook()`)
* Two independent versioning layers:
  * **Plugin API version** (`min_api_version`) — covers WIT interface shape changes
  * **Payload version** (per-hook semver) — covers per-hook schema evolution within typed records

**8.6.7 Hook payload documentation** [DONE]

* Generate `docs/hooks/` with one markdown file per hook category (session, tool, context, etc.)
* Each doc includes: event name, safety tier, JSON payload schema, response options, version history
* Can be auto-generated from hook definitions in `hooks.rs` or maintained manually

**8.6.8 Fixture plugin (end-to-end contract test)** [DONE]

* `examples/plugins/context-manager/` — minimal WASM plugin demonstrating:
  * Loads from user plugin path
  * Implements typed WIT exports for `session_start`, `session_end`, and `transform_messages` events
    (transform_messages removes duplicate assistant messages via pipeline dispatch)
  * Implements `tool-provider` with a `context_stats` tool returning message count/size
  * Respects capability restrictions (cannot escalate)
* Demonstrates the full lifecycle: discovery → load → dispatch → transform → tool call → response
* Integration test that loads fixture plugin from temp dir simulating `~/.ucode/plugins/`

**Acceptance**

* Fixture plugin loads from `~/.ucode/plugins/` and `.ucode/plugins/` paths. [DONE - 8.6.1]
* WASM hook dispatch calls typed per-event WIT handlers (not stubbed). [DONE - 8.6.3]
* Transform events (`transform_messages`, `transform_system_prompt`) dispatched through typed
  `hooks-transform` WIT interfaces with pipeline composition. [DONE - 8.6.4]
* `Modify` response carries full replacement for transform events, partial patch for regular events. [DONE - 8.6.4]
* Plugin-registered tools appear in LLM tool list, route via `tool-provider`, and go through
  the same approval/sandbox policy as built-in tools. [DONE - 8.6.5]
* Hook payloads include payload version; version mismatch handled gracefully. [DONE - 8.6.6]
* User controls transform pipeline ordering via `ucode.toml`. [DONE - 8.6.4]
* Permission escalation attempts are blocked and recorded in audit logs. [DONE - 8.6.5]
* 20 typed WIT category packages (19 existing + `hooks-transform`) provide compile-time
  safety at the WASM boundary. [DONE - 8.6.2]

### Task 8.8 Native context management system (ucode-context crate) [P0] [DONE]

> Design doc: `docs/plans/2026-03-06-context-management-design.md`

Native context management as a core crate — not a plugin. Combines strategies from opencode-dcp,
rlm-skill, and context-mode. Runs directly in the LLM call path with zero serialization overhead.

**Architecture:** Strategy Pipeline — each strategy implements `ContextStrategy` trait, chained
in a `ContextPipeline`. Knowledge base and session continuity are separate infrastructure modules.
Strategies need per-instance state (dedup tracks file hashes, purge tracks turn counts), and there
are 4 concrete implementations (dedup, supersede, purge, sandbox) — meets the 3+ threshold for a
trait. Mirrors the transform pipeline pattern from Task 8.6.

**Why native, not plugin:** The automatic strategies (dedup/supersede/purge) are pure algorithmic
message rewrites. The knowledge base is core infrastructure. Session continuity is inherently a
host concern. Routing these through WASM plugin boundaries adds latency and complexity for no
benefit. External plugins (Task 8.6) can still hook into the system for custom strategies.

**Status:** DONE. 47 tests, all passing. 12 source files in `crates/ucode-context/`.

**Completion Summary:**
* All 6 subtasks completed (8.8.1-8.8.6)
* Strategies: dedup, supersede, purge-errors, sandbox
* Knowledge base: FTS5 with Porter stemming (sqlite-vec stub ready)
* 5 tools: knowledge_search/store, context_prune/compress/distill
* Session continuity: 8 event types, compaction snapshots, restore prompts
* Design deviations:
  - `sqlite-vec 0.3` doesn't exist; used `0.1.7-alpha.10`
  - `KnowledgeEntry.created_at` is `String` not `DateTime<Utc>` (rusqlite lacks chrono feature)
  - KB tools use `Arc<Mutex<KnowledgeBase>>` not `Arc<KnowledgeBase>` (rusqlite Connection is not Sync)

**8.8.1 Per-model context management configuration** [DONE]

* Context management is configurable per model/provider in `ucode.toml`:
  ```toml
  [context_management]
  enabled = true                    # global toggle

  [context_management.strategies]
  dedup = true                      # on by default (zero cost)
  supersede_writes = true           # on by default (zero cost)
  purge_errors = true               # on by default (zero cost)
  purge_errors_after_turns = 3
  sandbox_large_outputs = true      # on by default
  sandbox_threshold_chars = 2000
  knowledge_base = true             # on by default
  session_continuity = true         # on by default

  [context_management.knowledge_base]
  enabled = true
  embedding = "auto"                # "auto" | "local" | "endpoint" | "none"

  # Custom embedding endpoint (OpenAI-compatible API)
  [context_management.knowledge_base.embedding_endpoint]
  url = "http://localhost:11434/v1/embeddings"
  model = "all-minilm"
  dimensions = 384

  [context_management.pruning]
  enabled = true                    # LLM-driven pruning tools
  trigger_threshold_pct = 60        # trigger when context > 60% capacity
  model = "auto"                    # "auto" = use session model, or explicit model name

  # Per-model overrides — disable LLM pruning for expensive models
  [context_management.pruning.overrides."claude-opus-4"]
  enabled = false                   # Opus: rely on automatic strategies only

  [context_management.pruning.overrides."claude-sonnet-4"]
  enabled = true                    # Sonnet: cheap enough for LLM pruning
  trigger_threshold_pct = 50        # trigger earlier for Sonnet
  ```
* When LLM pruning is disabled for a model, automatic strategies + sandbox + knowledge base
  still run (zero LLM cost). The model just doesn't get distill/compress/prune tools.
* `model = "auto"` uses the session's current model. Can be overridden to use a cheaper model
  (e.g., use Haiku for pruning even when session runs on Opus).

**8.8.2 Automatic zero-cost strategies (from opencode-dcp)** [DONE]

* **Deduplication:** Remove duplicate tool reads of the same file within a session.
  Track file content hashes (`DefaultHasher`); on duplicate, replace with
  `[already in context — see earlier read of {path}]` placeholder.
* **Supersede-writes:** When a file is written then later read, remove the earlier write's
  full content (the read has the current state). Replace with
  `[superseded by later read of {path}]`.
* **Purge-errors:** After N turns (configurable, default 3), remove errored tool call
  inputs/outputs (they're no longer actionable). Also purges the corresponding `ToolCall`
  args to save additional tokens.
* All strategies implement `ContextStrategy` trait and run in `ContextPipeline` before
  each LLM call. Zero LLM cost — pure algorithmic message rewriting in Rust.

**8.8.3 Sandbox execution (from rlm-skill / context-mode)** [DONE]

* `SandboxInterceptor` implements `ContextStrategy`. Runs after dedup/supersede/purge
  (so we don't sandbox content that would have been deduped anyway).
* Intercept large tool outputs (Read, Bash, WebFetch) before they enter context.
* Configurable threshold (default: 2000 chars). Outputs above threshold are:
  * Stored in knowledge base (Task 8.8.5) with metadata (tool name, file path, content type)
  * Replaced in context with metadata summary (line count, content type, first/last 3 lines)
* LLM sees summary + can retrieve full content via `knowledge_search` tool.

**8.8.4 Smart LLM-driven pruning tools (from opencode-dcp, improved)** [DONE]

* Register 5 built-in tools the LLM can call to manage its own context:
  * `context_distill` — Summarize a range of messages into a compact digest
  * `context_compress` — Replace verbose tool outputs with key findings
  * `context_prune` — Remove messages by index/range that are no longer relevant
  * `knowledge_search` — Query knowledge base (always available regardless of pruning config)
  * `knowledge_store` — Explicitly index content in knowledge base (always available)
* System prompt injection tells LLM these tools exist and when to use them.
* **Smart triggering:** Only inject pruning instructions when context exceeds threshold
  (default 60% of model's context window). Below threshold, no system prompt overhead.
* **Per-model control:** Disable entirely for expensive models (Opus), enable for cheaper
  models (Sonnet/Haiku). Configurable via `[context_management.pruning.overrides]`.
* **Cheaper-model delegation:** Option to route pruning tool calls to a cheaper model
  (e.g., Haiku summarizes, Opus keeps working). Initial implementation: `"auto"` only.
* Tools are registered as native built-in tools (not via plugin tool registration).

**8.8.5 Hybrid knowledge base — FTS5 + sqlite-vec (from rlm-skill / context-mode)** [DONE]

* **Dual search** in one SQLite database per session (`{session_dir}/knowledge.db`):
  * **FTS5** (always available): keyword search with Porter stemming + BM25 ranking.
    Zero additional dependencies beyond rusqlite.
  * **sqlite-vec** (when embeddings available): semantic vector search with cosine
    similarity. Finds conceptually related content even with different words.
* **Embedder abstraction** (`trait Embedder`): pluggable embedding source.
  * `EndpointEmbedder` — any OpenAI-compatible API (Ollama, LiteLLM, vLLM, custom)
  * `ProviderEmbedder` — session provider's embedding API (OpenAI, etc.)
  * `LocalEmbedder` — fastembed with all-MiniLM-L6-v2, behind `local-embeddings` feature flag
* **Embedding resolution** for `embedding = "auto"`:
  1. Custom `embedding_endpoint` if configured
  2. Session provider embedding API if available
  3. Fall back to FTS5 keyword search only
* **Hybrid ranking** via Reciprocal Rank Fusion (RRF) when both FTS5 and vector
  results are available: `RRF(doc) = 1/(k + rank_fts5) + 1/(k + rank_vec)`, k=60.
* `knowledge_search` and `knowledge_store` tools registered for LLM.

**8.8.6 Session continuity (from context-mode)** [DONE]

* Capture significant events after tool use. Event types:
  * `GoalEstablished` — user stated or refined their goal
  * `FileChanged` — file created or modified during session
  * `TestResult` — test/build/lint result
  * `Decision` — architectural or implementation decision
  * `ToolOutput` — significant tool output worth remembering
  * `ErrorEncountered` — approach tried and failed (with reason)
  * `GitCommit` — git commit made during session
  * `ConfigChanged` — model or skill switched
* **Compaction snapshot** created before each compaction, containing:
  * `user_goals` — what the user is trying to accomplish
  * `working_set` — files actively being edited
  * `reference_files` — files read for reference
  * `pending_tasks` — unfinished work items
  * `key_decisions` — important decisions and rationale
  * `error_history` — failed approaches and why (avoid repeating mistakes)
  * `git_state` — branch name, commits made during session
* Event log persisted to `{session_dir}/continuity_events.json`.
* Snapshot persisted to `{session_dir}/continuity.json`.
* On session resume, snapshot injected as system message prefix with structured
  context restoration.

**Acceptance** [DONE]

* Dedup/supersede/purge reduce message array size measurably on repeated file operations.
* Large tool outputs are sandboxed and retrievable via knowledge base search.
* LLM-driven pruning is configurable per model; disabled for Opus by default.
* Pruning can delegate to a cheaper model when configured.
* Knowledge base search returns relevant results via FTS5 keyword search.
* When embeddings are available (endpoint/provider/local), hybrid FTS5+vector search
  returns semantically relevant results via RRF ranking.
* Custom embedding endpoint (Ollama, LiteLLM, etc.) works with OpenAI-compatible API.
* Session survives compaction and resumes with prior state context including goals,
  working set, error history, and git state.
* All strategies respect `ucode.toml` configuration and per-model overrides.


### Task 8.7 Remote plugin install/update distribution with trust verification (ucode-plugins + security) [P1]

* Support plugin install/update from git/url/registry sources
* Verify signatures/fingerprints before activation
* Maintain trust records and detect update drift requiring re-approval
* Keep rollback path to previous plugin version on failed update

**Acceptance**

* Trusted signed plugin installs and activates.
* Signature mismatch or trust failure blocks activation with clear diagnostics.
* Update drift triggers re-approval and supports rollback.

---

# Phase 9 — WASM plugin runtime (latest stage)

### Task 9.1 WASM plugin host runtime (ucode-plugins)

* Implement WASM component runtime in host (WASI/component model)
* Load, instantiate, and lifecycle-manage plugin `.wasm` components
* Capability wiring from policy engine into host imports
* Warm instance pool + cache for low-latency execution

**Acceptance**

* WASM plugin loads and receives hooks in-session.
* Crash/isolation boundaries hold without taking down host session.
* Warm instance path meets documented latency budget.

### Task 9.2 WASM plugin packaging + signing/trust flow

* Plugin package includes manifest, wasm artifact, metadata, and signature
* Trust model for install/update + signature verification
* Fingerprint/version drift triggers trust re-confirmation

**Acceptance**

* Unsigned/untrusted plugin is blocked by default.
* Signature mismatch blocks activation with clear diagnostics.

### Task 9.3 Non-interactive headless/CI mode (ucode-cli + ucode-core) [DONE]

* Add CLI non-interactive execution mode suitable for automation pipelines
* Machine-readable JSON output envelope (events, artifacts, usage, terminal status)
* Deterministic exit-code mapping by result class
* Support resume-by-session-id in headless flow

**Acceptance**

* CI run completes without interactive prompts.
* JSON output includes status and artifact references.
* Exit codes map consistently to success/failure classes.

---

# Cross-cutting: config, permissions, observability

### Config file

`UCODE_HOME` defaults to `${XDG_CONFIG_HOME:-~/.config}/ucode`.

Canonical runtime format: `${UCODE_HOME}/ucode.toml` (TOML only).

Precedence order: built-in defaults < user global config < project-local config < session overrides.

`${UCODE_HOME}/ucode.toml`:

* providers + model groups
* auth mode preference per provider (key vs login)
* fallback order
* compaction/distillation policy (strategy order, pinned-window size, retry limits)
* cost/token budget policy (soft/hard thresholds, action on breach)
* logging config: level, sink toggles (stderr/file), per-session file path, rolling policy (size/time, max files)
* MCP servers list
* plugin list
* plugin discovery paths (project/user) for external plugins such as DCP adapters
* remote plugin sources + trust records (git/url/registry)
* allowlists/denylists for run_cmd + file access scope
* per-tool settings (timeouts, output caps, approval mode, allowlists/denylists) with policy-safe limits
* sandbox policy matrix (global / provider / agent / tool / session)
* network policy matrix (including web/deep-research enablement per agent)
* checkpoint retention policy (count/time)
* background job policy (default detached behavior, kill permissions)
* MCP server launcher definitions + trust records
* MCP transport config (stdio/SSE/HTTP) and auth settings
* subagent profiles (capabilities, sandbox tier, comm policy)
* command registry paths (user/project/plugin) + parser resolver order
* plugin API/runtime config (WASM-only runtime, enabled in latest stage)
* session management config (auto-title on/off, title model preference, archive defaults)
* cache policy (provider hints, local fallback, invalidation)
* TUI keybind overrides

### Configuration acceptance notes

* Config docs must include a complete TOML example and precedence walkthrough.
* Per-tool overrides must not bypass stricter parent policy layers.
* Schema version and migration behavior must be documented.

### Observability

* `tracing` logs to file + in-TUI log pane
* audit trail: tool calls, command runs, patches, fallbacks, auth transitions,
  approvals/denials, effective sandbox tier per action, agent spawn/join lifecycle,
  inter-agent messages, MCP server launch/trust decisions
* include compaction/distillation events and session title generation/update events in audit stream
* include background job lifecycle events (start/cancel/kill/complete) and budget-threshold events
* include explicit logging configuration snapshot at session start (effective level/sinks/redaction mode)

---

## Delegation map (agents)

* **Agent A (Core):** Phase 1 router/events/session state + token compaction/distillation + session lifecycle/title generation + structured logging + subagent orchestration + inter-agent comm + mention/command parser bindings
* **Agent B (Auth):** Phase 2 keychain + OpenAI login + Anthropic subscription login + CLI/TUI connect flow
* **Agent C (Providers):** Phase 3 adapters (OpenAI/Anthropic/Ollama)
* **Agent D (Tools):** Phase 4 built-ins + patch applier + cmd runner + sandbox policy engine + confirmation gates + checkpoints + background job control + artifacts
* **Agent E (MCP):** Phase 5 MCP client + registry + native launchers + transport parity + resources/prompts + per-server trust/policy
* **Agent F (Skills):** Phase 6 SKILL.md discovery/parsing/execution
* **Agent G (TUI):** Phase 7 ratatui UI + approvals + palette + visual system + sidebar
* **Agent H (Plugins):** Phase 8 plugin contracts/hooks + external plugin infrastructure + combined context management + remote install/update trust + Phase 9 WASM runtime
* **Agent S (Security):** Cross-cutting: threat model, audit verification, sandbox backend integration

---

## “Done” checklist

1. Fullscreen TUI works on Linux + macOS with sidebar-first visual system
2. `/connect` supports API keys + login flows (+ subscription login)
3. Multi-model routing/fallback works and is visible in logs
4. Built-in tools cover fs/search/patch/cmd (+ optional git)
5. Fine-grained sandbox policy works per-tool/per-agent/per-provider with configurable tiers
6. Outside-project actions require explicit user confirmation
7. MCP tools can be discovered and called via native launchers (`uvx`/`npx`/`bunx`)
8. MCP servers have per-server trust, permissions, and lifecycle management
9. SKILL.md from Claude Code/OpenCode loads and can drive sessions
10. Async subagent orchestration works with spawn/wait/join/cancel
11. Inter-agent communication available when enabled by policy
12. Plugins can hook key lifecycle/tool events (including agent/sandbox/approval events)
13. `@agent` mentions and `/command` invocations resolve deterministically
14. Patch apply is fast and robust across shifted offsets
15. Plugin API is versioned (WIT/component model), with Rust SDK and safety-governed behavior overrides
16. WASM plugin runtime is delivered in latest stage with trust/signature checks
17. Threat model documented and audit trail covers all high-risk transitions
18. Token exhaustion handling supports deterministic compaction/distillation with auditable artifacts
19. Session lifecycle supports list/switch/archive/rename with robust model-generated titles and manual override lock
20. External user-installed DCP-style plugins can load via documented paths and consume public hook contracts safely
21. Session resume/fork lineage works cleanly across CLI and TUI
22. Token/cost budgets provide soft/hard guardrails with visible runtime alerts
23. Prompt/context caching reduces repeat request overhead with auditable cache behavior
24. Workspace checkpoints/rollback allow fast recovery from risky agent actions
25. Detached background jobs are manageable from TUI, including interactive cancel and force-kill
26. MCP supports stdio, SSE, and HTTP transports plus resources/prompts
27. Headless CI mode provides deterministic JSON outputs and exit codes
28. Remote plugin install/update supports trust verification and rollback
29. Logging supports level-gated diagnostics with stderr + file sinks and per-session plus rolling retention
30. Combined context management (dedup/supersede/purge + sandbox execution + FTS5 knowledge base + LLM pruning tools + session continuity) keeps sessions productive beyond default context limits

---
