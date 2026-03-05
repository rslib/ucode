## EPIC 0 — Project bootstrap

### ISSUE 0001 — Create Rust workspace + CI (Linux/macOS) [DONE]

**Goal:** Establish repo skeleton and CI for all crates.
**Scope/Notes:**

* Cargo workspace with crates:

  * `ucode-core`, `ucode-auth`, `ucode-providers`, `ucode-tools`, `ucode-mcp`, `ucode-skills`, `ucode-plugins`, `ucode-tui`, `ucode-cli`
* Add formatting + lint + tests in CI for Linux + macOS.
  **Acceptance tests:**
* `cargo fmt --all --check`
* `cargo clippy --workspace --all-targets -- -D warnings`
* `cargo test --workspace`
  **Owner:** Platform/Core

---

## EPIC 1 — Core runtime + routing/fallback

### ISSUE 0101 — Canonical message + tool-call model (ucode-core) [DONE]

**Goal:** Define internal canonical message representation and tool call types.
**Scope/Notes:**

* Types:

  * `Message { role, parts: Vec<Part> }`
  * `Part::{Text, ToolCall, ToolResult}`
  * `ToolCall { id, name, args: serde_json::Value }`
  * `ToolResult { id, name, result: Value, is_error: bool }`
* Add serde round-trip tests.
  **Acceptance tests:**
* Unit test: serialize → deserialize → equals for each type.
  **Owner:** Core

### ISSUE 0102 — Canonical streaming event model (ucode-core) [DONE]

**Goal:** Standardize streaming output from providers/tools into one event stream.
**Scope/Notes:**

* `Event::{Token, ToolCall, ToolResult, Patch, Log, Error, Done}`
* Provide helper `EventStream` alias (boxed stream).
  **Acceptance tests:**
* Unit test: simulate event stream and ensure consumer can iterate and reconstruct transcript.
  **Owner:** Core

### ISSUE 0103 — Router + fallback policy engine (ucode-core) [DONE]

**Goal:** Implement routing between providers/models and fallback logic.
**Scope/Notes:**

* Model groups: `fast`, `strong`, `longctx`
* Fallback triggers:

  * rate limit/timeout → next provider/model
  * auth errors → next provider/model
  * context-too-large → shrink context pack + retry once
  * patch apply fails twice → escalate to `strong`
* Emit `Event::Log` for each decision.
  **Acceptance tests:**
* Unit tests with simulated errors confirm correct fallback order and escalation.
  **Owner:** Core

### ISSUE 0104 — Session state + transcript store (ucode-core) [DONE]

**Goal:** Persist session transcript and metadata (in-memory + disk).
**Scope/Notes:**

* MVP: JSON file store under app cache dir
* Store: active skill, active model, tool audit, last diff, last command
  **Acceptance tests:**
* Start session → write store → reload → transcript intact.
  **Owner:** Core/Platform

### ISSUE 0105 — Async subagent orchestration runtime (ucode-core) [DONE]

**Goal:** Support non-blocking subagent execution with spawn/wait/join semantics.
**Scope/Notes:**

* APIs: `spawn_agent`, `wait_agent`, `wait_all`, `wait_any`, `cancel_agent`, `list_agents`
* Parent/child task DAG with deterministic completion collection
* Lifecycle streamed via canonical events: `AgentSpawned`, `AgentMessage`, `AgentCompleted`, `AgentFailed`
* Each subagent gets its own event stream, tool registry view, and sandbox policy
  **Acceptance tests:**
* Spawn 3 agents concurrently and collect deterministic results with `wait_all`.
* Parent remains responsive (can process events) while children run.
* Cancellation and timeout produce explicit terminal states.
  **Owner:** Core

### ISSUE 0106 — Inter-agent communication bus (ucode-core)

**Goal:** Enable optional controlled communication among concurrent agents.
**Scope/Notes:**

* Mailbox messaging: `send(agent_id, payload)` with structured schema
* Optional shared context board for fan-out/fan-in coordination patterns
* Message size limits and audit logging for all messages
* Policy gate: inter-agent comm defaults to `off`, enabled per agent profile in config
* System functions without communication (spawn/wait orchestration still works)
  **Acceptance tests:**
* Two agents exchange messages and complete coordinated work.
* Communication disabled by policy does not break orchestration.
* All messages appear in audit trail.
  **Owner:** Core/Security

### ISSUE 0107 — Mention and slash-command directive parser (ucode-core) [DONE]

**Goal:** Support deterministic user directives for explicit orchestration and command execution.
**Scope/Notes:**

* Parse directives from user input:
  * `@agent-name` -> spawn matching registered subagent
  * `/command-name` -> resolve and execute command definition
