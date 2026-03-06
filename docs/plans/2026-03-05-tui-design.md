# ucode TUI — UI Design & Component Specification

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Establish the complete visual design, component inventory, and plugin UI extension API for ucode's terminal user interface as the canonical reference before implementation begins.

**Architecture:** The TUI is a fullscreen terminal application built on Ratatui with a sidebar-first information design emphasizing system state over chat aesthetics. The layout comprises a transcript pane (center), resizable sidebar (right, with 9 collapsible sections), title bar, input box, status bar, and a toast notification system. All plugin UI extensions are rendered by the host in standard style; plugins provide only data.

**Tech Stack:** Ratatui 0.29 + Crossterm for terminal control; Tokio async runtime; configuration via TOML; plugin UI API exposed as a versioned Rust trait with Safe/Guarded/Risky override classes.

---

## PLANS.md / EPIC.md Cross-Reference

The TUI work lives in **Phase 7** of PLANS.md and **EPIC 7** of EPIC.md. The specific tasks/issues are:

- PLANS.md Task 7.1 — TUI foundation (panes, keybinds)
- PLANS.md Task 7.2 — Approvals UX (diff modal, run_cmd modal)
- PLANS.md Task 7.3 — Visual system + sidebar-first information design
- PLANS.md Task 7.4 — Slash command UX and discoverability
- EPIC.md ISSUE 0701 — Ratatui fullscreen shell + panes
- EPIC.md ISSUE 0702 — Command palette + keybinds
- EPIC.md ISSUE 0703 — Diff viewer + apply/reject UX
- EPIC.md ISSUE 0704 — Tool call log + approvals UX
- EPIC.md ISSUE 0705 — /connect UI (providers + auth method picker)
- EPIC.md ISSUE 0706 — Sidebar-first visual system and safety state UX
- EPIC.md ISSUE 0707 — Slash command UX + registry integration

The TUI also surfaces state from many other phases:
- Phase 1 (router, session, token budget, subagents) → ROUTER, CONTEXT, AGENTS sidebar sections
- Phase 4 (tools, sandbox, checkpoints, jobs) → TOOLS, WORKSPACE, JOBS sidebar sections
- Phase 5 (MCP) → MCP SERVERS sidebar section
- Phase 6 (skills) → SKILL sidebar section
- Phase 8 (plugins) → Plugin UI extension API

---

## Identity & Design Principles

ucode's identity: **a runtime orchestrator that happens to have a chat interface** — not the other way around. The UI should feel like a system monitor (htop/tmux aesthetic) that also has a conversation pane, not a chat app with some runtime info bolted on.

Key differentiators to express visually:
1. **ROUTER** section (not PROVIDER) — shows model group `[fast]`/`[strong]`/`[longctx]`, fallback chain, sandbox tier
2. **WORKSPACE** section — diff tracking `+/-` per file + checkpoint indicator `⎘`
3. **Router fallback events inline** in transcript — `↪ router: rate-limit on anthropic → openai [strong]`
4. **Session lineage in title** — `ucode  main ⎇ fork-1`
5. **Teal accent** `#00d4aa` — not purple (Claude), not green (OpenCode), not rainbow (Crush)
6. **Status bar as system state line** — runtime info, not a keybind list
7. **Collapsible sidebar sections** — `▼` expanded, `▶` collapsed with inline summary

---

## Section 1: Layout Architecture

Describe the overall terminal layout. Include ASCII art sketches for:

### 1.1 Full layout (normal state, ~220×50)

