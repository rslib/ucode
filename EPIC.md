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

### ISSUE 0106 — Inter-agent communication bus (ucode-core) [DONE]

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

### ISSUE 0108 — Token budget manager + context compaction/distillation pipeline (ucode-core) [DONE]

**Goal:** Keep conversations reliable when nearing model context limits by applying deterministic compaction/distillation before hard failure.
**Scope/Notes:**

* Per provider/model token budget estimator (input + reserved output budget)
* Preflight context-fit check before each provider request
* Counting strategy order:

  * use provider-native token counting when adapter supports it
  * otherwise use local tokenizer-based estimator + conservative safety margin
* Budget envelope fields tracked per request: `max_context`, reserved output, available input
* Hybrid compaction modes:

  * rule-based/no-model compaction path (trim + deterministic packing)
  * optional model-assisted summarization path (same model or smaller summarizer model)
  * system must not require a smaller model to compact successfully
* Progressive recovery pipeline (deterministic order):

  * trim low-value context artifacts (verbose logs/tool chatter)
  * compact older turns into concise summaries
  * distill long tool outputs into structured memory records
  * preserve recent turns and unresolved tool-call context as pinned
* Retry policy: compact/distill and retry until within budget or explicit terminal error
* Persist compaction/distillation artifacts in session store with provenance/audit linkage
* Emit routing/runtime logs for each compaction step and final token budget used
* Persist count source (`provider_count` vs `local_estimate`) for observability/debugging
  **Acceptance tests:**
* Oversized transcript triggers compaction/distillation and request succeeds without user intervention.
* Pinned recent turns and unresolved tool context remain intact after compaction.
* Distilled artifacts reload with session and remain auditable.
* When provider count API is unavailable, local estimate path is used and guarded by safety margin.
* Rule-based path works without any summarizer model; model-assisted path is optional and policy-gated.
  **Owner:** Core

### ISSUE 0109 — Session lifecycle + model-generated session titles (ucode-core + ucode-cli + ucode-tui) [DONE]

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

### ISSUE 0110 — Session resume/fork lineage model (ucode-core + ucode-cli + ucode-tui) [P0] [DONE]

**Goal:** Support deterministic session resume and branch/fork workflows for parallel exploration without losing transcript lineage.
**Scope/Notes:**

* Resume by id from CLI/TUI with full state restore (model, skill, policy, transcript)
* `session continue` resumes the most recently updated non-archived session (convenience shortcut)
* Fork session creates child session with parent pointer and lineage metadata
* Session list/switch UI exposes parent-child lineage and fork source
* Audit events for resume/fork/switch actions
  **Acceptance tests:**
* `resume(session_id)` restores full runnable state.
* `fork(session_id)` creates a child session with correct ancestry.
* `session continue` resolves to the most recent active session.
* TUI and CLI can switch between parent/child sessions without state bleed.
  **Owner:** Core/CLI/TUI

### ISSUE 0111 — Token/cost governance and budget controls (ucode-core + ucode-providers + ucode-tui) [P1] [DONE]

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

### ISSUE 0112 — Structured logging subsystem (stderr + file, session + rolling) (ucode-core + ucode-cli + ucode-tui) [DONE]

**Goal:** Provide production-grade diagnostics with explicit log levels and sinks, while keeping noisy levels disabled by default.
**Scope/Notes:**

* Log levels: `ERROR`, `WARN`, `INFO`, `DEBUG`, `TRACE`
* Defaults: `INFO` by default; `DEBUG`/`TRACE` enabled only via explicit user config/flag
* Runtime control surfaces:

  * env vars (for example `UCODE_LOG_LEVEL`, `UCODE_LOG_STDERR`, `UCODE_LOG_FILE`, `UCODE_LOG_DIR`, `UCODE_LOG_ROLLING`)
  * CLI flags (for example `--log-level`, `--log-file`, `--log-dir`, `--log-stderr`, `--trace`)
* Env var semantics:

  * `UCODE_LOG_LEVEL`: `error|warn|info|debug|trace`
  * `UCODE_LOG_STDERR`: boolean sink toggle (`1/0`, `true/false`)
  * `UCODE_LOG_FILE`: file path override for log sink
  * `UCODE_LOG_DIR`: directory override for session and rolling log files
  * `UCODE_LOG_ROLLING`: boolean toggle for global rolling log
* Precedence for logging controls: CLI flags > env vars > config file > built-in defaults
* Multi-sink output:

  * stderr sink for interactive visibility
  * file sink for persistence and post-mortem analysis
* Stdout policy: reserve stdout for command/model result payloads and machine-readable output; logs go to stderr/file by default
* Retention strategy (hybrid):

  * per-session file as primary user-facing log (`session-<id>.log`)
  * optional rolling global log for cross-session diagnostics
* XDG-compliant log path defaults:

  * default log dir: `${XDG_STATE_HOME:-~/.local/state}/ucode/logs`
  * if `UCODE_LOG_DIR` or `--log-dir` is set, use that path explicitly
* Config root override for testability/integration:

  * `UCODE_HOME` defaults to `${XDG_CONFIG_HOME:-~/.config}/ucode`
  * canonical config file path: `${UCODE_HOME}/ucode.toml`
* Structured fields on each event: timestamp, level, session_id, agent_id, provider/model, tool_name, event_type
* Redaction guardrails to avoid secrets in logs
  **Acceptance tests:**
* Default run emits `INFO+` to stderr and per-session file.
* Default run keeps stdout clean for normal output/JSON mode consumers.
* `DEBUG` and `TRACE` are silent unless explicitly enabled.
* CLI and env var overrides apply deterministically using documented precedence.
* Per-session logs are easily attributable to a single session; rolling log rotates by size/time policy.
* With no explicit override, logs are written under XDG state log path.
* Sensitive values are redacted in both sinks.
  **Owner:** Core/CLI/TUI/Security

---

## EPIC 2 — Auth (API keys + login + subscription login)

### ISSUE 0201 — Secure credential store (keychain) (ucode-auth) [DONE]

**Goal:** Provide credential storage for Linux + macOS using OS keychain.
**Scope/Notes:**

* Use `keyring` crate
* Do not store secrets in plaintext config
* Provide `AuthMaterial::{ApiKey, OAuth, SessionToken}`
  **Acceptance tests:**
* `ucode auth set-key openai` stores key, `ucode auth status` shows configured, restarting keeps it.
* Verify config file contains no secret material.
  **Owner:** Auth

### ISSUE 0202 — Auth CLI commands (ucode-cli + ucode-auth) [DONE]

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