* Resolver order: slash command -> registered agent mention -> file reference
* Escaping support: `\@name`, `\/command`
* Clear diagnostics for unknown/ambiguous references
  **Acceptance tests:**
* `@agent-name` spawns target agent and returns handle.
* `/command-name` resolves from user/project/plugin registry with deterministic precedence.
* Ambiguous token handling follows resolver rules with clear error output.
  **Owner:** Core

### ISSUE 0108 — Token budget manager + context compaction/distillation pipeline (ucode-core)

**Goal:** Keep conversations reliable when nearing model context limits by applying deterministic compaction/distillation before hard failure.
**Scope/Notes:**

* Per provider/model token budget estimator (input + reserved output budget)
* Preflight context-fit check before each provider request
* Progressive recovery pipeline (deterministic order):

  * trim low-value context artifacts (verbose logs/tool chatter)
  * compact older turns into concise summaries
  * distill long tool outputs into structured memory records
  * preserve recent turns and unresolved tool-call context as pinned
* Retry policy: compact/distill and retry until within budget or explicit terminal error
* Persist compaction/distillation artifacts in session store with provenance/audit linkage
* Emit routing/runtime logs for each compaction step and final token budget used
  **Acceptance tests:**
* Oversized transcript triggers compaction/distillation and request succeeds without user intervention.
* Pinned recent turns and unresolved tool context remain intact after compaction.
* Distilled artifacts reload with session and remain auditable.
  **Owner:** Core

### ISSUE 0109 — Session lifecycle + model-generated session titles (ucode-core + ucode-cli + ucode-tui)

**Goal:** Provide robust session management UX with automatic, high-signal titles generated by the model and user override controls.
**Scope/Notes:**

* Extend session metadata: `title`, `title_source(auto|manual)`, `created_at`, `last_active_at`, `archived`
* Auto-title generation from early conversation turns (and regenerate on demand)
* Deterministic fallback title when model title generation fails/unavailable
* Manual rename and title lock so user-defined titles are not overwritten
* Session lifecycle ops: create/list/switch/archive/unarchive/rename in CLI and TUI
* Persist title/history fields in transcript store and surface in session picker
  **Acceptance tests:**
* New session receives auto title; fallback title applied when generation fails.
* Manual rename persists across reload and is not replaced by later auto-title events.
* Archive/unarchive and switch flows work in both CLI and TUI.
  **Owner:** Core/CLI/TUI

### ISSUE 0110 — Session resume/fork lineage model (ucode-core + ucode-cli + ucode-tui) [P0]

**Goal:** Support deterministic session resume and branch/fork workflows for parallel exploration without losing transcript lineage.
**Scope/Notes:**

* Resume by id from CLI/TUI with full state restore (model, skill, policy, transcript)
* Fork session creates child session with parent pointer and lineage metadata
* Session list/switch UI exposes parent-child lineage and fork source
* Audit events for resume/fork/switch actions
  **Acceptance tests:**
* `resume(session_id)` restores full runnable state.
* `fork(session_id)` creates a child session with correct ancestry.
* TUI and CLI can switch between parent/child sessions without state bleed.
  **Owner:** Core/CLI/TUI

### ISSUE 0111 — Token/cost governance and budget controls (ucode-core + ucode-providers + ucode-tui) [P1]

**Goal:** Provide guardrails for token and cost usage per request/session to match agentic-tool operational expectations.
**Scope/Notes:**

* Track per-request and per-session token usage and estimated cost across providers
* Configurable soft/hard budgets with policy actions (warn, downgrade model group, block)
* Runtime budget alerts in TUI sidebar/status and log stream
* Export usage summary in session metadata for post-run analysis
  **Acceptance tests:**
* Soft budget threshold emits warning and does not interrupt workflow.
* Hard budget threshold enforces configured block/fallback behavior.
* Session usage summary persists and reloads.
  **Owner:** Core/Providers/TUI

---

## EPIC 2 — Auth (API keys + login + subscription login)

### ISSUE 0201 — Secure credential store (keychain) (ucode-auth)

**Goal:** Provide credential storage for Linux + macOS using OS keychain.
**Scope/Notes:**

* Use `keyring` crate
* Do not store secrets in plaintext config
* Provide `AuthMaterial::{ApiKey, OAuth, SessionToken}`
  **Acceptance tests:**
* `ucode auth set-key openai` stores key, `ucode auth status` shows configured, restarting keeps it.
* Verify config file contains no secret material.
  **Owner:** Auth

### ISSUE 0202 — Auth CLI commands (ucode-cli + ucode-auth)

**Goal:** Implement CLI UX for auth.
**Scope/Notes:**
Commands:

* `ucode auth status`
* `ucode auth set-key <provider>`
* `ucode auth logout <provider>`
* `ucode auth login openai [--device]`
* `ucode auth login anthropic --subscription`
  **Acceptance tests:**