```
┌──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┬──────────────────────────────────────────────────────────────────┐
│ ucode  main ⎇ fork-1                                                                              abc123  2026-03-05 │ ▼ ROUTER                                                         │
├──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┤   model:  claude-3-5-sonnet  [strong]                            │
│                                                                                                                      │   chain:  anthropic → openai → ollama                            │
│   You: can you refactor the auth module?                                                                             │   tier:   ●ws workspace                                          │
│                                                                                                                      ├──────────────────────────────────────────────────────────────────┤
│   ─────────────────────────────────────────────────────────────────────────────────────────────────────────────────  │ ▼ SKILL                                                          │
│                                                                                                                      │   ml-paper-writing  ·  4 tools allowed                           │
│   Assistant:                                                                                                         ├──────────────────────────────────────────────────────────────────┤
│   I'll refactor the auth module. Reading current implementation.                                                     │ ▼ CONTEXT                                                        │
│                                                                                                                      │   12,847 / 200k                                                  │
│   ┌ tool: read_file ─────────────────────────────────────────────────────────────────────────────────────────────    │   ████░░░░░░░░░░░░░░░░░░░░░░░░░  6%                              │
│   │ crates/ucode-auth/src/lib.rs  ✓  2.1kb  0.1s                                                                    │   $0.04 req  ·  $0.12 session                                    │
│   └──────────────────────────────────────────────────────────────────────────────────────────────────────────────    ├──────────────────────────────────────────────────────────────────┤
│                                                                                                                      │ ▼ WORKSPACE                                                      │
│   ↪ router: rate-limit on anthropic → openai/gpt-4o  [strong]  0.8s                                                 │   ucode-auth/src/lib.rs      +24  -8                             │
│                                                                                                                      │   ucode-core/src/router.rs    +5  -2                             │
│   Here's my plan:                                                                                                    │   ──────────────────────────────                             │
│   1. Extract token refresh into TokenRefresher struct                                                                │   2 files  +29 -10                                               │
│   2. Replace inline refresh in AuthClient::request()                                                                 │   ⎘ checkpoint  2m ago  [z rollback]                             │
│   3. Add retry-on-401 with backoff                                                                                   ├──────────────────────────────────────────────────────────────────┤
│                                                                                                                      │ ▼ TOOLS                                                          │
│   ┌ tool: apply_patch ──────────────────────────────────────────────────────────────────────────────────────────     │   read_file    ✓  0.1s                                           │
│   │ crates/ucode-auth/src/lib.rs                                                                                     │   apply_patch  ⚠  pending approval                               │
│   │ ⚠  requires approval                                                                                            ├──────────────────────────────────────────────────────────────────┤
│   │ [a] apply  [r] reject  [d] diff                                                                                  │ ▶ AGENTS                                    1✓ 1⟳ 1✗             │
│   └──────────────────────────────────────────────────────────────────────────────────────────────────────────────    ├──────────────────────────────────────────────────────────────────┤
│                                                                                                                      │ ▶ NETWORK                                   off                  │
│                                                                                                                      ├──────────────────────────────────────────────────────────────────┤
│                                                                                                                      │ ▶ JOBS                                      (2 running)           │
│                                                                                                                      ├──────────────────────────────────────────────────────────────────┤
│                                                                                                                      │ ▶ MCP SERVERS                               (3 active)            │
├──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┴──────────────────────────────────────────────────────────────────┤
│ > _                                                                                                                                                                                      │
├──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┤
│ ^P ^O ^E  │  INFO  │  main ⎇  │  ●ws  │  [strong] claude-3-5-sonnet  │  ⟳ agent-b  │  +29 -10  │  $0.12  │  12k/200k                                                                  │
└──────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 1.2 Minimum terminal size (80×24) — icon-strip sidebar

At 80 columns, sidebar collapses to a narrow icon strip on the right edge:

```
┌──────────────────────────────────────────────────────────────────────────┬──────┐
│ transcript...                                                            │ R    │
│                                                                          │ S    │
│                                                                          │ C    │
│                                                                          │ T  ⚠ │
│                                                                          │ A  ⟳ │
│                                                                          │ N    │
│                                                                          │ J  2 │
│                                                                          │ M  3 │
├──────────────────────────────────────────────────────────────────────────┴──────┤
│ > _                                                                              │
├──────────────────────────────────────────────────────────────────────────────────┤
│ ^P ^O ^E                                                INFO  ⟳  $0.12          │
└──────────────────────────────────────────────────────────────────────────────────┘
```

Icon key: R=Router, S=Skill, C=Context, T=Tools, A=Agents, N=Network, J=Jobs, M=MCP

### 1.3 Pane dimensions

- **Sidebar width**: 34 columns default, user-resizable (min 28, max 48) via `[` / `]` keys
- **Input box**: single line, expands to multiline on `Shift+Enter`, max 8 lines before scrolling
- **Status bar**: 1 line, always visible
- **Title bar**: 1 line, always visible

---

## Section 2: Visual Design System

### 2.1 Color palette (semantic roles)

```
Background:   #0d0d0d  (or transparent at configurable opacity)
Surface:      #141414  (panels, sidebar background)
Border:       #2a2a2a  (default borders)
BorderFocus:  #3a3a3a  (focused pane border)
Accent:       #00d4aa  (teal — primary actions, cursor, active state)
Safe:         #22c55e  (green — sandbox ok, tool success, agent done, ✓)
Warning:      #f59e0b  (amber — pending approval, soft budget, agent running, ⚠)
Danger:       #ef4444  (red — denied, hard budget, agent failed, error, ✗)
Muted:        #6b7280  (secondary text, labels, keybind hints)
Text:         #e5e7eb  (primary text)
TextDim:      #9ca3af  (dimmed text, timestamps, metadata)
```

### 2.2 Sandbox tier color mapping

| Tier | Symbol | Color |
|------|--------|-------|
| `off` | `●off` | Muted (#6b7280) |
| `workspace` | `●ws` | Safe (#22c55e) |
| `networked` | `●net` | Warning (#f59e0b) |
| `strict` | `●strict` | Accent (#00d4aa) |

### 2.3 Model group badges

| Group | Badge | Meaning |
|-------|-------|---------|
| fast | `[fast]` | Low-latency models |
| strong | `[strong]` | High-capability models |
| longctx | `[longctx]` | Large context window models |

Badges rendered in Accent color when active, Muted when fallback target.

### 2.4 Density presets

- **compact**: 0 padding between sidebar sections, 1-line tool call entries
- **comfortable**: 1 blank line between sections, 2-line tool call entries (default)

### 2.5 Transparency

Configurable via `ucode.toml`:
```toml
[tui.theme]
opacity = 90          # 0-100, requires compositor support (kitty, alacritty, etc.)
preset = "hybrid"     # "hybrid" | "dark" | "light"
density = "comfortable"
accent = "#00d4aa"    # overridable
```

Terminal-dependent: if terminal does not support transparency, falls back gracefully to solid background. No hard dependency.

### 2.6 Border style

Standard box-drawing characters throughout. Main transcript/sidebar split uses the same weight as section dividers — minimalist, no decorative double-lines. Section dividers inside sidebar use `─` (thin horizontal rule).

---

## Section 3: Sidebar Sections

All sections are collapsible. Toggle with `s` + number (e.g., `s1` for Router, `s2` for Skill, etc.) or mouse click on header. State persists across sessions in config.

**Section anatomy:**
```
├──────────────────────────────────────────────────────────────────┤
│ ▼ SECTION TITLE                         [summary when collapsed] │
│   content line 1                                                  │
│   content line 2                                                  │
└──────────────────────────────────────────────────────────────────┘
```
- `▼` = expanded, `▶` = collapsed
- Collapsed header shows one-line summary of most important state
- Section title in Muted color, content in Text color

### 3.1 ROUTER (replaces "PROVIDER") — ties to PLANS.md Task 1.2, EPIC ISSUE 0103

Expanded:
```
▼ ROUTER
  model:  claude-3-5-sonnet  [strong]
  chain:  anthropic → openai → ollama
  tier:   ●ws workspace
  last:   direct  0.3s ago
```

Collapsed:
```
▶ ROUTER    claude-3-5-sonnet [strong] ●ws
```

Fields:
- `model`: active model name + group badge (colored in Accent)
- `chain`: full fallback chain, current provider highlighted
- `tier`: sandbox tier with colored dot
- `last`: last routing decision (direct/fallback) + elapsed time

Ties to: PLANS.md Task 1.2 (router/fallback), Task 1.9 (token/cost governance), EPIC ISSUE 0103

### 3.2 SKILL — ties to PLANS.md Task 6.2, EPIC ISSUE 0603

Expanded:
```
▼ SKILL
  ml-paper-writing
  tools: 4 allowed
  routing: prefers [strong]
```

Collapsed:
```
▶ SKILL    ml-paper-writing  ·  4 tools
```

Ties to: PLANS.md Task 6.2, EPIC ISSUE 0603

### 3.3 CONTEXT — ties to PLANS.md Task 1.6, Task 1.9, EPIC ISSUE 0108, 0111

Expanded:
```
▼ CONTEXT
  12,847 / 200k
  ████░░░░░░░░░░░░░░░░░░░░░░░░░  6%
  $0.04 req  ·  $0.12 session
  count: provider_count