### ISSUE 0301 — Provider trait + capability model (ucode-providers) [DONE]

**Goal:** Define provider interface that returns canonical `Event` stream.
**Scope/Notes:**

* `Provider::stream_chat(req) -> EventStream`
* `Provider::capabilities() -> Capabilities { tool_calls, json_mode, max_context, max_output, streaming, token_counting }`
* Optional `count_tokens(req)` adapter method for provider-native counting
  **Acceptance tests:**
* Mock provider compiles and streams tokens + done.
* Capability matrix correctly reports token-count availability.
  **Owner:** Providers

### ISSUE 0302 — OpenAI adapter (streaming + tools) (ucode-providers) [DONE]

**Goal:** Implement OpenAI-compatible chat adapter with streaming and tool calls.
**Scope/Notes:**

* SSE streaming → `Event::Token`
* Tool calls translated → canonical `ToolCall`
* Uses `ucode-auth` credentials (API key or login)
  **Acceptance tests:**
* `ucode chat --provider openai` streams output (can be mocked)
* Tool-call event emitted for a tool-using prompt (mock ok)
  **Owner:** Providers

### ISSUE 0303 — Anthropic adapter (streaming + tools) (ucode-providers) [DONE]

**Goal:** Implement Anthropic adapter with streaming and tool use.
**Scope/Notes:**

* Translate tool-use blocks to canonical `ToolCall`
* Auth: API key + subscription login token
  **Acceptance tests:**
* `ucode chat --provider anthropic` streams output (mock ok)
  **Owner:** Providers

### ISSUE 0304 — Ollama/local adapter (optional but recommended) (ucode-providers) [DONE]

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

## Design principle: Rust-native tooling

All user-facing tools must be implemented as baked-in Rust libraries — no shelling out to external CLIs.
This ensures consistent behavior, no runtime dependency on installed binaries, and full control over error handling.

| Capability | Rust library | Notes |
|---|---|---|
| File search (ripgrep-like) | `ignore` + `regex` | `ignore` is from the ripgrep ecosystem; gitignore-aware walking |
| Patch apply (unified diff) | `mpatch` | Fuzzy context matching, designed for AI-generated diffs |
| AST structural search/rewrite | `ast-grep-core` + `ast-grep-language` | Tree-sitter based; pattern matching on syntax trees |
| Git operations | `gix` | Pure-Rust git implementation |
| Command execution | `tokio::process` | Async process spawning with timeout/output caps |

---

## EPIC 4 — Built-in tools + permissions

### ISSUE 0401 — Tool registry + invocation runtime (ucode-tools) [DONE]

**Goal:** Create unified tool registry for built-in + MCP + plugins.
**Scope/Notes:**

* `ToolSpec { name, description, input_schema }`
* `ToolHandler` async
* `ToolRegistry::{register, list, invoke}`
  **Acceptance tests:**
* `list_tools` returns registered tools; invoke demo tool returns ToolResult.
  **Owner:** Tools

### ISSUE 0402 — Permissions system (ucode-tools) [DONE]

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

### ISSUE 0403 — Built-in filesystem tools (ucode-tools) [DONE]

**Goal:** Implement `read_file`, `list_dir` with gitignore-aware behavior.
**Acceptance tests:**

* Read a file inside repo succeeds; outside repo fails by policy.
  **Owner:** Tools

### ISSUE 0404 — Built-in search tool (ripgrep-like) (ucode-tools) [DONE]

**Goal:** Implement `ripgrep_search(query, paths?, max_results?)`.
**Scope/Notes:**

* Rust-native: `ignore` crate (gitignore-aware walking) + `regex` crate (matching)
* No CLI shelling — all baked-in Rust
* Supports max_results cap, include_glob filter, context_lines
  **Acceptance tests:**
* Search finds expected match; respects .gitignore.
  **Owner:** Tools

### ISSUE 0405 — Patch apply tool (unified diff) (ucode-tools) [DONE]

**Goal:** Implement `apply_patch(diff)` robustly using `mpatch` crate.
**Scope/Notes:**

* Rust-native: `mpatch` crate — fuzzy context matching designed for AI-generated diffs
* No CLI shelling — all baked-in Rust
* `mpatch::parse_auto()` handles both raw unified diffs and markdown-embedded diffs
* `mpatch::apply_patches_to_dir()` applies patches with fuzzy matching + smart indentation
* Built-in path traversal protection
* Return `applied`, `files_changed`, `rejects` with reasons
  **Acceptance tests:**
* Apply patch to sample file; verify content changed.
* Invalid patch returns rejects with reason.
* Shifted/fuzzy-context patch applies correctly.
* Markdown-embedded diff block parsed and applied.
  **Owner:** Tools

### ISSUE 0406 — Command runner tool (ucode-tools) [DONE]

**Goal:** Implement `run_cmd` with timeouts/output caps and approval gating.
**Acceptance tests:**

* `run_cmd("echo hi")` returns output.
* `run_cmd("sleep 10", timeout=1)` times out cleanly.
  **Owner:** Tools

### ISSUE 0407 — Git helpers (optional) (ucode-tools) [DONE]

**Goal:** Provide comprehensive git tooling using `gix` (pure-Rust git).
**Scope/Notes:**

* Rust-native: `gix` crate — no shelling out to `git` CLI
* 17 tools organized in `git/` module directory:
  * **Read:** `git_status`, `git_diff`, `git_diff_staged`, `git_diff_commits`, `git_log`, `git_show`
  * **Write:** `git_add`, `git_commit`, `git_tag`, `git_stash`
  * **Branch:** `git_branch`, `git_checkout`, `git_reset`, `git_restore`
  * **Merge:** `git_merge`, `git_cherry_pick`, `git_rebase` (full interactive with pick/squash/reword/drop)
* Conflict handling: returns conflict markers in worktree + conflict file list
* `register_all_git_tools()` convenience function
  **Acceptance tests:**
* 73 tests covering all 17 tools
  **Owner:** Tools

### ISSUE 0407b — AST structural search/rewrite tool (ucode-tools) [DONE]

**Goal:** Provide AST-aware code search and rewrite using `ast-grep-core`.
**Scope/Notes:**

* Rust-native: `ast-grep-core` + `ast-grep-language` crates (tree-sitter based)
* No CLI shelling — all baked-in Rust
* `ast_search(pattern, path, lang)`: find code matching an AST pattern
* `ast_rewrite(pattern, replacement, path, lang)`: structural find-and-replace
* Supports major languages via tree-sitter grammars: Rust, Python, TypeScript, JavaScript, Go, C, C++
* Pattern syntax: write code patterns with `$VAR` wildcards (e.g., `console.log($MSG)`)
  **Acceptance tests:**