* Each command runs and updates keychain state accordingly.
  **Owner:** Auth/CLI

### ISSUE 0203 — OpenAI login (browser OAuth) (ucode-auth)

**Goal:** Implement OpenAI browser login flow that yields tokens usable by provider adapter.
**Scope/Notes:**

* Store access + refresh + expiry
* Auto-refresh
* Provide friendly output for copy/paste device fallback if needed later
  **Acceptance tests:**
* `ucode auth login openai` completes and `ucode auth status` shows “logged in”
* Token refresh path exercised via forced expiry in tests/mocks
  **Owner:** Auth

### ISSUE 0204 — OpenAI login (device-code) (ucode-auth)

**Goal:** Implement device-code login for headless use.
**Scope/Notes:**

* `--device` prints user code + verification URL
* Poll token endpoint until success/timeout
  **Acceptance tests:**
* `ucode auth login openai --device` works end-to-end (mocked integration OK if real not possible)
  **Owner:** Auth

### ISSUE 0205 — Anthropic subscription login (OpenCode-like) (ucode-auth)

**Goal:** Implement “subscription login” flow: browser sign-in → user pastes code/token.
**Scope/Notes:**

* UX:

  * opens browser URL
  * prompts user to paste returned code/token
* Store as `SessionToken` with TTL if available; otherwise treat as short-lived and re-login when invalid.
  **Acceptance tests:**
* Flow stores a token and Anthropic adapter can attempt a request with it.
* On invalid token, system produces a clean auth error and can fallback.
  **Owner:** Auth

### ISSUE 0206 — Auth-aware fallback integration (ucode-core + providers)

**Goal:** Ensure auth failures trigger routing fallback with clear logs.
**Scope/Notes:**

* Map provider 401/403 to `AuthInvalid`
* Missing creds to `AuthMissing`
* Emit `Event::Log("fallback: auth invalid on X → switching to Y")`
  **Acceptance tests:**
* In a mocked provider test, returning 401 triggers fallback to next provider.
  **Owner:** Core/Auth

---

## EPIC 3 — Provider adapters (streaming + tool calls)

### ISSUE 0301 — Provider trait + capability model (ucode-providers)

**Goal:** Define provider interface that returns canonical `Event` stream.
**Scope/Notes:**

* `Provider::stream_chat(req) -> EventStream`
* `Provider::capabilities() -> Capabilities { tool_calls, json_mode, max_context, streaming }`
  **Acceptance tests:**
* Mock provider compiles and streams tokens + done.
  **Owner:** Providers

### ISSUE 0302 — OpenAI adapter (streaming + tools) (ucode-providers)

**Goal:** Implement OpenAI-compatible chat adapter with streaming and tool calls.
**Scope/Notes:**

* SSE streaming → `Event::Token`
* Tool calls translated → canonical `ToolCall`
* Uses `ucode-auth` credentials (API key or login)
  **Acceptance tests:**
* `ucode chat --provider openai` streams output (can be mocked)
* Tool-call event emitted for a tool-using prompt (mock ok)
  **Owner:** Providers

### ISSUE 0303 — Anthropic adapter (streaming + tools) (ucode-providers)

**Goal:** Implement Anthropic adapter with streaming and tool use.
**Scope/Notes:**

* Translate tool-use blocks to canonical `ToolCall`
* Auth: API key + subscription login token
  **Acceptance tests:**
* `ucode chat --provider anthropic` streams output (mock ok)
  **Owner:** Providers

### ISSUE 0304 — Ollama/local adapter (optional but recommended) (ucode-providers)

**Goal:** Provide local fast fallback model support.
**Acceptance tests:**

* `ucode chat --provider ollama` streams output (requires local ollama running or mock)
  **Owner:** Providers

### ISSUE 0305 — Prompt/context caching integration (ucode-providers + ucode-core) [P2]

**Goal:** Reduce repeated token cost/latency with explicit provider-aware prompt/context cache integration.
**Scope/Notes:**

* Cache policy for reusable prompt prefixes/system context blocks
* Provider-specific cache hints when available; local fallback cache strategy when not
* Cache invalidation based on model/provider/session-policy changes
* Cache hit/miss telemetry in runtime logs
  **Acceptance tests:**
* Repeated compatible requests show measurable cache-hit behavior.
* Invalidation triggers correctly on provider/model change.
* Cache behavior remains transparent in logs and audit events.
  **Owner:** Providers/Core

---

## EPIC 4 — Built-in tools + permissions

### ISSUE 0401 — Tool registry + invocation runtime (ucode-tools)

**Goal:** Create unified tool registry for built-in + MCP + plugins.
**Scope/Notes:**

* `ToolSpec { name, description, input_schema }`
* `ToolHandler` async
* `ToolRegistry::{register, list, invoke}`
  **Acceptance tests:**
