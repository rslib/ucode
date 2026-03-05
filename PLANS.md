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

### Task 1.3: Async subagent orchestration (ucode-core)

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

### Task 1.5: Mention/command parser + orchestrator bindings (ucode-core)

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

---

# Phase 2 — Auth (API key + login + subscription login)

## 2.1 Credential storage (ucode-auth)

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
* `capabilities()` (tools, json mode, max context)

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
* `on_skill_changed`
* `on_auth_changed`
* `on_agent_spawned/completed/failed`
* `on_agent_message`
* `on_command_invoked`
* `on_sandbox_decision`
* `on_permission_decision`
* `on_approval_granted/denied`
* `on_mcp_server_launch/restart/crash`

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

---

# Cross-cutting: config, permissions, observability

### Config file

`~/.config/ucode/config.toml`:

* providers + model groups
* auth mode preference per provider (key vs login)
* fallback order
* MCP servers list
* plugin list
* allowlists/denylists for run_cmd + file access scope
* sandbox policy matrix (global / provider / agent / tool / session)
* network policy matrix (including web/deep-research enablement per agent)
* MCP server launcher definitions + trust records
* subagent profiles (capabilities, sandbox tier, comm policy)
* command registry paths (user/project/plugin) + parser resolver order
* plugin API/runtime config (WASM-only runtime, enabled in latest stage)
* TUI keybind overrides

### Observability

* `tracing` logs to file + in-TUI log pane
* audit trail: tool calls, command runs, patches, fallbacks, auth transitions,
  approvals/denials, effective sandbox tier per action, agent spawn/join lifecycle,
  inter-agent messages, MCP server launch/trust decisions

---

## Delegation map (agents)

* **Agent A (Core):** Phase 1 router/events/session state + subagent orchestration + inter-agent comm + mention/command parser bindings
* **Agent B (Auth):** Phase 2 keychain + OpenAI login + Anthropic subscription login + CLI/TUI connect flow
* **Agent C (Providers):** Phase 3 adapters (OpenAI/Anthropic/Ollama)
* **Agent D (Tools):** Phase 4 built-ins + patch applier + cmd runner + sandbox policy engine + confirmation gates
* **Agent E (MCP):** Phase 5 MCP client + registry + native launchers + per-server trust/policy
* **Agent F (Skills):** Phase 6 SKILL.md discovery/parsing/execution
* **Agent G (TUI):** Phase 7 ratatui UI + approvals + palette + visual system + sidebar
* **Agent H (Plugins):** Phase 8 plugin contracts/hooks + Phase 9 WASM runtime
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

---

If you want, I can convert this into a **ready-to-paste issue backlog** (one issue per task with acceptance tests and file/module pointers) so your agentic tool can execute it cleanly.