```

Collapsed:
```
▶ CONTEXT    12,847 / 200k  6%  $0.12
```

Progress bar color: green (0–70%), amber (70–90%), red (90–100%).

Fields:
- Token count: `used / max`
- Progress bar with percentage
- Cost: per-request and session total
- Count source: `provider_count` or `local_estimate` (from Task 1.6)

Ties to: PLANS.md Task 1.6 (token budget), Task 1.9 (cost governance), EPIC ISSUE 0108, 0111

### 3.4 WORKSPACE — NEW, ties to PLANS.md Task 4.9, EPIC ISSUE 0411

Expanded:
```
▼ WORKSPACE
  ucode-auth/src/lib.rs      +24  -8
  ucode-core/src/router.rs    +5  -2
  ──────────────────────────────────
  2 files  +29 -10
  ⎘ checkpoint  2m ago  [z rollback]
```

Collapsed:
```
▶ WORKSPACE    +29 -10  ⎘ 2m
```

Fields:
- Per-file diff summary: filename (truncated from right), `+N` in Safe color, `-N` in Danger color
- Total summary line
- Checkpoint indicator: `⎘` symbol + age + rollback keybind
- If no checkpoint: `⎘ none`
- If no changes: shows `clean`

Ties to: PLANS.md Task 4.9 (checkpoints), EPIC ISSUE 0411

### 3.5 TOOLS — ties to PLANS.md Task 7.2, EPIC ISSUE 0704

Expanded:
```
▼ TOOLS
  read_file    ✓  0.1s
  search       ✓  0.3s
  apply_patch  ⚠  pending approval
```

Collapsed (shows most urgent state):
```
▶ TOOLS    apply_patch ⚠ pending
```

Status icons:
- `✓` Safe color — completed successfully
- `⚠` Warning color — pending approval
- `⟳` Accent color — running
- `✗` Danger color — failed/denied

Ties to: PLANS.md Task 7.2, EPIC ISSUE 0704

### 3.6 AGENTS — ties to PLANS.md Task 1.3, EPIC ISSUE 0105

Expanded:
```
▼ AGENTS
  agent-a  ✓ done    1.2s
  agent-b  ⟳ running  4s
  agent-c  ✗ failed
```

Collapsed:
```
▶ AGENTS    1✓ 1⟳ 1✗
```

Shows parent-child indentation for DAG relationships:
```
  agent-a  ✓ done
  └─ agent-a-1  ✓ done
  agent-b  ⟳ running
```

Ties to: PLANS.md Task 1.3 (subagent orchestration), EPIC ISSUE 0105

### 3.7 NETWORK — ties to PLANS.md Task 4.8, EPIC ISSUE 0410

Expanded:
```
▼ NETWORK
  egress: off
  agent-b: anthropic.com ✓
```

Collapsed:
```
▶ NETWORK    off
```

When networked tier active, shows active connections per agent.

Ties to: PLANS.md Task 4.8, EPIC ISSUE 0410

### 3.8 JOBS — ties to PLANS.md Task 4.10, EPIC ISSUE 0412

Expanded:
```
▼ JOBS
  job-1  cargo test  ⟳ running  12s
  job-2  npm build   ✓ done     3s
```

Collapsed:
```
▶ JOBS    (2 running)
```

Actions: `k` to cancel focused job, `K` to force-kill.

Ties to: PLANS.md Task 4.10, EPIC ISSUE 0412

### 3.9 MCP SERVERS — ties to PLANS.md Task 5.4, EPIC ISSUE 0504

Expanded:
```
▼ MCP SERVERS
  filesystem  ✓ trusted   running
  github      ⚠ untrusted  stopped
  search      ✓ trusted   running
```

Collapsed:
```
▶ MCP SERVERS    (3 active)
```

Ties to: PLANS.md Task 5.4, EPIC ISSUE 0504

### 3.10 Plugin-injected sections

Plugins can register custom sidebar sections via `ui::sidebar_section(id, title, lines, priority?)`. These appear after built-in sections (or at plugin-specified priority). Rendered identically to built-in sections — same collapsible pattern, same style. Plugin sections have a small `[plugin]` badge in Muted color next to the title.

---

## Section 4: Transcript Content Components

All rendered inside the TranscriptPane (scrollable, keyboard-navigable).

### 4.1 UserMessage

```
  You: can you refactor the auth module?
```

Styled with Accent color on "You:" prefix.

### 4.2 AssistantMessage (streaming)

```
  Assistant:
  I'll refactor the auth module. Let me start by reading the current implementation.
```

Streaming cursor shown as `▌` at end of current token. Markdown rendered: bold, italic, inline code, code blocks with language tag and border.

### 4.3 ToolCallBlock (collapsible)

```
  ┌ tool: read_file ─────────────────────────────────────────────────────────────────
  │ crates/ucode-auth/src/lib.rs  ✓  2.1kb  0.1s
  └──────────────────────────────────────────────────────────────────────────────────
```

Expanded shows args and result. Collapsed (default after completion) shows one-line summary. Press `t` to toggle.

### 4.4 RouterEvent (inline audit line) — ucode-specific

```
  ↪ router: rate-limit on anthropic → openai/gpt-4o  [strong]  0.8s
```

Rendered in Warning color. Makes routing decisions auditable in the conversation itself. Ties to PLANS.md Task 1.2 router fallback events.

### 4.5 SystemEvent

```
  ─ session started  abc123  2026-03-05 14:32:01
  ─ skill activated: ml-paper-writing
  ─ checkpoint created: ⎘ cp-001  2m ago
```

Rendered in Muted color with `─` prefix.

### 4.6 ApprovalBlock (inline, ties to PLANS.md Task 7.2, EPIC ISSUE 0703/0704)

```
  ┌ tool: apply_patch ──────────────────────────────────────────────────────────────
  │ crates/ucode-auth/src/lib.rs
  │ ⚠  requires approval
  │ [a] apply  [r] reject  [d] view diff
  └──────────────────────────────────────────────────────────────────────────────────
```

Approval options: `once`, `session`, `deny`. Ties to PLANS.md Task 4.7 (confirmation gates).

### 4.7 DiffBlock (inline or modal)

Syntax-highlighted unified diff. Inline for small diffs (<20 lines), opens DiffModal for larger. `+` lines in Safe color, `-` lines in Danger color.

### 4.8 ErrorBlock

```
  ✗ error: provider anthropic returned 429 rate-limit
    fallback initiated → openai
```

Danger color for `✗ error:`, normal text for detail.

### 4.9 AgentEvent

```
  ⊕ agent-b spawned  (research profile, networked tier)
  ⊗ agent-b completed  4.2s  →  result: 3 files found
```

`⊕` in Accent, `⊗` in Safe/Danger depending on outcome.

### 4.10 PluginEvent

```
  [my-plugin] budget warning: 80% of session limit used