* `list_tools` returns registered tools; invoke demo tool returns ToolResult.
  **Owner:** Tools

### ISSUE 0402 — Permissions system (ucode-tools)

**Goal:** Gate tool usage (file access + command execution).
**Scope/Notes:**

* Default safe policy:

  * file reads limited to repo root
  * command runner requires explicit approval (or allowlist)
* Policy sources: config + active skill + plugin veto
  **Acceptance tests:**
* Attempt to read outside repo denied.
* run_cmd blocked until approved.
  **Owner:** Tools/Security

### ISSUE 0403 — Built-in filesystem tools (ucode-tools)

**Goal:** Implement `read_file`, `list_dir` with gitignore-aware behavior.
**Acceptance tests:**

* Read a file inside repo succeeds; outside repo fails by policy.
  **Owner:** Tools

### ISSUE 0404 — Built-in search tool (ripgrep-like) (ucode-tools)

**Goal:** Implement `ripgrep_search(query, paths?, max_results?)`.
**Scope/Notes:**

* Use `ignore` crate traversal; optionally shell out to `rg` if installed (configurable).
  **Acceptance tests:**
* Search finds expected match; respects ignore.
  **Owner:** Tools

### ISSUE 0405 — Patch apply tool (unified diff) (ucode-tools)

**Goal:** Implement `apply_patch(diff)` robustly.
**Scope/Notes:**

* Return `applied`, `files_changed`, `rejects` with reasons
* Context-anchor matching with bounded offset scanning for shifted hunks
* Fast path for exact context matches; fallback matcher for nearby offsets
* LF/CRLF normalization and deterministic reject reasons
  **Acceptance tests:**
* Apply patch to sample file; verify content changed.
* Invalid patch returns rejects with reason.
* Shifted-hunk patch applies correctly using offset search.
* Large patch apply stays within documented performance budget.
  **Owner:** Tools

### ISSUE 0406 — Command runner tool (ucode-tools)

**Goal:** Implement `run_cmd` with timeouts/output caps and approval gating.
**Acceptance tests:**

* `run_cmd("echo hi")` returns output.
* `run_cmd("sleep 10", timeout=1)` times out cleanly.
  **Owner:** Tools

### ISSUE 0407 — Git helpers (optional) (ucode-tools)

**Goal:** Provide `git_status`, `git_diff` as tools (gix or shell).
**Acceptance tests:**

* In a git repo, status/diff returns data.
  **Owner:** Tools

### ISSUE 0408 — Fine-grained hierarchical sandbox policy engine (ucode-tools)

**Goal:** Provide configurable sandbox tiers with per-tool/per-agent/per-provider control.
**Scope/Notes:**

* Policy hierarchy (most restrictive wins): global → provider/model → agent → tool → session
* Sandbox tiers: `off`, `workspace`, `networked`, `strict`
* Per-tool capability flags: file read/write scope, command exec, network egress, spawn external process
* Linux backend: `bwrap` sandbox profiles
* macOS backend: native sandbox profile or documented degraded mode with explicit warning
* Canonical path guard: resolve symlinks, `..`, relative paths before all policy checks
  **Acceptance tests:**
* Effective policy reflects hierarchy and restriction precedence.
* Tool-level policy cannot escalate beyond parent constraints.
* Symlink escape attempt denied.
* If sandbox backend unavailable, user sees explicit warning and fallback behavior documented.
  **Owner:** Tools/Security

### ISSUE 0409 — Outside-project confirmation and boundary enforcement (ucode-tools + ucode-tui)

**Goal:** Require explicit consent for out-of-boundary actions with persisted trust decisions.
**Scope/Notes:**

* Mandatory approval for:
  * out-of-workspace file access
  * out-of-workspace command cwd/path
  * external process spawn (including MCP server first launch)
  * network access when policy requires consent
* Approval scopes: `once`, `session`
* Denials are first-class: enforced, logged with reason, visible in UI
* Canonical path checks (resolve symlinks, `..`, relative inputs) before policy evaluation
  **Acceptance tests:**
* Out-of-workspace read/exec is blocked until user approval.
* Denials are enforced and logged with clear reason.
* `../` traversal and symlink escape both denied.
  **Owner:** Tools/TUI/Security

### ISSUE 0410 — Network capability policy for web/deep research (ucode-tools)

**Goal:** Allow safe internet-enabled research without weakening default isolation.
**Scope/Notes:**

* Separate `network` capability from general command execution
* Policy can allow network for selected tools/agents only (e.g., research agent gets `networked` tier while coding agent stays `workspace`)
* Domain/port allowlists for fine-grained egress control
* Designed so local-model agents and web-research agents coexist in same session
  **Acceptance tests:**