* AST search finds structural matches that regex would miss (e.g., ignoring whitespace/comments).
* AST rewrite correctly transforms matched code.
* Language detection works for common file extensions.
  **Owner:** Tools

### ISSUE 0408 — Fine-grained hierarchical sandbox policy engine (ucode-tools) [DONE]

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

### ISSUE 0409 — Outside-project confirmation and boundary enforcement (ucode-tools + ucode-tui) [DONE]

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

### ISSUE 0410 — Network capability policy for web/deep research (ucode-tools) [DONE]

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

### ISSUE 0411 — Workspace checkpoints + rollback safety controls (ucode-tools + ucode-core + ucode-tui) [P1] [DONE]

**Goal:** Add lightweight checkpoints before risky actions so users can quickly revert agent-side modifications.
**Scope/Notes:**

* `CheckpointStore`: create/list/restore/delete/prune/get with directory-copy snapshots
* Storage: `.ucode/.checkpoints/{id}/meta.json` + `files/{relative_path}`
* `RetentionPolicy`: max_count (default 10) and max_age based pruning
* `CheckpointInfo`: id, name, description, created_at, file_count, total_bytes
* `CheckpointError`: NotFound, Io, Metadata variants
* 13 tests
* TUI rollback actions deferred to TUI phase
  **Acceptance tests:**
* Applying a patch creates a checkpoint and rollback restores pre-patch state.
* Command-induced workspace changes can be reverted to prior checkpoint.
* Expired checkpoints are pruned according to retention policy.
  **Owner:** Tools/Core/TUI

### ISSUE 0412 — Background job controller + interactive cancel/kill (ucode-tools + ucode-core + ucode-tui) [P0] [DONE]

**Goal:** Support detached long-running operations with explicit lifecycle control, including interactive kill from TUI.
**Scope/Notes:**

* `JobController`: start/list/status/cancel/kill/wait/prune_completed
* `JobState`: Queued/Running/Completed/Failed/Cancelled/Killed with `is_terminal()`
* Graceful cancel (cancel_tx) vs force kill (cancel_tx + kill_tx) signals
* One-shot result consumption via `wait()`, prevents double-await
* State set before signal sent (prevents race with task completion)
* Task only updates state if not already terminal (cancel/kill wins over natural completion)
* 15 tests
* TUI background jobs panel deferred to TUI phase
  **Acceptance tests:**
* Long-running job can be detached and monitored while chat remains responsive.
* User can cancel and force-kill jobs interactively from TUI.
* Job lifecycle transitions are auditable and persisted to session metadata.
  **Owner:** Tools/Core/TUI

### ISSUE 0413 — Structured artifact output and export pipeline (ucode-tools + ucode-cli) [P1] [DONE]

**Goal:** Produce and export structured artifacts (reports, diffs, run logs) for machine and human workflows.
**Scope/Notes:**

* `ArtifactStore`: create/get/read_content/list/list_by_type/list_by_session/verify/delete/export
* `ArtifactEnvelope`: id, type, source, title, metadata, checksum, content_size, created_at, session_id, tool_call_id
* `ArtifactType`: MarkdownReport, UnifiedDiff, CommandLog, TestLog
* Storage: `{base_dir}/{artifact_id}/envelope.json` + `content`
* Checksum via DefaultHasher (no sha2 dependency)
* Integrity verification via `verify()` method
* 14 tests
  **Acceptance tests:**
* A run producing diff/report/logs emits tracked artifacts with metadata.
* Artifacts can be retrieved by id from session history.
* Exported artifacts are reproducibly referenced in CLI JSON output.
  **Owner:** Tools/CLI

---

## EPIC 5 — MCP client + integration

### ISSUE 0501 — MCP client (stdio transport) (ucode-mcp) [DONE]

**Goal:** Implement MCP client able to discover and call tools over stdio.
**Scope/Notes:**

* JSON-RPC style framing
* Tool discovery → ToolSpec conversion
  **Acceptance tests:**
* Connect to a dummy MCP server and call a tool successfully.
  **Owner:** MCP

### ISSUE 0502 — MCP registry integration (ucode-tools) [DONE]

**Goal:** Expose MCP tools through ToolRegistry with namespacing.
**Scope/Notes:**

* Names: `mcp.<server>.<tool>`
* Collision handling
* `McpBridge` facade + `McpToolHandler` implementing `ToolHandler`
* `register_tool_defs()` testable free function (no live server needed)
* `parse_namespaced()` for reverse lookup
* 16 tests
  **Acceptance tests:**
* `list_tools` includes MCP tool; invoking it works.
  **Owner:** MCP/Tools

### ISSUE 0503 — Native MCP launcher support (`uvx`/`npx`/`bunx`/binary) (ucode-mcp) [DONE]

**Goal:** Natively launch user-installed MCP servers across common package ecosystems.
**Scope/Notes:**

* `LauncherType`: Uvx, Npx, Bunx, Binary — each maps to wrapper command
* `LauncherDef`: launcher_type, package, args, env, startup_timeout
* `ServerIdentity`: fingerprint (DefaultHasher hex of canonical command string), command_line, created_at
* `TrustRecord`: identity, trusted, decided_at, decided_by
* `TrustStatus`: Trusted, Untrusted, FingerprintDrifted
* `launcher_to_command()`: returns (command, args) tuple for `StdioTransport::spawn()`
* `compute_fingerprint()`: deterministic hash of `"{type}:{package}:{sorted_args}"`
* Trust cache: JSON file at `{base_dir}/.ucode/trust.json` with load/save/verify
* `LauncherNotTrusted` and `FingerprintDrift` error variants added to McpError
* 13 tests
  **Acceptance tests:**
* MCP tools are discoverable and callable for each launcher mode.
* Startup timeout and invalid command errors handled clearly.
* Command/package/version drift triggers trust re-approval.
  **Owner:** MCP/Security

### ISSUE 0504 — MCP per-server policy + lifecycle controls (ucode-mcp + ucode-tools) [DONE]

**Goal:** Treat each MCP server as an isolated trust domain with managed lifecycle.
**Scope/Notes:**