```

Plugin name in Muted, message in Text. Injected via `ui::transcript_event()`.

---

## Section 5: Input Area

### 5.1 TextInput

Single-line by default. `Shift+Enter` adds newline, expands box up to 8 lines. `Enter` submits. `Esc` clears.

### 5.2 SlashAutocomplete (ties to PLANS.md Task 7.4, EPIC ISSUE 0707)

Appears above input when `/` typed. Shows command name, description, source badge `[builtin]`/`[user]`/`[plugin]`. `↑↓` navigate, `Tab` complete, `Enter` execute, `Esc` dismiss.

```
  /session fork     Fork current session              [builtin]
  /session list     List all sessions                 [builtin]
  /skills           Browse and activate skills        [builtin]
  /my-command       My custom command                 [user]
```

### 5.3 MentionAutocomplete

Appears above input when `@` typed. Shows registered agent names.

### 5.4 InlineValidationError

Shown below input in Danger color when command is unknown or args are invalid.

---

## Section 6: Overlays / Modals

### 6.1 CommandPalette (Ctrl+P) — ties to PLANS.md Task 7.4, EPIC ISSUE 0702

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│                                                                                  │
│   ┌──────────────────────────────────────────────────────────────────────────┐   │
│   │ > /                                                                      │   │
│   └──────────────────────────────────────────────────────────────────────────┘   │
│                                                                                  │
│   ─── recent ──────────────────────────────────────────────────────────────────  │
│   /connect          Connect provider or auth method              [builtin]        │
│   /skills           Browse and activate skills                   [builtin]        │
│                                                                                  │
│   ─── session ─────────────────────────────────────────────────────────────────  │
│   /session list     List all sessions                            [builtin]        │
│   /session fork     Fork current session                         [builtin]        │
│   /session rename   Rename current session                       [builtin]        │
│                                                                                  │
│   ─── tools ───────────────────────────────────────────────────────────────────  │
│   /checkpoint       Create workspace checkpoint                  [builtin]        │
│   /rollback         Restore prior checkpoint                     [builtin]        │
│   /jobs             View background jobs                         [builtin]        │
│                                                                                  │
│   ─── plugins ─────────────────────────────────────────────────────────────────  │
│   /my-plugin-cmd    Plugin-registered command                    [plugin]         │
│                                                                                  │
│   esc close   ↑↓ navigate   enter execute   tab autocomplete                     │
└──────────────────────────────────────────────────────────────────────────────────┘
```

### 6.2 DiffModal (Ctrl+E or triggered by apply_patch) — ties to PLANS.md Task 7.2, EPIC ISSUE 0703

```
┌─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┐
│  apply_patch — crates/ucode-auth/src/lib.rs                                                              ⚠ approval │
├─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┤
│  @@ -42,8 +42,24 @@ impl AuthClient {                                                                                │
│  -    async fn refresh_token(&self) -> Result<Token> {                                                              │
│  +    async fn refresh_token(&self) -> Result<Token> {                                                              │
│  +        let refresher = TokenRefresher::new(self.http.clone());                                                   │
│  +        refresher.refresh_with_backoff(3).await                                                                   │
│  +    }                                                                                                             │
├─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┤
│  [a] apply    [r] reject    [e] edit before apply    esc cancel                                                     │
└─────────────────────────────────────────────────────────────────────────────────────────────────────────────────────┘
```

### 6.3 ApprovalModal (run_cmd, file/network consent) — ties to PLANS.md Task 4.7, EPIC ISSUE 0409

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│  run_cmd — approval required                                             ⚠        │
├──────────────────────────────────────────────────────────────────────────────────┤
│  command:  cargo test --workspace                                                │
│  cwd:      /home/user/code/ucode                                                 │
│  sandbox:  ●ws workspace                                                         │
├──────────────────────────────────────────────────────────────────────────────────┤
│  [o] approve once    [s] approve session    [d] deny                             │
└──────────────────────────────────────────────────────────────────────────────────┘
```

### 6.4 SessionSwitcher (Ctrl+S) — ties to PLANS.md Task 1.7, 1.8, EPIC ISSUE 0109, 0110

```
┌──────────────────────────────────────────────────────────────────────────────────┐
│  sessions                                                                        │
├──────────────────────────────────────────────────────────────────────────────────┤
│  ● main          Refactor auth module token refresh          2m ago   $0.12      │
│    ├─ fork-1     Explore alternative retry strategy          1h ago   $0.03      │
│    └─ fork-2     Test with mock provider                     3h ago   $0.01      │
│    debug-run     Debug MCP server crash on startup           1d ago   $0.44      │
│  [archived]      3 archived sessions                                             │
├──────────────────────────────────────────────────────────────────────────────────┤
│  n new   f fork   r rename   a archive   enter switch   esc close                │
└──────────────────────────────────────────────────────────────────────────────────┘
```

### 6.5 ProviderConnect (/connect) — ties to PLANS.md Task 2.1–2.5, EPIC ISSUE 0705

Multi-step modal: provider picker → auth method picker → credential entry/browser flow.

### 6.6 SkillBrowser (/skills) — ties to PLANS.md Task 6.1–6.2, EPIC ISSUE 0601–0603

List of discovered skills with name, description, source path. `Enter` to activate.

### 6.7 ModelPicker (/models) — ties to PLANS.md Task 1.2

Shows model groups (fast/strong/longctx) with available models per group. `Enter` to switch.

### 6.8 CheckpointBrowser (/checkpoint list) — ties to PLANS.md Task 4.9, EPIC ISSUE 0411

List of checkpoints with id, description, age, file count. `Enter` to rollback.

### 6.9 JobManager (/jobs) — ties to PLANS.md Task 4.10, EPIC ISSUE 0412

List of background jobs with state, elapsed time. `k` cancel, `K` force-kill, `Enter` view output.

### 6.10 KeybindOverlay (?)

Full keybind reference. All keybinds listed by category.

### 6.11 SearchOverlay (Ctrl+F)

Search transcript text. Highlights matches, `n`/`N` to navigate.

### 6.12 ConfirmDialog

Generic yes/no/cancel. Used by host and plugins (guarded).

### 6.13 InputPromptDialog

Single-line text input overlay. Used by host and plugins (guarded).

### 6.14 PluginModal

Custom overlay opened by plugin via `ui::modal()`. Renders title, content lines, action buttons in standard style. Plugin provides data, host renders.

---

## Section 7: Notifications / Feedback

### 7.1 Toast — plugin-triggerable (Safe override class)

Position: top-right corner, stacked vertically, newest on top. Max 3 visible simultaneously; older ones slide off.

```
                                    ┌──────────────────────────────────────────┐
                                    │ ✓  Checkpoint created                    │
                                    │    ⎘ cp-001 · z to rollback              │
                                    └──────────────────────────────────────────┘
                                    ┌──────────────────────────────────────────┐
                                    │ ⚠  Budget warning                        │
                                    │    80% of session budget used            │
                                    └──────────────────────────────────────────┘
                                    ┌──────────────────────────────────────────┐
                                    │ ●  agent-b completed                     │
                                    │    my-plugin · 4.2s                      │
                                    └──────────────────────────────────────────┘