* Agent A (local-only profile) has no network; Agent B (research profile) has constrained network.
* Policy changes are visible in runtime logs and TUI sidebar.
* Unauthorized network access attempt is blocked and logged.
  **Owner:** Tools/Security

### ISSUE 0411 — Workspace checkpoints + rollback safety controls (ucode-tools + ucode-core + ucode-tui) [P1]

**Goal:** Add lightweight checkpoints before risky actions so users can quickly revert agent-side modifications.
**Scope/Notes:**

* Create checkpoint snapshots before patch apply and command-run actions that modify workspace
* Rollback API and TUI actions to restore checkpoint state
* Checkpoint retention policy (count/time based) with explicit user visibility
  **Acceptance tests:**
* Applying a patch creates a checkpoint and rollback restores pre-patch state.
* Command-induced workspace changes can be reverted to prior checkpoint.
* Expired checkpoints are pruned according to retention policy.
  **Owner:** Tools/Core/TUI

### ISSUE 0412 — Background job controller + interactive cancel/kill (ucode-tools + ucode-core + ucode-tui) [P0]

**Goal:** Support detached long-running operations with explicit lifecycle control, including interactive kill from TUI.
**Scope/Notes:**

* Background job APIs: start/list/status/cancel/kill
* Job states: queued/running/completed/failed/cancelled/killed
* TUI background jobs panel with keyboard actions for cancel/kill
* Clear distinction between graceful cancel and force kill in logs/UI
  **Acceptance tests:**
* Long-running job can be detached and monitored while chat remains responsive.
* User can cancel and force-kill jobs interactively from TUI.
* Job lifecycle transitions are auditable and persisted to session metadata.
  **Owner:** Tools/Core/TUI

### ISSUE 0413 — Structured artifact output and export pipeline (ucode-tools + ucode-cli) [P1]

**Goal:** Produce and export structured artifacts (reports, diffs, run logs) for machine and human workflows.
**Scope/Notes:**

* Standard artifact envelope: id, type, source, metadata, checksum, created_at
* Export targets: workspace files and machine-readable CLI output references
* Artifact types include at minimum: markdown report, unified diff, command/test run logs
* Audit linkage from session/tool events to artifact ids
  **Acceptance tests:**
* A run producing diff/report/logs emits tracked artifacts with metadata.
* Artifacts can be retrieved by id from session history.
* Exported artifacts are reproducibly referenced in CLI JSON output.
  **Owner:** Tools/CLI

---

## EPIC 5 — MCP client + integration

### ISSUE 0501 — MCP client (stdio transport) (ucode-mcp)

**Goal:** Implement MCP client able to discover and call tools over stdio.
**Scope/Notes:**

* JSON-RPC style framing
* Tool discovery → ToolSpec conversion
  **Acceptance tests:**
* Connect to a dummy MCP server and call a tool successfully.
  **Owner:** MCP

### ISSUE 0502 — MCP registry integration (ucode-tools)

**Goal:** Expose MCP tools through ToolRegistry with namespacing.
**Scope/Notes:**

* Names: `mcp.<server>.<tool>`
* Collision handling
  **Acceptance tests:**
* `list_tools` includes MCP tool; invoking it works.
  **Owner:** MCP/Tools

### ISSUE 0503 — Native MCP launcher support (`uvx`/`npx`/`bunx`/binary) (ucode-mcp)

**Goal:** Natively launch user-installed MCP servers across common package ecosystems.
**Scope/Notes:**

* Configurable launcher definitions in config:
  * `uvx <pkg> [args...]`
  * `npx <pkg> [args...]`
  * `bunx <pkg> [args...]`
  * direct executable path
* Capture runtime metadata: version, executable path, startup timeout, health status
* Validate command schema before launch
* First-run trust prompt with persisted decision
* Server identity fingerprint (command + package + version hash)
* Fingerprint drift detection: command/package/version changes re-trigger trust prompt
  **Acceptance tests:**
* MCP tools are discoverable and callable for each launcher mode.
* Startup timeout and invalid command errors handled clearly.
* Command/package/version drift triggers trust re-approval.
  **Owner:** MCP/Security

### ISSUE 0504 — MCP per-server policy + lifecycle controls (ucode-mcp + ucode-tools)

**Goal:** Treat each MCP server as an isolated trust domain with managed lifecycle.
**Scope/Notes:**

* Per-server sandbox tier, network policy, and tool permission profile
* Namespaced server identity to prevent command substitution drift
* Managed lifecycle: start/stop/restart with crash diagnostics
* Health check + auto-restart with backoff
* Full audit events: launch, approval, deny, crash, restart
  **Acceptance tests:**
* Untrusted server cannot execute tools until approved.
* Per-server policy is enforced at invocation time.
* Crash/restart cycle produces clear diagnostics in logs.
  **Owner:** MCP/Tools/Security