* `ServerTier`: Trusted/Sandboxed/Untrusted (default Untrusted)
* `ServerPolicy`: per-server tier, network policy, tool permission, restart config
* `ToolPermission`: AllowAll/AllowList/DenyAll with `check_tool_permission()`
* `ServerLifecycle`: state machine (Stopped/Starting/Running/Crashed/Restarting)
* Exponential backoff restart: `base_ms * 2^(attempt-1)` with overflow safety
* `AuditEvent`/`AuditEventType`: timestamped lifecycle audit records
* `ServerPolicyStore`: HashMap-backed registry for multiple servers
* 19 tests
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

### ISSUE 0506 — MCP resources/prompts integration (ucode-mcp + ucode-tools + ucode-core) [P0] [DONE]

**Goal:** Support MCP resources and prompts in addition to MCP tools for compatibility with modern agent ecosystems.
**Scope/Notes:**

* Resource types: `McpResourceDef`, `McpResourceContent` (text + blob)
* Prompt types: `McpPromptDef`, `McpPromptArgument`, `McpPromptMessage`, `McpPromptMessageContent`
* `McpClient` methods: `list_resources()`, `read_resource()`, `list_prompts()`, `get_prompt()`
* Capability checks: `supports_resources()`, `supports_prompts()`
* `ServerCapabilities` extended with `resources` and `prompts` fields
* `McpResourceRegistry`: collision-detecting registry with namespacing (`mcp.<server>.<name>`)
* `NamespacedResource`/`NamespacedPrompt` with server origin tracking
* 19 new tests (9 type tests + 10 registry tests)
  **Acceptance tests:**
* Resources and prompts are discoverable and invokable from registered MCP servers.
* Resource/prompt access obeys policy and audit requirements.
* Namespace collisions are handled deterministically.
  **Owner:** MCP/Tools/Core

---

## EPIC 6 — Skills (Claude Code/OpenCode compatible)

### ISSUE 0601 — SKILL.md discovery (ucode-skills) [DONE]

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

### ISSUE 0602 — SKILL.md parsing (YAML frontmatter + markdown body) (ucode-skills) [DONE]

**Goal:** Parse `name`, `description` (min) and instruction body.
**Scope/Notes:**

* Ignore unknown keys
* Support optional `ucode:` namespace (tool allowlists, routing hints)
  **Acceptance tests:**
* Parse sample; unknown keys don’t break.
  **Owner:** Skills

### ISSUE 0603 — Skill execution binding (ucode-core + skills) [DONE]

**Goal:** Use skill instructions as prompt prefix; enforce skill tool policy.
**Scope/Notes:**

* `SkillBinding`: system prompt prefix, tool filter, routing hints, preferred model group
* `SkillManager`: activate/deactivate/switch skills, active tool filter, active system prefix
* `ToolFilter::AllowAll | AllowList(HashSet)` with `is_allowed()` check
* Empty allowlist = all tools permitted
* 16 tests
  **Acceptance tests:**
* Selecting a skill changes system prompt; tool allowlist enforced.
  **Owner:** Skills/Core

---

## EPIC 7 — Fullscreen TUI

> **Design spec:** `docs/plans/2026-03-05-tui-design.md` — full component inventory, visual system, plugin UI API, and ASCII layout sketches. All issues in this epic implement that spec.

### ISSUE 0701 — Ratatui fullscreen shell + panes (ucode-tui) [DONE]

**Goal:** Build base TUI: transcript, input box, sidebar, status bar.
**Scope/Notes:**

* Includes terminal compatibility: tmux/zellij/screen detection, alternate screen, true color detection, SIGWINCH resize (see Task 7.7)
* Clipboard: OSC 52 with external/file fallback (see Task 7.7)
* Mouse support toggle for tmux coexistence

**Acceptance tests:**

* Launch TUI; type prompt; see streaming output.
* `[tmux]` indicator shows in status bar when inside tmux.
* Clipboard works via OSC 52 inside tmux.
  **Owner:** TUI

**Completed:** 742 tests, 5 phases (0701a–0701e), 11 modules.

### ISSUE 0702 — Command palette + keybinds (ucode-tui) [DONE]

**Goal:** Implement command palette and core keybinds.
**Scope/Notes:**

* `Ctrl+P` palette
* `/connect`, `/skills`, `/models`, `/tools`
* Keybinding presets: vscode (default), vim (modal), emacs (Meta+x) (see Task 7.8)
* Config: `[tui.keybinds] preset = "vscode"` with individual override layering

**Acceptance tests:**

* Palette opens; commands execute.
* `preset = "vim"` activates modal editing; `preset = "emacs"` activates emacs nav.
  **Owner:** TUI

**Completed:** 767 tests, 4 phases (0702a-0702e). Palette overlay with 10 built-in commands, fuzzy filtering, keyboard navigation. Input editing keys wired (Backspace/Delete/arrows/Home/End).

### ISSUE 0703 — Diff viewer + apply/reject UX (ucode-tui) [DONE]

**Goal:** When `Event::Patch` arrives, show diff modal; allow apply/reject.
**Acceptance tests:**

* Model proposes patch → diff modal appears → apply modifies file.
  **Owner:** TUI/Tools

### ISSUE 0704 — Tool call log + approvals UX (ucode-tui) [DONE]

**Goal:** Show tool calls in UI and require approvals for `run_cmd` (and optionally patch).
**Acceptance tests:**

* Tool call displayed; run_cmd prompts approval; deny blocks execution.
  **Owner:** TUI

### ISSUE 0705 — /connect UI (providers + auth method picker) (ucode-tui + auth)

**Goal:** In-TUI provider connect flow for API keys + login + subscription login.
**Acceptance tests:**

* `/connect` → choose provider → complete method → status updates.
  **Owner:** TUI/Auth

### ISSUE 0706 — Sidebar-first visual system and safety state UX (ucode-tui) [DONE]

**Goal:** Deliver polished, distinctive TUI UX with persistent operational awareness via sidebar.
**Scope/Notes:**

* Visual token system: color roles, spacing, border emphasis, semantic states (safe/warning/danger) [DONE]
* Sidebar priority panels: [DONE]
  * active provider/model + effective sandbox tier
  * active skill
  * context pack summary
  * tool call queue with approval state
  * subagent status panel (running/completed/failed)
  * network state indicator
* Transparent-friendly theme profile (terminal-dependent; no hard dependency) [DONE]
* Compact/comfortable density presets [DONE]
* Keyboard-first focus behavior across all panes
  **Acceptance tests:**
* Critical safety state (sandbox tier, approvals, network) remains visible during normal chat flow. [DONE]
* Sidebar remains usable at common terminal dimensions (80x24 minimum). [DONE]
* Theme toggle preserves contrast/accessibility in supported terminals. [DONE]
  **Owner:** TUI