```

Toast types and auto-dismiss:
| Type | Icon | Color | Auto-dismiss |
|------|------|-------|-------------|
| `info` | `●` | Accent (teal) | 4s |
| `success` | `✓` | Safe (green) | 4s |
| `warning` | `⚠` | Warning (amber) | 8s |
| `error` | `✗` | Danger (red) | persistent |

Manual dismiss: `q` or `Esc` on focused toast.

System-triggered toasts (automatic):
- Checkpoint created → success toast
- Budget soft threshold → warning toast (ties to PLANS.md Task 1.9, EPIC ISSUE 0111)
- Agent completed/failed → info/error toast (ties to PLANS.md Task 1.3, EPIC ISSUE 0105)
- MCP server crash → error toast (ties to PLANS.md Task 5.4, EPIC ISSUE 0504)
- Auth token expired → warning toast (ties to PLANS.md Task 2.5)

Plugin-triggered: `ui::toast(level, title, body?, duration_ms?)` — Safe override class, no approval needed.

### 7.2 ProgressBar

Used in CONTEXT section for token usage. Color shifts with percentage.

### 7.3 Spinner

`⟳` shown next to streaming assistant message, running tool calls, running agents.

### 7.4 Badge

Count/state indicator on collapsed sidebar section headers (e.g., `1✓ 1⟳ 1✗` on AGENTS).

### 7.5 InlineStatusDot

`●ws` `●net` `●strict` `●off` — colored sandbox tier indicator. Used in ROUTER section and status bar.

### 7.6 CopyModeOverlay

Activated with `v` (vim-like). Allows selection of transcript text for clipboard copy. `y` to yank, `Esc` to exit.

---

## Section 8: Status Bar

Single line at bottom. System state line — not a keybind list. Keybinds live in the palette.

```
^P ^O ^E  │  INFO  │  main ⎇  │  ●ws  │  [strong] claude-3-5-sonnet  │  ⟳ agent-b  │  +29 -10  │  $0.12  │  12k/200k
```

Segments (left to right):
1. Core keybind hints (minimal: `^P ^O ^E`)
2. Log level indicator (`INFO` / `DEBUG` / `TRACE`)
3. Session name + lineage (`main ⎇` or `main ⎇ fork-1`)
4. Sandbox tier dot (`●ws`)
5. Active model group + name (`[strong] claude-3-5-sonnet`)
6. Running agents (if any: `⟳ agent-b`)
7. Workspace diff summary (`+29 -10` or `clean`)
8. Session cost (`$0.12`)
9. Token usage (`12k/200k`)

Plugin segments inserted between items 6 and 7 via `ui::status_segment(id, content, priority?)`.

---

## Section 9: Title Bar

```
ucode  main ⎇ fork-1                                    abc123  2026-03-05
```

Left: `ucode` wordmark + session name + lineage indicator.
Right: session ID (short) + date.

`⎇` shown only when in a forked child session. On root session: just `main`.

Ties to: PLANS.md Task 1.7 (session titles), Task 1.8 (session lineage), EPIC ISSUE 0109, 0110.

---

## Section 10: Keybinds

### Global
| Key | Action |
|-----|--------|
| `Ctrl+P` | Open command palette |
| `Ctrl+O` | Open file picker |
| `Ctrl+E` | View last diff |
| `Ctrl+R` | Rerun last command |
| `Ctrl+T` | Run tests |
| `Ctrl+S` | Session switcher |
| `Ctrl+F` | Search transcript |
| `?` | Keybind overlay |
| `Tab` | Cycle focus: transcript → input → sidebar |

### Sidebar
| Key | Action |
|-----|--------|
| `s1`–`s9` | Toggle sidebar section 1–9 |
| `[` / `]` | Decrease/increase sidebar width |

### Transcript
| Key | Action |
|-----|--------|
| `↑↓` / `PgUp PgDn` | Scroll |
| `v` | Enter copy mode |
| `t` | Toggle focused tool call block |

### Approval (when ApprovalBlock focused)
| Key | Action |
|-----|--------|
| `a` | Apply/approve once |
| `s` | Approve for session |
| `r` / `d` | Reject/deny |

### Jobs (when JobManager open)
| Key | Action |
|-----|--------|
| `k` | Cancel job |
| `K` | Force-kill job |

### Workspace
| Key | Action |
|-----|--------|
| `z` | Rollback to last checkpoint (with confirm dialog) |

All keybinds overridable via `ucode.toml` `[tui.keybinds]` section.

### Keybinding presets

Three built-in presets. Default is `vscode` so mainstream users feel at home. Configurable via `ucode.toml`:

```toml
[tui.keybinds]
preset = "vscode"   # "vscode" | "vim" | "emacs"
```

Individual overrides layer on top of the active preset.

#### vscode preset (default)

The tables above describe the vscode preset. This is the baseline.

#### vim preset

| Key | Action | Difference from vscode |
|-----|--------|----------------------|
| `Esc` | Return to normal mode (transcript focus) | Input requires `i`/`a`/`o` to enter |
| `i` | Enter insert mode (focus input) | — |
| `j` / `k` | Scroll transcript down/up | Replaces arrow keys as primary |
| `gg` / `G` | Jump to top/bottom of transcript | — |
| `Ctrl+U` / `Ctrl+D` | Half-page up/down | — |
| `/` | Search transcript (same as Ctrl+F) | Vim-native search |
| `n` / `N` | Next/prev search match | Same as vscode |
| `:` | Open command palette | Replaces Ctrl+P |
| `v` | Enter visual/copy mode | Same as vscode |
| `y` | Yank selection | Same as vscode |
| `Ctrl+P` | Open command palette (alias) | Kept for muscle memory |

Normal mode is the default. `i` enters insert mode (input box focused). `Esc` returns to normal mode. Approval keys (`a`/`r`/`d`) work in normal mode when an approval block is focused.

#### emacs preset

| Key | Action | Difference from vscode |
|-----|--------|----------------------|
| `Meta+x` / `Alt+x` | Open command palette | Replaces Ctrl+P |
| `Ctrl+P` | Open command palette (alias) | Kept for muscle memory |
| `Ctrl+A` / `Ctrl+E` | Beginning/end of input line | Standard emacs line nav |
| `Ctrl+N` / `Ctrl+P` (in transcript) | Scroll down/up | Emacs-native nav |
| `Ctrl+V` / `Meta+V` | Page down/up | — |
| `Ctrl+S` | Search transcript | Replaces Ctrl+F |
| `Ctrl+R` | Reverse search transcript | — |
| `Ctrl+G` | Cancel/dismiss overlay | Replaces Esc in overlays |
| `Meta+W` | Copy selection | Replaces `y` |
| `Ctrl+Space` | Set mark (start selection) | Replaces `v` |

Input box always active (no modal insert/normal distinction). `Ctrl+G` is the universal cancel.

---

## Section 10b: Tmux Integration

ucode must work seamlessly inside tmux (and other terminal multiplexers like zellij, screen). Users should be able to scroll, select, and copy without fighting the TUI.

### Design principles

1. **Detect tmux**: check `$TMUX` env var at startup. Surface in status bar as `[tmux]` indicator when detected.
2. **Mouse passthrough**: when tmux mouse mode is on (`set -g mouse on`), tmux intercepts mouse events before they reach the app. ucode must remain fully usable with keyboard-only navigation. Mouse support is a convenience layer, never required.
3. **Scroll compatibility**: ucode uses alternate screen (`smcup`/`rmcup`). In tmux, scrollback is per-pane. ucode's transcript scroll (arrow keys, PgUp/PgDn, vim `j`/`k`) works inside the alternate screen. Tmux's own scroll mode (`Ctrl+B [`) scrolls tmux's scrollback buffer, not ucode's transcript — this is expected and correct.
4. **Clipboard integration**: use OSC 52 escape sequence for clipboard writes. This works through tmux (requires `set -g set-clipboard on` in tmux.conf, which is the default since tmux 3.3). Fallback: if OSC 52 is not supported, write to a file (`~/.local/share/ucode/clipboard`) and log a warning.
5. **Copy mode coexistence**: ucode's `v` copy mode operates inside the alternate screen on the transcript content. Tmux's copy mode (`Ctrl+B [`) operates on tmux's scrollback. Both are valid — ucode's copy mode gives structured access to transcript content; tmux's copy mode gives raw terminal output. Document this distinction.
6. **Terminal title**: set terminal title via OSC escape (`\033]0;ucode - session-name\007`). Tmux displays this in the pane title / window name.
7. **True color**: detect `$COLORTERM=truecolor` or tmux's `Tc` / `RGB` terminal features. Fall back to 256-color palette if not available.
8. **Resize handling**: handle `SIGWINCH` (terminal resize). Tmux sends this when panes are resized. Ratatui/crossterm handle this natively.

### Config

```toml
[tui.terminal]
clipboard = "osc52"       # "osc52" | "external" | "file"
# external uses xclip/xsel/pbcopy; file writes to ~/.local/share/ucode/clipboard
mouse = true              # enable mouse support (disable if fighting with tmux mouse mode)
```

---

## Section 11: Plugin UI Extension API

This is the stable, versioned surface plugins write to. Plugins observe events via the existing hooks API (PLANS.md Task 8.2, EPIC ISSUE 0802) and push UI updates via these calls.

### 11.1 API surface

| Call | Signature | Override class | Description |
|------|-----------|---------------|-------------|
| `ui::toast` | `(level, title, body?, ms?)` | Safe | Push toast notification |
| `ui::notify` | `(level, message)` | Safe | Short status-bar flash |
| `ui::sidebar_section` | `(id, title, lines, priority?)` | Safe | Register/update custom sidebar section |
| `ui::status_segment` | `(id, content, priority?)` | Safe | Add/update status bar segment |
| `ui::palette_command` | `(name, desc, handler)` | Safe | Add command to palette |
| `ui::transcript_event` | `(style, content)` | Guarded | Inject styled line into transcript |
| `ui::modal` | `(title, content, actions)` | Guarded | Open custom modal overlay |
| `ui::badge` | `(section_id, count, level)` | Safe (own section only) | Update badge on sidebar section |
| `ui::confirm` | `(title, message) → bool` | Guarded | Ask user yes/no |
| `ui::input_prompt` | `(title, placeholder) → String` | Guarded | Ask user for text input |

### 11.2 Plugin UI lifecycle

- Plugin registers UI extensions during `on_session_start` hook
- Host renders plugin content in standard style (plugins provide data, host renders — no custom widget trees)
- Plugin updates content by calling same API with same `id`
- On `on_session_end`, host cleans up all plugin-registered UI elements
- Risky overrides (replacing built-in sections, intercepting input) are blocked

### 11.3 Example: budget warning plugin

```
// Plugin observes on_budget_threshold_warning hook
// → calls ui::toast(warning, "Budget Warning", "80% of session budget used", 8000)
// → calls ui::status_segment("budget-plugin", "⚠ 80%", priority=high)
// → calls ui::transcript_event(warning, "[budget-plugin] approaching session limit")
```

### 11.4 Ties to PLANS.md / EPIC.md

- Plugin UI API is part of the versioned plugin API contract: PLANS.md Task 8.3, EPIC ISSUE 0804
- Override class matrix enforced by host: PLANS.md Task 8.2, EPIC ISSUE 0802
- Plugin isolation model: PLANS.md Task 8.4, EPIC ISSUE 0805
- External DCP-style plugins use same UI API: PLANS.md Task 8.5, EPIC ISSUE 0806

---

## Section 12: Implementation Issues (new/updated EPIC 7 issues)

These are the implementation issues that should be added to EPIC.md or used to expand existing ones:

### ISSUE 0701 (expand) — Ratatui fullscreen shell + panes

Add to scope:
- TitleBar with session name, lineage `⎇`, session ID, date
- TranscriptPane (scrollable, keyboard-navigable)
- InputBox (single-line expanding to multiline on Shift+Enter)
- Sidebar container (resizable, min 28 / default 34 / max 48 cols)
- StatusBar (system state line)
- Responsive layout: full sidebar at ≥120 cols, icon-strip at <120 cols

### ISSUE 0706 (expand) — Sidebar-first visual system

Add to scope:
- All 9 built-in sidebar sections (Router, Skill, Context, Workspace, Tools, Agents, Network, Jobs, MCP Servers)
- Collapsible sections with `▼`/`▶` toggle, `s1`–`s9` keybinds
- Collapsed summary line per section
- WORKSPACE section (new — diff tracking + checkpoint indicator)
- RouterEvent inline transcript line (`↪ router: ...`)
- Session lineage in title bar (`⎇`)
- Teal accent `#00d4aa` color system
- Semantic color roles (Safe/Warning/Danger/Accent/Muted)
- Sandbox tier colored dots (`●ws` `●net` `●strict`)
- Model group badges (`[fast]` `[strong]` `[longctx]`)
- Progress bar with color-shift by percentage
- Status bar as system state line (not keybind list)
- Transparency support via `ucode.toml` `[tui.theme]`
- Compact/comfortable density presets

### ISSUE 0708 (new) — Toast notification system + plugin UI extension API

**Goal:** Implement the toast notification system and the plugin UI extension API surface.

**Scope:**
- Toast component: stacked top-right, 4 types (info/success/warning/error), auto-dismiss timers, manual dismiss
- System-triggered toasts: checkpoint created, budget warning, agent completed/failed, MCP crash, auth expired
- Plugin UI extension API: `ui::toast`, `ui::notify`, `ui::sidebar_section`, `ui::status_segment`, `ui::palette_command`, `ui::transcript_event`, `ui::modal`, `ui::badge`, `ui::confirm`, `ui::input_prompt`
- Override class enforcement (Safe/Guarded/Risky) for all plugin UI calls
- Plugin UI lifecycle: register on session start, cleanup on session end
- Plugin sidebar sections rendered in standard style with `[plugin]` badge

**Acceptance tests:**
- System toast fires on checkpoint creation, budget warning, agent completion
- Plugin calls `ui::toast()` and toast appears with correct style and auto-dismiss
- Plugin registers sidebar section; it appears after built-in sections
- Plugin registers palette command; it appears in palette with `[plugin]` badge
- Guarded calls (modal, transcript_event) require plugin to have guarded capability declared
- Plugin UI elements cleaned up on session end

**Ties to:** PLANS.md Task 8.2 (hooks API), Task 8.3 (plugin API contract), EPIC ISSUE 0802, 0804

### ISSUE 0709 (new) — Copy mode + search overlay + keybind overlay

**Goal:** Implement transcript copy mode (vim-like selection), Ctrl+F search, and `?` keybind overlay.

**Acceptance tests:**
- `v` enters copy mode; `y` copies selected text to clipboard
- `Ctrl+F` opens search; matches highlighted; `n`/`N` navigate
- `?` shows full keybind reference

---

## Section 13: Config additions (ucode.toml)

New TUI-specific config keys to add to the cross-cutting config section of PLANS.md:

```toml
[tui.theme]
opacity = 90              # 0-100, terminal-dependent
preset = "hybrid"         # "hybrid" | "dark" | "light"
density = "comfortable"   # "compact" | "comfortable"
accent = "#00d4aa"        # accent color override

[tui.sidebar]
width = 34                # default sidebar width in columns
collapsed = []            # list of section ids to start collapsed, e.g. ["jobs", "mcp"]

[tui.keybinds]
preset = "vscode"         # "vscode" | "vim" | "emacs"
# Individual overrides layer on top of preset, e.g.:
# palette = "ctrl+p"
# session_switcher = "ctrl+s"

[tui.terminal]
clipboard = "osc52"       # "osc52" | "external" | "file"
mouse = true              # enable mouse support (disable if fighting with tmux mouse mode)
```

---

## Section 14: Ratatui crate dependencies

Add to `crates/ucode-tui/Cargo.toml`:

```toml
[dependencies]
ratatui = { version = "0.29", features = ["crossterm"] }
crossterm = "0.28"
tokio = { workspace = true }
serde = { workspace = true }
thiserror = { workspace = true }
ucode-core = { workspace = true }
ucode-auth = { workspace = true }
ucode-providers = { workspace = true }
ucode-tools = { workspace = true }
ucode-mcp = { workspace = true }
ucode-skills = { workspace = true }
ucode-plugins = { workspace = true }
```

---

## Section 15: Module structure for ucode-tui

```
crates/ucode-tui/src/
  lib.rs                  # public API, App struct, run()
  app.rs                  # App state, event loop
  theme.rs                # color palette, density presets, transparency
  keybinds.rs             # keybind definitions, preset system (vscode/vim/emacs), override loading
  clipboard.rs            # OSC 52, external (xclip/pbcopy), file fallback
  layout.rs               # terminal size detection, pane sizing
  components/
    title_bar.rs
    transcript.rs         # TranscriptPane, all transcript content types
    input.rs              # InputBox, SlashAutocomplete, MentionAutocomplete
    sidebar/
      mod.rs              # Sidebar container, collapsible section logic
      router.rs           # ROUTER section
      skill.rs            # SKILL section
      context.rs          # CONTEXT section
      workspace.rs        # WORKSPACE section (diff tracking, checkpoint)
      tools.rs            # TOOLS section
      agents.rs           # AGENTS section
      network.rs          # NETWORK section
      jobs.rs             # JOBS section
      mcp.rs              # MCP SERVERS section
      plugin_section.rs   # Plugin-injected sections
    status_bar.rs
    toast.rs              # Toast stack, auto-dismiss timers
  overlays/
    mod.rs
    palette.rs            # CommandPalette
    diff_modal.rs         # DiffModal
    approval_modal.rs     # ApprovalModal
    session_switcher.rs   # SessionSwitcher
    connect.rs            # ProviderConnect
    skill_browser.rs      # SkillBrowser
    model_picker.rs       # ModelPicker
    checkpoint_browser.rs
    job_manager.rs
    keybind_overlay.rs
    search_overlay.rs
    confirm_dialog.rs
    input_prompt.rs
    plugin_modal.rs
  plugin_ui/
    mod.rs                # Plugin UI extension API implementation
    api.rs                # ui::toast, ui::sidebar_section, etc.
    registry.rs           # Plugin UI element registry, lifecycle
```

---

## Section 16: Performance Architecture

Ratatui provides built-in double buffering (two `Buffer`s diffed on each `draw()`, only changed cells written to terminal). We do NOT implement custom double buffering. Instead, we focus on six performance-critical patterns:

### 16.1 Event-driven rendering with dirty flag

Never render at a fixed FPS when nothing changed. A static 60fps loop burns 7-50% CPU on idle.

```rust
struct App {
    dirty: bool,
    // ...
}
```

- Set `dirty = true` on any state mutation (new token, user input, resize, sidebar update, toast)
- Only call `terminal.draw()` when `dirty == true`
- Reset `dirty = false` after each draw

### 16.2 Adaptive frame rate with render tick

Use an adaptive render tick — fast during streaming, idle when nothing changes:

- **Streaming active:** `tokio::time::interval(Duration::from_millis(16))` (60fps cap). Tokens arrive frequently; high frame rate keeps the typewriter effect smooth.
- **Idle:** No tick. Render immediately and only on state change (input, resize, toast). CPU usage drops to ~0%.
- Transition: set `streaming = true` when first token arrives, `streaming = false` on stream-end event. Adjust interval accordingly.

The render tick is a MAXIMUM frame rate, not a minimum. If the model is slow (e.g., one token every 200ms), we still render each token within one frame (~16ms latency). If the model is fast (tokens every 5ms), multiple tokens land in `StreamingMessage.content` before the next frame draws — the frame simply renders the current string state. This is implicit coalescing from the frame rate cap, not explicit batching. There is no separate batch buffer.

This is the same principle as browser `requestAnimationFrame`: React state updates between frames are "batched" not because React has a batching buffer, but because the DOM only repaints at ~60fps.

### 16.3 Streaming typewriter UX

The goal: text appears naturally, like someone typing fast. No artificial delays, no explicit batching. Tokens are appended to state immediately; the render tick draws whatever state exists.

**How it works:**

1. Token arrives from LLM → appended to `StreamingMessage.content` immediately → `dirty = true`. No intermediate buffer.
2. On the next render tick (within 16ms), the transcript re-renders showing the current `content` string. If multiple tokens arrived since the last frame, they all appear at once — this is a natural consequence of the frame rate, not explicit batching.
3. A blinking block cursor `█` is rendered at the end of the streaming text to indicate "still generating."
4. When the stream completes, the cursor disappears and the message is finalized into the transcript history.

**Auto-scroll during streaming:**

- While streaming AND the user has not manually scrolled up, the transcript auto-scrolls to keep the latest line visible (pinned to bottom).
- If the user scrolls up during streaming (to read earlier context), auto-scroll disengages. A `↓ New content below` indicator appears at the bottom.
- Pressing `End` or `G` (vim) re-engages auto-scroll and jumps to bottom.

**Token granularity:**

- Tokens are appended raw as received — no word-boundary buffering. A token might be `"hel"` followed by `"lo "` followed by `"world"`. This matches how ChatGPT/Claude web render.
- Markdown rendering (bold, code blocks, headers) is applied incrementally. Partial markdown (e.g., opening `` ``` `` without closing) is rendered as plain text until the closing delimiter arrives.

**Streaming indicator in status bar:**

- While streaming: status bar shows `streaming... ●` with a pulsing dot and elapsed time.
- Token rate displayed: e.g., `42 tok/s` updated every second.

```rust
struct StreamingMessage {
    content: String,          // accumulated tokens
    started_at: Instant,      // for elapsed time display
    token_count: usize,       // for tok/s calculation
    is_complete: bool,        // false while streaming, true when done
}
```

### 16.4 Virtual scrolling for transcript

Sessions can grow to thousands of messages. Never iterate the full transcript on each frame.

- Maintain `scroll_offset: usize` and `viewport_height: u16`
- On draw, compute only the visible slice: `messages[scroll_offset..scroll_offset + viewport_height]`
- Line wrapping computed lazily per visible message (cache wrapped line count per message to avoid recomputation)
- Scroll position clamped to `0..=max(0, total_lines - viewport_height)`

### 16.5 Async event loop with `tokio::select!`

Multiple concurrent event sources multiplexed in one loop:

```rust
loop {
    tokio::select! {
        // Terminal input (crossterm EventStream)
        Some(event) = event_stream.next() => {
            app.handle_terminal_event(event?);
        }
        // Render tick (adaptive: 60fps during streaming, idle otherwise)
        _ = render_tick.tick(), if app.streaming || app.dirty => {
            if app.dirty {
                execute!(stderr, BeginSynchronizedUpdate)?;
                terminal.draw(|f| app.render(f))?;
                execute!(stderr, EndSynchronizedUpdate)?;
                app.dirty = false;
            }
        }
        // LLM streaming tokens — append immediately, mark dirty
        Some(token) = llm_rx.recv() => {
            app.push_token(token); // appends to StreamingMessage, sets dirty
        }
        // Stream completion
        Some(()) = stream_done_rx.recv() => {
            app.finalize_streaming(); // cursor disappears, message committed
        }
        // MCP / agent / system events
        Some(event) = system_rx.recv() => {
            app.handle_system_event(event);
        }
    }
}
```

When `app.streaming` is false and `app.dirty` is false, the render tick branch is disabled entirely — zero CPU.

### 16.6 Synchronized output

Wrap frame writes with crossterm's synchronized update to prevent partial-frame flicker. Already shown in the event loop above (§16.5). Supported by modern terminals (kitty, iTerm2, WezTerm, foot, tmux 3.3+). Terminals that don't support it silently ignore the escape sequences.

### 16.7 Cross-platform key filtering

Windows emits both `KeyEventKind::Press` and `KeyEventKind::Release` for each keypress. Filter to `Press` only:

```rust
if let Event::Key(key) = event {
    if key.kind != KeyEventKind::Press {
        return; // ignore Release/Repeat on Windows
    }
    // handle key
}
```

### Performance budget

| Operation | Target | Notes |
|-----------|--------|-------|
| Idle CPU | <1% | dirty flag + disabled render tick when idle |
| Streaming render | 60fps cap | ~16ms per frame, tokens appear within one frame |
| Token-to-screen latency | <16ms | token appended immediately, rendered on next tick |
| Input latency | <16ms | immediate dirty + next render tick |
| Transcript scroll | <5ms per frame | virtual scrolling, visible slice only |
| Startup to first frame | <100ms | no heavy init in render path |
| Auto-scroll disengage | instant | user scroll-up immediately stops auto-scroll |

---

## Document End

This design document serves as the canonical reference specification for the ucode TUI. All implementation against Phase 7 tasks (PLANS.md Task 7.1–7.4) and corresponding EPIC 7 issues (ISSUE 0701–0709) should follow these specifications precisely. The document includes visual mockups, component definitions, keybinds, color semantics, plugin API surface, config schema, and Ratatui module structure.

**Next step:** Use `superpowers:executing-plans` to begin implementation of ISSUE 0701 (Ratatui shell + panes) as the first implementation task.