### ISSUE 0505 — MCP transport parity (stdio + SSE + HTTP) (ucode-mcp) [P0]

**Goal:** Match common agentic-tool MCP connectivity by supporting stdio, SSE, and HTTP transports.
**Scope/Notes:**

* Add configurable MCP transports: stdio, SSE, HTTP
* Auth header/token config for networked transports
* Reconnect/backoff strategy for unstable remote transports
* Transport capability and health visible in logs/TUI
  **Acceptance tests:**
* Tool discovery/invocation works across stdio, SSE, and HTTP servers.
* Transport disconnect triggers bounded reconnect with diagnostics.
* Auth failures on remote transport surface clear actionable errors.
  **Owner:** MCP

### ISSUE 0506 — MCP resources/prompts integration (ucode-mcp + ucode-tools + ucode-core) [P0]

**Goal:** Support MCP resources and prompts in addition to MCP tools for compatibility with modern agent ecosystems.
**Scope/Notes:**

* Discover/list MCP resources and prompts with server namespacing
* Resolve/read resource payloads through policy/sandbox/audit pipeline
* Prompt invocation and variable binding flow integrated into command/session context
* Collision handling across servers for resource/prompt identifiers
  **Acceptance tests:**
* Resources and prompts are discoverable and invokable from registered MCP servers.
* Resource/prompt access obeys policy and audit requirements.
* Namespace collisions are handled deterministically.
  **Owner:** MCP/Tools/Core

---

## EPIC 6 — Skills (Claude Code/OpenCode compatible)

### ISSUE 0601 — SKILL.md discovery (ucode-skills)

**Goal:** Discover skills from common paths.
**Scope/Notes:**
Search:

* `.claude/skills/*/SKILL.md`
* `.agents/skills/*/SKILL.md`
* `skills/*/SKILL.md`
* `~/.config/ucode/skills/*/SKILL.md`
  **Acceptance tests:**
* Put a sample SKILL.md in each path; tool lists them.
  **Owner:** Skills

### ISSUE 0602 — SKILL.md parsing (YAML frontmatter + markdown body) (ucode-skills)

**Goal:** Parse `name`, `description` (min) and instruction body.
**Scope/Notes:**

* Ignore unknown keys
* Support optional `ucode:` namespace (tool allowlists, routing hints)
  **Acceptance tests:**
* Parse sample; unknown keys don’t break.
  **Owner:** Skills

### ISSUE 0603 — Skill execution binding (ucode-core + skills)

**Goal:** Use skill instructions as prompt prefix; enforce skill tool policy.
**Acceptance tests:**

* Selecting a skill changes system prompt; tool allowlist enforced.
  **Owner:** Skills/Core

---

## EPIC 7 — Fullscreen TUI

### ISSUE 0701 — Ratatui fullscreen shell + panes (ucode-tui)

**Goal:** Build base TUI: transcript, input box, sidebar, status bar.
**Acceptance tests:**

* Launch TUI; type prompt; see streaming output.
  **Owner:** TUI

### ISSUE 0702 — Command palette + keybinds (ucode-tui)

**Goal:** Implement command palette and core keybinds.
**Scope/Notes:**

* `Ctrl+P` palette
* `/connect`, `/skills`, `/models`, `/tools`
  **Acceptance tests:**
* Palette opens; commands execute.
  **Owner:** TUI

### ISSUE 0703 — Diff viewer + apply/reject UX (ucode-tui)

**Goal:** When `Event::Patch` arrives, show diff modal; allow apply/reject.
**Acceptance tests:**

* Model proposes patch → diff modal appears → apply modifies file.
  **Owner:** TUI/Tools

### ISSUE 0704 — Tool call log + approvals UX (ucode-tui)

**Goal:** Show tool calls in UI and require approvals for `run_cmd` (and optionally patch).
**Acceptance tests:**

* Tool call displayed; run_cmd prompts approval; deny blocks execution.
  **Owner:** TUI

### ISSUE 0705 — /connect UI (providers + auth method picker) (ucode-tui + auth)

**Goal:** In-TUI provider connect flow for API keys + login + subscription login.
**Acceptance tests:**

* `/connect` → choose provider → complete method → status updates.
  **Owner:** TUI/Auth

### ISSUE 0706 — Sidebar-first visual system and safety state UX (ucode-tui)

**Goal:** Deliver polished, distinctive TUI UX with persistent operational awareness via sidebar.
**Scope/Notes:**

* Visual token system: color roles, spacing, border emphasis, semantic states (safe/warning/danger)
* Sidebar priority panels:
  * active provider/model + effective sandbox tier
  * active skill
  * context pack summary
  * tool call queue with approval state
  * subagent status panel (running/completed/failed)
  * network state indicator