### ISSUE 0707 — Slash command UX + registry integration (ucode-tui + ucode-core) [DONE]

**Goal:** Make `/command` first-class in the TUI with discoverability and safe execution.
**Scope/Notes:**

* Execute slash commands from input line and command palette [DONE]
* Show command source badge: user/project/plugin [DONE]
* Inline argument hints and validation messages [DONE]
* Command execution routed through same sandbox/approval pipeline [DONE]
  **Acceptance tests:**
* User-defined `/command` resolves and executes from TUI input. [DONE]
* Unknown command returns ranked suggestions. [DONE]
* Command execution enforces normal policy gates. [DONE]
  **Owner:** TUI/Core

**Completed:** 298 tests, 4 phases (0707a-0707d). CommandRegistry with register/resolve/search/suggest, slash autocomplete on `/` prefix, command execution via directive parser, source badges and argument hints in autocomplete dropdown and palette.

### ISSUE 0708 — Toast notification system + plugin UI extension API (ucode-tui + ucode-plugins) [DONE]

**Goal:** Implement the toast notification system and the versioned plugin UI extension API surface.

> See `docs/plans/2026-03-05-tui-design.md` §7 (Notifications) and §11 (Plugin UI Extension API).

**Scope/Notes:**

* Toast component: stacked top-right, 4 types (info/success/warning/error), auto-dismiss timers (info/success 4s, warning 8s, error persistent), manual dismiss with `q`/`Esc`
* Max 3 toasts visible simultaneously; older ones slide off
* System-triggered toasts: checkpoint created, budget soft threshold, agent completed/failed, MCP server crash, auth token expired
* Plugin UI extension API (10 calls, Safe/Guarded/Risky override classes):
  * Safe: `ui::toast`, `ui::notify`, `ui::sidebar_section`, `ui::status_segment`, `ui::palette_command`, `ui::badge`
  * Guarded: `ui::transcript_event`, `ui::modal`, `ui::confirm`, `ui::input_prompt`
* Plugin sidebar sections rendered in standard collapsible style with `[plugin]` badge
* Plugin UI lifecycle: register on `on_session_start`, cleanup on `on_session_end`
* Override class enforcement: Safe calls auto-applied, Guarded require plugin capability declaration, Risky blocked

**Acceptance tests:**

* System toast fires on checkpoint creation, budget warning, agent completion/failure.
* Plugin calls `ui::toast()` and toast appears with correct level styling and auto-dismiss.
* Plugin registers sidebar section; it appears after built-in sections with `[plugin]` badge.
* Plugin registers palette command; it appears in palette with `[plugin]` badge.
* Guarded calls are blocked if plugin has not declared guarded capability in manifest.
* Plugin UI elements are fully cleaned up on session end.
  **Owner:** TUI/Plugins

**Completed:** 332 TUI tests + 56 plugin tests, 4 phases (0708a-0708d). Toast notification system with auto-dismiss and rendering, system-triggered toast events (checkpoint/budget/agent/MCP/auth), plugin UI extension API types with Safe/Guarded/Risky enforcement, plugin sidebar sections with [plugin] badge, plugin palette command registration, and plugin UI lifecycle (session_start/session_end cleanup).

### ISSUE 0709 — Copy mode + search overlay + keybind overlay (ucode-tui) [DONE]

**Goal:** Implement transcript copy mode, full-text search, and keybind reference overlay.

> See `docs/plans/2026-03-05-tui-design.md` §6.10–6.11.

**Scope/Notes:**

* Copy mode: `v` enters vim-like selection in transcript; `y` yanks to clipboard; `Esc` exits
* Search overlay: `Ctrl+F` opens; regex or literal search; matches highlighted in transcript; `n`/`N` navigate; `Esc` closes
* Keybind overlay: `?` opens full keybind reference grouped by category; `Esc` closes

**Acceptance tests:**

* `v` enters copy mode; `y` copies selected transcript text to system clipboard.
* `Ctrl+F` opens search; matches are highlighted; `n`/`N` cycle through matches.
* `?` shows full keybind reference overlay with all keybinds grouped by category.
  **Owner:** TUI

**Completed:** 446 TUI tests, 3 phases (0709a-0709c). Keybind reference overlay with grouped bindings and preset-aware title. Search overlay with preset-aware key handling (vim: Enter closes, emacs: Ctrl+S/Ctrl+R/Ctrl+G, vscode: Enter=next match). Copy mode with anchor/cursor selection, j/k navigation, y to yank to clipboard via OSC 52/external/file fallback chain.

### ISSUE 0710 — Markdown rendering in transcript (ucode-tui) [DONE]

**Goal:** Render assistant messages with rich markdown formatting (bold, italic, code blocks, headers, tables, lists, links).

**Scope/Notes:**

* Uses `pulldown-cmark` (v0.13.1, minimal features) for parsing
* Event-driven state machine (`RenderCtx`) processes pulldown-cmark events into styled ratatui `Line`/`Span` vectors
* Inline styles: Bold (`BOLD`), Italic (`ITALIC`), Strikethrough (`CROSSED_OUT`), inline code (accent fg + surface bg)
* Code blocks: language label in dim, content with surface background, no word-wrap
* Headers: H1 (accent+bold+underline), H2 (accent+bold), H3-H6 (accent)
* Lists: bullet (`- `) and numbered (`1. `) with nesting support
* Tables: measured column widths, bold header row, muted pipe separators
* Links: accent+underline text, dim URL in parentheses
* Graceful fallback to plain text wrapping

**Acceptance tests:**

* Assistant messages render markdown formatting correctly.
* Streaming messages render markdown incrementally.
* `entry_height` uses `markdown_height` for correct virtual scrolling.
* Plain text without markdown renders identically to before.
  **Owner:** TUI

**Completed:** 493 TUI tests (47 new markdown tests). Created `components/markdown.rs` (1,544 lines) with `render_markdown` and `markdown_height` public API. Integrated into `render_assistant_message`, `render_streaming_message`, and `entry_height`. Removed dead `render_indented_text` helper. Updated demo with markdown-rich responses (headers, tables, code blocks, inline styles, links).

### ISSUE 0711 — Tmux / terminal multiplexer integration (ucode-tui) [DONE]

**Goal:** Terminal integration for multiplexer detection, color support, mouse toggle, and terminal title.

**Scope/Notes:**

