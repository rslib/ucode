---

## Project goals

* Fullscreen TUI (Linux + macOS)
* Multi-model routing + fallback (fast/strong/longctx)
* Async subagent orchestration with inter-agent communication
* Mention-driven orchestration (`@agent`) + user-defined slash commands (`/command`)
* Built-in tools (fs/search/patch/cmd/git)
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

### Task 1.4: Inter-agent communication channels (ucode-core)

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

# Phase 2 — Auth (API key + login + subscription login)

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

## 2.2 OpenAI login (ucode-auth)

Support:

* Browser OAuth login
* Device-code login (`--device`) for headless

Store:

* access + refresh token + expiry
  Auto-refresh.

CLI:

* `ucode auth login openai [--device]`

**Acceptance**

* Works without API key; survives token expiry (refresh).

## 2.3 Anthropic login (subscription plan) (ucode-auth)

Support OpenCode-like flow:

* Browser opens Claude account sign-in
* User receives a code/token and pastes into CLI/TUI
* Store as session token (and refresh/relogin as needed)

CLI:

* `ucode auth login anthropic --subscription`

**Acceptance**

* If the provider issues a usable token, requests succeed; if not, errors are shown clearly and routing can fallback automatically.

## 2.4 Anthropic API key (ucode-auth)

* `ucode auth set-key anthropic` + env support

**Acceptance**

* Anthropic adapter works with API key.

## 2.5 Auth-aware fallback (ucode-core + providers)

* Auth errors mapped to `AuthMissing/AuthInvalid/AuthExpired`
* Router immediately falls back and emits `Event::Log` explaining why

**Acceptance**

* Break token mid-session and the conversation continues via fallback model.

---

# Phase 3 — Provider adapters (streaming + tool translation)

### Task 3.1 Provider trait (ucode-providers)

* `Provider::stream_chat(req) -> Stream<Event>`
* `capabilities()` (tools, json mode, max context, max output, token counting support)
* optional adapter `count_tokens(req)` for provider-native counting

### Task 3.2 OpenAI adapter

* Streaming tokens
* Tool/function calls → canonical ToolCall
* Uses auth module (key or login)

### Task 3.3 Anthropic adapter

* Streaming tokens
* Tool use mapping → canonical ToolCall
* Uses auth module (API key or subscription login)

### Task 3.4 Local adapter (Ollama) (optional but recommended)

* Fast fallback / offline mode

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

### Task 4.1 Tool registry (ucode-tools)

* `ToolSpec { name, schema, description }`
* `ToolHandler: async fn(args)->ToolResult`
* Permissions gate integrated (per session + per skill)

### Task 4.2 Filesystem/search tools

* `read_file`, `list_dir`
* `ripgrep_search` (or ignore+regex implementation)
* Respect `.gitignore` via `ignore`

### Task 4.3 Patch tool (core feature)

* `apply_patch(unified_diff)` robust applier
* Context-anchor matching + bounded offset scanning for shifted hunks
* Fast path for exact matches; fallback matcher for nearby offsets
* Support LF/CRLF normalization and deterministic reject reasons
* Return rejects + reasons on failure

### Task 4.4 Command runner tool

* `run_cmd(cmd, cwd, timeout, env)`
* output cap + timeout
* require user approval (TUI prompt) unless allowlisted

### Task 4.5 Git helpers (optional)

* `git_status`, `git_diff` via `gix` or shell `git`

### Task 4.6 Fine-grained sandbox policy engine (ucode-tools)

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

### Task 4.7 Outside-project confirmation gates (ucode-tools + ucode-tui)

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

### Task 4.8 Network capability policy for web/deep research (ucode-tools)

* Separate `network` capability from general command execution
* Policy can allow network for selected tools/agents only (e.g., web-search agent gets `networked` tier)
* Designed so local-only model agents and web-research agents coexist in same session
* Domain/port allowlists for fine-grained egress control

**Acceptance**

* Agent A (local-only) has no network; Agent B (research) has constrained network.
* Policy changes are visible in runtime logs and UI.

### Task 4.9 Workspace checkpoints + rollback controls (ucode-tools + ucode-core + ucode-tui) [P1]

* Create lightweight checkpoints before risky mutating operations (patch apply, mutating commands)
* Provide rollback API and TUI action to restore prior checkpoint
* Checkpoint retention policy (count/time based) with UI visibility

**Acceptance**

* Patch apply creates checkpoint and rollback restores prior state.
* Command-induced mutations can be rolled back.
* Retention pruning follows configured policy.

### Task 4.10 Background jobs with interactive cancel/kill (ucode-tools + ucode-core + ucode-tui) [P0]

* Add background job runtime: start/list/status/cancel/kill
* Persist job states: queued/running/completed/failed/cancelled/killed
* Add TUI background jobs panel with keyboard shortcuts for cancel and force-kill
* Keep chat responsive while jobs run detached

**Acceptance**

* Long-running job can run detached without blocking chat.
* User can cancel and force-kill running jobs interactively from TUI.
* Job lifecycle transitions are auditable and persisted.

### Task 4.11 Structured artifact output/export (ucode-tools + ucode-cli) [P1]

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

### Task 5.1 MCP client library (ucode-mcp)

* stdio transport first
* tool discovery + tool execution
* schema conversion → ToolSpec

### Task 5.2 MCP registry integration (ucode-tools)

* namespacing strategy
* collision handling
* tool call routing (built-in vs MCP)

### Task 5.3 Native MCP launcher manager (ucode-mcp)

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