* Transparent-friendly theme profile (terminal-dependent; no hard dependency)
* Compact/comfortable density presets
* Keyboard-first focus behavior across all panes
  **Acceptance tests:**
* Critical safety state (sandbox tier, approvals, network) remains visible during normal chat flow.
* Sidebar remains usable at common terminal dimensions (80x24 minimum).
* Theme toggle preserves contrast/accessibility in supported terminals.
  **Owner:** TUI

### ISSUE 0707 — Slash command UX + registry integration (ucode-tui + ucode-core)

**Goal:** Make `/command` first-class in the TUI with discoverability and safe execution.
**Scope/Notes:**

* Execute slash commands from input line and command palette
* Show command source badge: user/project/plugin
* Inline argument hints and validation messages
* Command execution routed through same sandbox/approval pipeline
  **Acceptance tests:**
* User-defined `/command` resolves and executes from TUI input.
* Unknown command returns ranked suggestions.
* Command execution enforces normal policy gates.
  **Owner:** TUI/Core

---

## EPIC 8 — Plugins + hooks (user customization)

### ISSUE 0801 — Plugin manifest + loader (ucode-plugins)

**Goal:** Define plugin manifests/registry and lifecycle contracts without introducing runtime complexity early.
**Scope/Notes:**

* Plugin manifest + discovery/registration flow
* Register hooks + optional tools + declared capabilities
* Runtime implementation deferred to latest-stage WASM issue
  **Acceptance tests:**
* Example plugin manifest is discovered and validated.
* Capability declarations load into policy engine.
  **Owner:** Plugins

### ISSUE 0802 — Hooks API v1 (ucode-plugins + core/tools)

**Goal:** Define hook events and dispatch them.
**Hooks:**

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
* Override classes matrix enforced by host policy:

| Class | Examples | Auto-apply | Requires explicit approval |
| --- | --- | --- | --- |
| Safe | fallback ranking hints, logging verbosity, context-shrink preference | Yes (policy-safe) | No |
| Guarded | model group preference changes within limits, retry tuning in bounds | Yes (bounded) | No |
| Risky | widening network/filesystem/process permissions, bypassing deny rules | No | Yes |
  **Acceptance tests:**
* Example plugin logs router/fallback decisions and can propose safe fallback preferences.
* Unsafe behavior override attempt is blocked unless explicitly user-approved.
* Risky class override is never auto-applied.
  **Owner:** Plugins/Core

### ISSUE 0803 — Plugin tool registration (ucode-plugins + ucode-tools)

**Goal:** Allow WASM plugins to add tools to registry safely.
**Scope/Notes:**

* Plugin declares tools during initialization using typed `ToolSpec`
* Host namespaces each tool as `plugin.<plugin-id>.<tool-name>`
* Host validates schemas and capability requirements before activation
* Plugin tool calls flow through normal sandbox/approval/audit pipeline
**Acceptance tests:**

* Plugin registers `hello_tool`; it appears in `list_tools` and can be invoked.
* Plugin tool with disallowed capability is rejected at activation.
* Plugin tool invocation obeys same policy gates as built-in tools.
  **Owner:** Plugins/Tools

### ISSUE 0804 — Versioned plugin API contract + Rust SDK (ucode-plugins)

**Goal:** Define stable plugin API and Rust-first SDK for WASM plugins.
**Scope/Notes:**

* Versioned WIT/component-model contract (`v1`)
* Handshake/capability negotiation and hook subscription model
* Structured request/response/error envelope
* Rust-first SDK for WASM plugin authors
* Manifest fields for minimum API version and requested capabilities
  **Acceptance tests:**
* Rust WASM sample plugin handshake and hook flow compile against contract.
* Version mismatch returns explicit incompatibility error.
  **Owner:** Plugins

### ISSUE 0805 — WASM plugin runtime isolation model (latest stage) (ucode-plugins + security)

**Goal:** Implement WASM-only plugin runtime in the latest stage, with strict safety controls.
**Scope/Notes:**

* WASM component runtime with policy-gated capabilities
* Per-plugin policy profile: filesystem scope, network, command spawn, hook scope
* Plugin-originated actions must pass normal approval/sandbox/audit pipeline
* Runtime is scheduled for latest stage to avoid early complexity
  **Acceptance tests:**
* Untrusted plugin cannot exceed granted permissions.
* Plugin-originated action triggers normal approval/sandbox checks.
* Runtime model and effective plugin permissions are visible in logs/UI.
  **Owner:** Plugins/Security

### ISSUE 0806 — External DCP-style plugin compatibility + public hook surface (ucode-plugins + core) [P0]

**Goal:** Ensure users can ship their own DCP-style plugins (for example `opencode-dcp`) using documented, stable hook contracts.
**Scope/Notes:**