* Multiplexer detection: `$TMUX`, `$ZELLIJ`, `$STY` → `[tmux]`/`[zellij]`/`[screen]` in title bar
* OSC 52 clipboard writes with fallback chain (external tool → file)
* Color support detection: `$COLORTERM` (truecolor/24bit) → `$TERM` (256color) → tmux → Basic
* Mouse capture toggle via `app.mouse_enabled` (defaults to true)
* Terminal title set to "ucode" on startup via OSC 0, restored on exit
* SIGWINCH resize handled natively by crossterm

**Acceptance tests:**

* `[tmux]` shows in title bar when `$TMUX` is set.
* Copy in copy mode writes to clipboard via OSC 52 inside tmux.
* Fallback clipboard works when OSC 52 is unavailable.
* `mouse_enabled = false` disables mouse capture for tmux mouse passthrough.
  **Owner:** TUI

**Completed:** 502 TUI tests (7 new). Created `terminal.rs` with `set_terminal_title`, `restore_terminal_title`, `ColorSupport` enum, and `detect_color_support`. Added `color_support` and `mouse_enabled` fields to `AppState`. Terminal title set on startup and restored in `TerminalGuard::drop`. Mouse capture conditional on `app.mouse_enabled`.

### ISSUE 0712 — Keybinding presets (ucode-tui) [DONE]

**Goal:** Three built-in keybinding presets (vscode, vim, emacs) with individual override support.

**Scope/Notes:**

* Three presets: vscode (default), vim (modal with Normal/Insert), emacs (Meta+x)
* `KeybindResolver` resolves key events to `Action`s based on active preset
* Vim modal editing: `InputMode::Normal`/`Insert`, mode-aware action suppression
* `override_binding()` and `remove_binding()` for individual customization
* Config file integration deferred to config system build-out

**Acceptance tests:**

* `preset = "vim"` activates modal editing with `i`/`Esc` mode switching.
* `preset = "emacs"` activates emacs-style navigation and `Meta+x` palette.
* `override_binding()` / `remove_binding()` allow individual overrides on top of any preset.
* Default preset is `vscode` when no config is set.
  **Owner:** TUI

**Completed:** 506 TUI tests (4 new override tests). `KeybindResolver` with 3 presets, vim modal editing, and `override_binding`/`remove_binding` methods. Preset-aware search overlay and keybind overlay from ISSUE 0709.

---

## EPIC 8 — Plugins + hooks (user customization)

### ISSUE 0801 — Plugin manifest + loader (ucode-plugins) [DONE]

**Goal:** Define plugin manifests/registry and lifecycle contracts without introducing runtime complexity early.
**Scope/Notes:**

* `PluginManifest`: name, version, description, author, min_api_version, hooks, tools, capabilities
* `PluginToolDef`: name, description, input_schema for plugin-exported tools
* `PluginCapabilities`: filesystem, network, process_spawn flags
* `parse_manifest()`/`parse_manifest_file()`: TOML parsing with validation
* `validate_manifest()`: checks non-empty name/version/tool-names/hook-names
* `discover_plugins()`: scans directories for `plugin.toml` in subdirectories
* `PluginRegistry`: register/list/find/activate/deactivate/mark_failed
* `PluginStatus`: Discovered/Active/Inactive/Failed
* 23 tests (12 manifest + 11 loader)
  **Acceptance tests:**
* Example plugin manifest is discovered and validated.
* Capability declarations load into policy engine.
  **Owner:** Plugins

### ISSUE 0802 — Hooks API v1 (ucode-plugins + core/tools) [DONE]

**Goal:** Define hook events and dispatch them.

**Full hook event surface (64 events, 16 categories).** Initial implementation delivered 22 events; remaining 42 events added in ISSUE 0803.

Session lifecycle: `session_start`, `session_end`, `session_title_generated`, `session_title_updated`, `config_reloaded`
Message flow: `user_message_received`, `assistant_response_started`, `assistant_response_completed`, `message_retry` (Guarded)
Model selection: `before_model_call` (Guarded), `after_model_call`, `before_model_select` (Guarded), `model_fallback` (Risky), `router_decision`, `model_rate_limited`, `model_quota_exhausted`
Tool calls (generic): `before_tool_call` (Guarded), `after_tool_call`, `tool_error`, `tool_timeout`
Tool calls (specific): `before_apply_patch` (Guarded), `after_apply_patch`, `before_run_cmd` (Guarded), `after_run_cmd`, `before_file_read` (Guarded), `after_file_read`, `before_file_write` (Guarded), `after_file_write`
Context: `context_overflow` (Guarded), `context_compaction` (Guarded), `context_distilled`, `token_usage_updated`
Agent: `agent_spawned`, `agent_message`, `agent_completed`, `agent_failed`, `agent_cancelled`
Approval: `approval_required` (Guarded), `approval_granted`, `approval_denied`, `sandbox_decision`, `permission_decision`
Auth: `auth_changed`, `auth_failed`, `provider_switched`
MCP: `mcp_server_connected`, `mcp_server_disconnected`, `mcp_server_launch`, `mcp_server_restart`, `mcp_server_crash`, `mcp_tool_invoked`
Skills: `skill_activated`, `skill_deactivated`
Plugins: `plugin_loaded`, `plugin_unloaded`, `plugin_error`
Checkpoints: `checkpoint_created` (Guarded), `checkpoint_restored` (Risky)
Budget: `budget_threshold_warning`, `budget_threshold_reached` (Guarded), `cost_incurred`
Background: `background_job_state_changed`
Commands/UI: `command_invoked`, `palette_command_executed`
Diagnostics: `unhandled_error`

Override classes matrix enforced by host policy:

| Class | Examples | Auto-apply | Requires explicit approval |
| --- | --- | --- | --- |
| Safe | fallback ranking hints, logging verbosity, context-shrink preference | Yes (policy-safe) | No |
| Guarded | model group preference changes within limits, retry tuning in bounds | Yes (bounded) | No |
| Risky | widening network/filesystem/process permissions, bypassing deny rules | No | Yes |

**Acceptance tests:**
* Example plugin logs router/fallback decisions and can propose safe fallback preferences.
* Unsafe behavior override attempt is blocked unless explicitly user-approved.
* Risky class override is never auto-applied.

**Completed:** Initial 22 events implemented with `HookEvent` enum, `HookDispatcher`, `HookRecord`, `HookSubscription`, `OverrideClass`. 12 tests.

**Owner:** Plugins/Core

### ISSUE 0803 — Plugin API contract, tool registration, and Rust SDK (ucode-plugins) -- DONE

**Status:** DONE. 95 plugin tests (52 new), 506 TUI tests, 0 clippy warnings.