### Task 5.4 MCP per-server policy + lifecycle controls (ucode-mcp + ucode-tools)

* Per-server sandbox tier, network policy, and tool permission profile
* Managed lifecycle: start/stop/restart with crash diagnostics
* Health check + auto-restart with backoff
* Full audit events: launch, approval, deny, crash, restart

**Acceptance**

* Untrusted server cannot execute tools until approved.
* Per-server policy is enforced at invocation time.
* Crash/restart cycle produces clear diagnostics.

### Task 5.5 MCP transport parity (stdio + SSE + HTTP) (ucode-mcp) [P0]

* Support stdio, SSE, and HTTP MCP transports
* Add auth header/token config for remote transports
* Reconnect/backoff strategy for transient transport failures
* Surface transport health in logs/UI

**Acceptance**

* Discovery/invocation works across stdio, SSE, and HTTP servers.
* Disconnect triggers bounded reconnect with diagnostics.
* Remote auth failures return actionable errors.

### Task 5.6 MCP resources/prompts integration (ucode-mcp + ucode-tools + ucode-core) [P0]

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

### Task 6.1 Skill discovery + parsing (ucode-skills)

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

### Task 6.2 Skill selection/execution

* active skill becomes prompt prefix + tool constraints + routing hints
* switch skill from TUI

**Acceptance**

* Drop in existing skills; they appear and work.

---

# Phase 7 — Fullscreen TUI (ratatui)

### Task 7.1 TUI foundation (ucode-tui)

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

### Task 7.2 Approvals UX

* Diff modal: approve/apply, reject
* run_cmd modal: approve once / approve session / deny

### Task 7.3 Visual system + sidebar-first information design (ucode-tui)

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

### Task 7.4 Slash command UX and discoverability (ucode-tui + ucode-core)

* Command palette and input parser support `/command` invocation
* Show command source badges: user/project/plugin
* Inline argument hints and validation errors
* Integrate command execution with same policy/sandbox/approval pipeline as normal actions

**Acceptance**

* User-defined `/command` resolves and executes from TUI input.
* Unknown command returns suggestions.
* Command execution obeys the same sandbox/approval controls.

---

# Phase 8 — Plugins & hooks (user customization)

Define plugin contracts, event surfaces, and safety policy first. Defer WASM runtime implementation to the latest stage.

### Task 8.1 Plugin manifest + loader (ucode-plugins)

* `plugin.toml`: name, version, command, hooks, tools exported

### Task 8.2 Hooks API (v1)

Events:

* `on_session_start/end`
* `before/after_tool_call`
* `before/after_apply_patch`
* `before/after_run_cmd`
* `before_model_select`
* `on_model_fallback`
* `on_router_decision`
* `on_context_shrink`
* `on_context_distilled`
* `on_skill_changed`
* `on_auth_changed`
* `on_session_title_generated/updated`
* `on_agent_spawned/completed/failed`
* `on_agent_message`
* `on_command_invoked`
* `on_sandbox_decision`
* `on_permission_decision`
* `on_approval_granted/denied`
* `on_mcp_server_launch/restart/crash`
* `on_budget_threshold_warning/reached`
* `on_background_job_state_changed`

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

### Task 8.3 Plugin API contract + SDKs (ucode-plugins)

* Define versioned plugin API using WIT/component-model interfaces (`v1` contract):

  * handshake/capability negotiation
  * hook subscriptions
  * optional tool export schema
  * structured error/result envelope with policy decision metadata

* Tool registration model:

  * plugin declares `ToolSpec` set during initialization
  * host namespaces tools as `plugin.<plugin-id>.<tool-name>`
  * host validates schema and applies capability policy before activation

* Provide Rust-first SDK for authoring WASM plugins
* Plugin manifest includes requested capabilities and minimum API version

**Acceptance**

* Rust WASM example plugin passes handshake and receives hooks.
* Version mismatch produces clear incompatibility error.

### Task 8.4 Plugin runtime isolation model (ucode-plugins + security)

* Runtime policy model prepared for WASM-only plugin execution
* Per-plugin policy profile: filesystem scope, network, command spawn, hook scopes
* All plugin-originated actions routed through the same approval and audit pipeline

**Acceptance**

* Untrusted plugin cannot execute outside its granted policy.
* Plugin-originated tool call triggers normal approval/sandbox checks.
* Runtime model and effective permissions visible in logs/UI.

### Task 8.5 External DCP-style plugin support + hook exposure (ucode-plugins + ucode-core) [P0]

* Support user-installed DCP-style plugins (for example `opencode-dcp`) via documented public hook contracts
* Guarantee stable, versioned payload schemas for hooks used by DCP workflows:

  * `on_context_shrink`, `on_context_distilled`
  * `on_session_title_generated/updated`
  * `on_session_start/end`, `before/after_tool_call`

* Ensure user and project plugin discovery paths are documented and tested
* Keep capability model strict: external DCP plugins cannot bypass effective sandbox/network/filesystem policy

**Acceptance**

* Fixture external plugin `opencode-dcp` loads from user plugin path and receives documented hooks.
* Plugin can react to compaction/distillation and session-title events without private host APIs.
* Permission escalation attempts are blocked and recorded in audit logs.

### Task 8.6 Remote plugin install/update distribution with trust verification (ucode-plugins + security) [P1]

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

### Task 9.3 Non-interactive headless/CI mode (ucode-cli + ucode-core) [P0]

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
* **Agent H (Plugins):** Phase 8 plugin contracts/hooks + external DCP compatibility + remote install/update trust + Phase 9 WASM runtime
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

---