* Hooks required for DCP-style plugins are public and documented (not host-internal only):
  * `on_context_shrink`, `on_context_distilled`
  * `on_session_title_generated/updated`
  * `on_session_start/end`, `before/after_tool_call`
* Hook payload schema/versioning policy documented so user plugins can parse safely across host upgrades
* Capability guardrails remain enforced: DCP plugin cannot widen sandbox/network/filesystem beyond effective policy
* User/plugin discovery path supports project and user-level plugin install for external DCP plugins
  **Acceptance tests:**
* Sample external plugin `opencode-dcp` (fixture) loads from user plugin path and receives documented hooks.
* Plugin can observe compaction/distillation and session-title events and emit a policy-safe recommendation.
* Plugin attempt to escalate permissions is blocked and auditable.
  **Owner:** Plugins/Core/Security

### ISSUE 0807 — Plugin install/update distribution with trust verification (ucode-plugins + security) [P1]

**Goal:** Let users install/update external plugins from git/url/registry with signature/trust verification.
**Scope/Notes:**

* Plugin install/update commands for remote sources (git/url/registry)
* Signature/fingerprint verification before activation
* Trust record and drift detection on updates
* Rollback path for failed or untrusted updates
  **Acceptance tests:**
* Signed trusted plugin installs and activates successfully.
* Signature mismatch or trust failure blocks activation with clear diagnostics.
* Update drift triggers re-approval and preserves previous working version.
  **Owner:** Plugins/Security

---

## EPIC 9 — End-to-end integration + release quality

### ISSUE 0901 — Integrated “happy path” scenario test

**Goal:** Confirm full workflow works together.
**Scenario:**

* `/connect` (OpenAI or Anthropic)
* load a skill from `.claude/skills/.../SKILL.md`
* `ripgrep_search` finds code location
* model proposes patch → apply → run tests
* force provider error → fallback occurs
  **Acceptance tests:**
* Scripted integration test (can be semi-manual initially) documented in `docs/e2e.md`.
  **Owner:** Platform

### ISSUE 0902 — Config file + docs

**Goal:** Document config keys and default behaviors.
**Acceptance tests:**

* `docs/config.md` exists + example config.
  **Owner:** Platform/Docs

### ISSUE 0903 — Packaging + distribution (Linux/macOS)

**Goal:** Provide install instructions and artifacts.
**Scope/Notes:**

* `cargo install` works
* optional: release binaries via GitHub Releases
  **Acceptance tests:**
* Fresh machine install instructions succeed.
  **Owner:** Platform

### ISSUE 0904 — Security threat model and audit trail verification (platform-wide)

**Goal:** Formalize trust boundaries and verify all high-risk transitions are auditable.
**Scope/Notes:**

* Document trust model covering: model output, tool runtime, MCP servers, plugins, subagents, user approvals
* Define event taxonomy for audit trail: approval, denial, fallback, sandbox tier, network decision, MCP launch, agent lifecycle
* Add integration scenario that exercises denial/approval/sandbox fallback path end-to-end
  **Acceptance tests:**
* `docs/security-threat-model.md` exists and maps to implemented controls.
* E2E run produces auditable records for each policy gate event.
* Audit trail verifiable for: tool approvals, sandbox tier changes, MCP trust decisions, agent spawn/complete.
  **Owner:** Platform/Security

### ISSUE 0905 — Non-interactive headless/CI execution mode (ucode-cli + ucode-core) [P0]

**Goal:** Enable deterministic headless execution for CI/automation with machine-readable outputs and strict exit codes.
**Scope/Notes:**

* CLI non-interactive mode for scripted runs with no TUI prompts
* JSON output envelope with events, artifacts, usage, and final status
* Deterministic exit codes by terminal result class (success, policy deny, tool failure, timeout)
* Support resume-by-session-id in non-interactive flows
  **Acceptance tests:**
* CI runner command executes end-to-end without interactive input.
* JSON output includes terminal status + artifact references.
* Exit codes map correctly to failure/success classes.
  **Owner:** CLI/Core

---

# Suggested initial milestone ordering (so agents don't block each other)

**Milestone M1 (MVP CLI):** 0001, 0101–0105, 0108–0111, 0201–0202, 0301–0302 (one provider), 0401, 0403–0406, 0408–0409, 0107
**Milestone M2 (MVP TUI):** 0701–0704, 0706, 0707
**Milestone M3 (Compatibility + MCP):** 0601–0603, 0501–0506, 0305
**Milestone M4 (Plugins contracts + auth upgrades + subagents):** 0801–0803, 0203–0205, 0705, 0106, 0410–0413
**Milestone M5 (Polish + security + WASM runtime):** 0901–0905, 0804–0807

---