**Goal:** Define the v1 plugin API as Rust traits, expand hook surface to 64 events, implement tool registration with namespacing, and provide an in-process example plugin. Traits-first approach; WASM/WIT deferred to ISSUE 0805.

**Scope/Notes:**

Manifest changes:
* `id` field: reverse-domain globally unique identifier (e.g., `org.acme.code-analyzer`), minimum 3 dot-separated segments, each segment `[a-z0-9-]`
* `name` field: human-readable display name (arbitrary string, e.g., "Code Analyzer")
* `required_features`: unversioned feature set (`["hooks", "tools", "ui"]`)
* Tool names are local in manifest; host constructs FQN as `{plugin_id}.{tool_name}`

Handshake protocol:
* Plugin sends `HandshakeRequest { plugin_id, min_api_version, required_features, capabilities }`
* Host responds `HandshakeResponse::Accepted { api_version, supported_features, granted_capabilities }` or `Rejected { reason }`
* Semver check: host API version >= plugin min_api_version (same major)
* Feature check: plugin required_features is subset of host supported_features
* Capability check: host may grant subset of requested capabilities

Plugin traits (v1):
* `Plugin` (mandatory): `handshake()`, `initialize()`, `shutdown()`
* `HookHandler` (opt-in, `hooks` feature): `on_event(&HookEvent) -> HookResponse`
* `ToolProvider` (opt-in, `tools` feature): `tool_specs()`, `invoke_tool()`
* `HookResponse`: `Ok` / `Modify { changes }` (Guarded only) / `Veto { reason }` (Risky only)

Hook event expansion:
* Expand `HookEvent` from 22 to 64 variants (full surface from ISSUE 0802 spec)

Tool registration:
* Plugin declares `ToolSpec` via `ToolProvider::tool_specs()`
* Host namespaces as `{plugin_id}.{tool_name}` (e.g., `org.acme.code-analyzer.lint`)
* Host validates JSON schema and capability policy before activation
* Plugin tools flow through normal sandbox/approval/audit pipeline

Version negotiation:
* `semver` crate for parsing and comparison
* `Feature` enum: `Hooks`, `Tools`, `Ui`
* Host reports `API_VERSION` constant and supported features

**Acceptance tests:**
* Rust in-process example plugin passes handshake and receives hooks.
* Version mismatch produces `HandshakeError::VersionIncompatible`.
* Feature mismatch produces `HandshakeError::UnsupportedFeatures`.
* Plugin tool registered with namespaced FQN and invocable through host registry.
* All 64 hook events have `event_name()` and `override_class()`.

**Owner:** Plugins/Tools

### ISSUE 0804 — WIT interface + wasmtime WASM runtime (ucode-plugins) [DONE]

**Status:** DONE. 115 plugin tests (111 unit + 4 integration), 0 clippy warnings.

**Goal:** Translate the Rust trait API (from ISSUE 0803) into WIT/component-model interfaces and integrate wasmtime for WASM plugin execution.
**Scope/Notes:**

* 65 WIT hook interfaces across 20 versioned category packages + shared types + lifecycle/tool-provider
* wasmtime 42 host runtime with component-model support and dynamic export probing
* `wasmtime::component::bindgen!` for host-side type generation; `wit_bindgen::generate!` for guest-side bindings
* WASM plugin lifecycle: load `.wasm` -> probe exports -> build dispatch table -> dispatch hooks
* `ucode-plugin-sdk` guest SDK crate (minimal-plugin world, compiles to `wasm32-wasip2`)
* `hello-wasm` example plugin demonstrating lifecycle + session/on-start hook
* Host-log import wired for plugin-to-host logging
* `WasmPlugin` integrated into `PluginHost` via `Wasm` variant with `load_wasm()` method

Implementation plan: `docs/plans/2026-03-06-wasm-runtime.md` (10 tasks)
Design doc: `docs/plans/2026-03-06-wasm-runtime-design.md`

  **Acceptance tests:**
* Rust WASM plugin compiled to `.wasm` loads via wasmtime and passes handshake.
* WASM plugin receives hook events and returns `HookResponse`.
* WASM plugin exports tools accessible through host registry with namespaced FQN.
* Version mismatch between WIT contract versions produces clear error.
  **Owner:** Plugins

### ISSUE 0805 — WASM plugin runtime isolation model (latest stage) (ucode-plugins + security) -- DONE

**Goal:** Implement WASM-only plugin runtime in the latest stage, with strict safety controls.
**Scope/Notes:**

* WASM component runtime with policy-gated capabilities
* Per-plugin policy profile: filesystem scope, network, command spawn, hook scope
* Plugin-originated actions must pass normal approval/sandbox/audit pipeline
* Runtime is scheduled for latest stage to avoid early complexity
* Ed25519 signed plugin verification (feature-gated: `signed-plugins`)
* WASM resource limits (fuel metering + memory caps via StoreLimits)
* Plugin isolation levels (Full / Ordered) with accumulated_changes tracking
* Dynamic policy hot-reload from TOML config
* WASI preopens for defense-in-depth filesystem sandboxing
* Tracing instrumentation across all policy enforcement paths
  **Tests:** 145 (no features) / 178 (wasm) / 180 (wasm + signed-plugins), 0 clippy warnings
  **Acceptance tests:**
* Untrusted plugin cannot exceed granted permissions.
* Plugin-originated action triggers normal approval/sandbox checks.
* Runtime model and effective plugin permissions are visible in logs/UI.
  **Owner:** Plugins/Security

### ISSUE 0806 — External plugin infrastructure + public hook surface (ucode-plugins + core) [P0]

**Goal:** Complete the plugin runtime plumbing so external plugins (DCP-style context managers, custom tools, etc.) can load, receive hooks, transform messages, and register tools — all within the existing capability/policy model.
**Scope/Notes:**

* **Plugin discovery paths:** Project-local (`.ucode/plugins/`) > user-level (`~/.ucode/plugins/`) > config-driven extras. First match wins on plugin ID conflict. Existing `discover_plugins()` just needs default paths wired in.
* **Unified WIT interface (replaces 65 typed interfaces):** Remove the 65 per-hook WIT packages. New single `ucode:plugin@1.0.0` with three interfaces:
  * `hook-handler`: `handle(event: hook-event) -> hook-response` — event is `{ name, payload-version, payload(JSON) }`
  * `tool-handler`: `handle-tool-call(name, args) -> result<string, string>`
  * `transform-handler`: `transform-messages(json) -> json`, `transform-system-prompt(text) -> text`
  * Rationale: typed WIT per hook makes versioning hard; JSON payload allows additive changes without breaking plugins
* **Complete WASM hook dispatch:** Currently stubbed at `host.rs:294-302`. Wire actual calls via unified `hook-handler.handle()`: serialize HookRecord → JSON, call WASM export, deserialize response, accumulate Modify changes. Fail-open on error/fuel exhaustion.
* **Message transform hooks (new, separate from dispatch_hook):**
  * `transform-messages` — plugin receives full message array (JSON), returns modified array
  * `transform-system-prompt` — plugin receives system prompt, returns modified text
  * New `PluginHost` methods: `transform_messages()`, `transform_system_prompt()`
  * **User controls ordering** via `ucode.toml` (plugins do NOT declare priority):
    ```toml
    [context_management.transform_pipeline]
    order = ["org.acme.custom-dedup", "native", "org.acme.extra-pruner"]
    ```
  * Default: `["native"]`. Omitting `"native"` disables native context management.
  * Composable (output of one feeds next), `Guarded` safety tier, latency-sensitive
* **Plugin tool registration:** Tools declared in `plugin.toml`, routed via `tool-handler` WIT interface. Same sandbox/approval policy as built-in tools. Namespaced: `{plugin_id}.{tool_name}`.
* **Hook payload versioning:** Two layers — plugin API version (`min_api_version`) for WIT shape, payload version (per-hook semver in JSON `payload-version` field) for schema changes. Host skips dispatch on version mismatch.
* **Hook payload documentation:** `docs/hooks/` with schema, safety tier, version history per hook category
* **Fixture plugin:** `examples/plugins/context-manager/` — implements all three WIT interfaces, demonstrates full lifecycle: discovery → load → dispatch → tool call → transform → response
  **Acceptance tests:**
* Fixture plugin loads from user/project plugin paths.
* WASM dispatch calls handlers via unified `hook-handler.handle()` (not stubbed).
* Message transforms modify messages before LLM call via `transform-handler`.
* Plugin tools route via `tool-handler`, same approval/sandbox as built-ins.
* Hook payloads include `payload-version`; mismatch handled gracefully.
* User controls transform pipeline ordering via `ucode.toml`.
* Old 65 typed WIT packages removed; unified `ucode:plugin@1.0.0` is the only interface.
* Permission escalation blocked and auditable.
  **Owner:** Plugins/Core/Security

### ISSUE 0808 — Native context management system (ucode-context crate) [P0]

**Goal:** Native context management as a core crate combining strategies from opencode-dcp, rlm-skill, and context-mode. Runs directly in the LLM call path — not through the plugin system. Per-model configurable so users can tune strategies per model (e.g., disable LLM pruning for Opus, enable for Sonnet).
**Scope/Notes:**

* **Native, not plugin:** Automatic strategies are pure Rust message rewrites in the call path. Knowledge base and session continuity are core infrastructure. No WASM boundary overhead. External plugins (ISSUE 0806) can still add custom strategies.
* **Per-model configuration** via `ucode.toml`:
  * Global toggle + per-strategy toggles (dedup, supersede, purge, sandbox, pruning)
  * `[context_management.pruning.overrides."model-name"]` for per-model LLM pruning control
  * Disable LLM pruning for expensive models (Opus), enable for cheaper ones (Sonnet/Haiku)
  * Option to delegate pruning to a cheaper model (e.g., Haiku summarizes while Opus works)
* **Automatic zero-cost strategies** (from opencode-dcp):
  * Deduplication — remove duplicate file reads within session
  * Supersede-writes — remove earlier writes when file was subsequently read
  * Purge-errors — remove errored tool inputs after N turns (default 3)
  * All run as native message transform pass, zero LLM cost
* **Sandbox execution** (from rlm-skill / context-mode):
  * Intercept large tool outputs (>2000 chars configurable)
  * Store full content in knowledge base, replace in context with metadata summary
  * LLM retrieves via knowledge base search tool
* **Smart LLM-driven pruning** (from opencode-dcp, improved):
  * `context_distill`, `context_compress`, `context_prune` as native built-in tools
  * Smart triggering: only inject pruning instructions when context > threshold (default 60%)
  * Per-model control: disable for Opus (rely on automatic strategies), enable for Sonnet
  * Cheaper-model delegation: route pruning calls to Haiku while session runs on Opus
* **FTS5 knowledge base** (from rlm-skill / context-mode):
  * SQLite FTS5 with Porter stemming + trigram matching
  * `knowledge_search` and `knowledge_store` tools for LLM
  * Per-session database in session directory
* **Session continuity** (from context-mode):
  * Capture significant events; create compaction snapshots
  * Restore context on session resume
  * Sessions survive multiple compaction cycles
  **Acceptance tests:**
* Dedup/supersede/purge measurably reduce message array on repeated file ops.
* Large tool outputs sandboxed and retrievable via knowledge base.
* LLM pruning configurable per model; disabled for Opus by default.
* Pruning delegates to cheaper model when configured.
* Knowledge base search returns relevant results with fuzzy matching.
* Session survives compaction and resumes with prior state.
* All strategies respect `ucode.toml` config and per-model overrides.
  **Owner:** Core/Context

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
**Scope/Notes:**

* Canonical runtime configuration format: TOML (`~/.config/ucode/ucode.toml`)
* Config root env override: `UCODE_HOME` (default `${XDG_CONFIG_HOME:-~/.config}/ucode`)
* Canonical config path: `${UCODE_HOME}/ucode.toml`
* YAML/JSON are out of scope; configuration is TOML-only
* Document precedence: defaults < global config < project config < session overrides
* Include per-tool configuration model (timeouts, output caps, approval mode, allowlists/denylists) with safe bounds
* Publish config schema versioning/migration notes for backward compatibility
**Acceptance tests:**

* `docs/config.md` exists + example config.
* Example includes per-tool overrides and precedence behavior.
* Docs include `UCODE_HOME` example for integration testing.
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

### ISSUE 0905 — Non-interactive headless/CI execution mode (ucode-cli + ucode-core) [DONE]

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

**Milestone M1 (MVP CLI):** 0001, 0101–0105, 0108–0111, 0201–0202, 0301–0302 (one provider), 0401, 0403–0407b, 0408–0409, 0107
**Milestone M2 (MVP TUI):** 0701–0704, 0706, 0707
**Milestone M3 (Compatibility + MCP):** 0601–0603, 0501–0506, 0305
**Milestone M4 (Plugins contracts + auth upgrades + subagents):** 0801–0803, 0203–0205, 0705, 0106, 0410–0413
**Milestone M5 (Polish + security + WASM runtime):** 0901–0905, 0804–0808

---
