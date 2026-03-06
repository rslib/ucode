# Context Management Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement the `ucode-context` crate — native context management with zero-cost automatic strategies (dedup, supersede, purge), sandbox execution, LLM-driven pruning tools, hybrid FTS5+sqlite-vec knowledge base, and session continuity.

**Architecture:** Strategy Pipeline — each strategy implements `ContextStrategy` trait, chained in `ContextPipeline`. Knowledge base (SQLite FTS5 + sqlite-vec) and session continuity are separate infrastructure modules. 5 built-in tools registered via `ToolRegistry`.

**Tech Stack:** Rust 2024 edition, `rusqlite` (bundled, FTS5), `sqlite-vec`, `zerocopy`, `serde`/`serde_json`, `chrono`, `ucode-core`, `ucode-tools`. Optional: `fastembed` behind `local-embeddings` feature flag.

**Design doc:** `docs/plans/2026-03-06-context-management-design.md`

---

### Task 1: Crate scaffold + config types + ContextStrategy trait (8.8.1 + core types)

**Files:**
- Create: `crates/ucode-context/Cargo.toml`
- Create: `crates/ucode-context/src/lib.rs`
- Create: `crates/ucode-context/src/config.rs`
- Create: `crates/ucode-context/src/strategy.rs`
- Create: `crates/ucode-context/src/error.rs`
- Modify: `Cargo.toml` (workspace members)

**Step 1: Add crate to workspace**

Add `"crates/ucode-context"` to workspace members in root `Cargo.toml` and add
workspace dependencies for `rusqlite` and `sqlite-vec`.

Root `Cargo.toml` additions to `[workspace.dependencies]`:
```toml
rusqlite = { version = "0.35", features = ["bundled", "column_decltype"] }
sqlite-vec = "0.3"
zerocopy = { version = "0.8", features = ["derive"] }
```

`crates/ucode-context/Cargo.toml`:
```toml
[package]
name = "ucode-context"
version.workspace = true
edition.workspace = true

[dependencies]
chrono = { workspace = true }
rusqlite = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }
sqlite-vec = { workspace = true }
thiserror = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
ucode-core = { workspace = true }
ucode-tools = { path = "../ucode-tools" }
zerocopy = { workspace = true }

[dev-dependencies]
tempfile = { workspace = true }
tokio = { workspace = true }

[features]
default = []
local-embeddings = []  # placeholder for fastembed integration
```

**Step 2: Write error types**

Create `crates/ucode-context/src/error.rs`:
```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("knowledge base error: {0}")]
    KnowledgeBase(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("continuity error: {0}")]
    Continuity(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}
```

**Step 3: Write config types with tests**

Create `crates/ucode-context/src/config.rs` with all config structs from the
design doc: `ContextConfig`, `StrategiesConfig`, `KnowledgeBaseConfig`,
`EmbeddingMode`, `EmbeddingEndpointConfig`, `PruningConfig`, `PruningOverride`.

All structs derive `Debug, Clone, Serialize, Deserialize`. Implement `Default`
for `ContextConfig`, `StrategiesConfig`, `KnowledgeBaseConfig`, `PruningConfig`
with the defaults from the design doc.

Tests:
- `default_config_has_all_strategies_enabled` — verify all defaults
- `config_roundtrip_toml` — serialize to TOML string, deserialize back, assert equal
- `embedding_mode_default_is_auto`
- `pruning_override_resolves_correctly` — test override merging logic

**Step 4: Write ContextStrategy trait and StrategyContext**

Create `crates/ucode-context/src/strategy.rs`:
```rust
use ucode_core::{Message, TokenBudget, TokenCounter};

/// Shared context passed to each strategy.
pub struct StrategyContext<'a> {
    pub session_id: &'a str,
    pub turn_count: usize,
    pub token_budget: &'a TokenBudget,
    pub counter: &'a dyn TokenCounter,
}

/// Result of applying a strategy.
#[derive(Debug, Default)]
pub struct StrategyResult {
    pub messages_removed: usize,
    pub messages_modified: usize,
    pub chars_saved: usize,
}

/// A message transform strategy.
pub trait ContextStrategy: Send + Sync {
    fn name(&self) -> &str;
    fn apply(&self, messages: &mut Vec<Message>, ctx: &StrategyContext) -> StrategyResult;
}
```

Note: `StrategyContext` does NOT include `knowledge_base` reference yet. That
gets added in Task 4 when the knowledge base exists. Avoids circular dependency.

**Step 5: Write ContextPipeline**

Add to `strategy.rs`:
```rust
use crate::config::ContextConfig;

pub struct ContextPipeline {
    strategies: Vec<Box<dyn ContextStrategy>>,
}

impl ContextPipeline {
    pub fn new() -> Self {
        Self { strategies: Vec::new() }
    }

    pub fn add_strategy(&mut self, strategy: Box<dyn ContextStrategy>) {
        self.strategies.push(strategy);
    }

    pub fn transform(
        &self,
        messages: &mut Vec<Message>,
        ctx: &StrategyContext,
    ) -> Vec<StrategyResult> {
        self.strategies
            .iter()
            .map(|s| s.apply(messages, ctx))
            .collect()
    }

    pub fn strategy_names(&self) -> Vec<&str> {
        self.strategies.iter().map(|s| s.name()).collect()
    }
}
```

Tests:
- `empty_pipeline_returns_no_results`
- `pipeline_runs_strategies_in_order` — use a mock strategy that appends to a
  shared `Arc<Mutex<Vec<String>>>` to verify ordering

**Step 6: Write lib.rs**

Create `crates/ucode-context/src/lib.rs`:
```rust
//! ucode-context: native context management — strategies, knowledge base, session continuity

pub mod config;
pub mod error;
pub mod strategy;

pub use config::{
    ContextConfig, EmbeddingEndpointConfig, EmbeddingMode, KnowledgeBaseConfig,
    PruningConfig, PruningOverride, StrategiesConfig,
};
pub use error::ContextError;
pub use strategy::{ContextPipeline, ContextStrategy, StrategyContext, StrategyResult};
```

**Step 7: Verify build**

Run: `cargo build -p ucode-context`
Run: `cargo test -p ucode-context`
Run: `cargo clippy -p ucode-context -- -D warnings`

Expected: all pass, 0 warnings.

**Step 8: Commit**

```
feat(context): scaffold ucode-context crate with config, strategy trait, and pipeline
```

---

### Task 2: Automatic zero-cost strategies (8.8.2)

**Files:**
- Create: `crates/ucode-context/src/dedup.rs`
- Create: `crates/ucode-context/src/supersede.rs`
- Create: `crates/ucode-context/src/purge.rs`
- Modify: `crates/ucode-context/src/lib.rs`

**Step 1: Write DedupStrategy tests**

In `crates/ucode-context/src/dedup.rs`, write `#[cfg(test)] mod tests` first:
- `dedup_removes_duplicate_file_reads` — two identical `read_file` ToolResults
  for same path, second gets replaced with placeholder
- `dedup_keeps_different_content` — same path but different content hash, both kept
- `dedup_ignores_non_file_tools` — ToolResults from `run_cmd` are not deduped
- `dedup_replaces_with_correct_placeholder` — verify placeholder text format

**Step 2: Implement DedupStrategy**

```rust
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use crate::strategy::{ContextStrategy, StrategyContext, StrategyResult};
use ucode_core::{Message, Part};

const FILE_READ_TOOLS: &[&str] = &["read_file", "list_dir", "ripgrep_search", "ast_search"];

pub struct DedupStrategy;

impl ContextStrategy for DedupStrategy {
    fn name(&self) -> &str { "dedup" }

    fn apply(&self, messages: &mut Vec<Message>, _ctx: &StrategyContext) -> StrategyResult {
        // Track (tool_name, file_path) -> content_hash
        let mut seen: HashMap<(String, String), u64> = HashMap::new();
        let mut modified = 0usize;
        let mut chars_saved = 0usize;

        for msg in messages.iter_mut() {
            for part in msg.parts.iter_mut() {
                if let Part::ToolResult(tr) = part {
                    if !FILE_READ_TOOLS.contains(&tr.name.as_str()) {
                        continue;
                    }
                    let path = extract_file_path_from_name(&tr.name, &tr.result);
                    let content_str = tr.result.to_string();
                    let hash = hash_content(&content_str);
                    let key = (tr.name.clone(), path.clone());

                    if let Some(prev_hash) = seen.get(&key) {
                        if *prev_hash == hash {
                            let old_len = content_str.len();
                            tr.result = serde_json::Value::String(
                                format!("[already in context -- see earlier read of {path}]")
                            );
                            chars_saved += old_len.saturating_sub(tr.result.to_string().len());
                            modified += 1;
                        } else {
                            seen.insert(key, hash);
                        }
                    } else {
                        seen.insert(key, hash);
                    }
                }
            }
        }

        StrategyResult { messages_removed: 0, messages_modified: modified, chars_saved }
    }
}

fn hash_content(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

fn extract_file_path_from_name(_tool_name: &str, result: &serde_json::Value) -> String {
    // Try common field names in tool result
    if let Some(path) = result.get("path").and_then(|v| v.as_str()) {
        return path.to_string();
    }
    if let Some(path) = result.get("file").and_then(|v| v.as_str()) {
        return path.to_string();
    }
    // Fallback: use the result string itself as key
    result.to_string()
}
```

**Step 3: Run tests**

Run: `cargo test -p ucode-context dedup`
Expected: all pass.

**Step 4: Write SupersedeStrategy tests**

In `crates/ucode-context/src/supersede.rs`:
- `supersede_replaces_write_when_later_read_exists` — write then read of same
  file, write content replaced with placeholder
- `supersede_keeps_write_without_later_read` — write without subsequent read, kept
- `supersede_handles_apply_patch` — `apply_patch` tool result treated as write

**Step 5: Implement SupersedeStrategy**

Scan messages in two passes:
1. Forward pass: record all file writes (tool name + path + message index)
2. Forward pass: record all file reads (tool name + path + message index)
3. For each write, if a read of the same path exists at a later index, replace
   the write's content with `[superseded by later read of {path}]`

Extract file path from `ToolCall` args (the `path` or `file` field in JSON args)
by scanning the corresponding `ToolCall` part in the same or adjacent message.

**Step 6: Run tests**

Run: `cargo test -p ucode-context supersede`
Expected: all pass.

**Step 7: Write PurgeErrorsStrategy tests**

In `crates/ucode-context/src/purge.rs`:
- `purge_removes_old_errors` — error ToolResult older than 3 turns, purged
- `purge_keeps_recent_errors` — error ToolResult within 3 turns, kept
- `purge_respects_configurable_turn_count` — custom turn threshold
- `purge_also_purges_corresponding_tool_call` — ToolCall args for the error are
  also replaced

**Step 8: Implement PurgeErrorsStrategy**

```rust
pub struct PurgeErrorsStrategy {
    pub after_turns: usize,
}

impl PurgeErrorsStrategy {
    pub fn new(after_turns: usize) -> Self {
        Self { after_turns }
    }
}
```

Count turns from the end. For each `ToolResult { is_error: true }` that is more
than `after_turns` from the end, replace content with
`[error purged after {n} turns]`. Also find the matching `ToolCall` (by `id`)
and replace its args with `{}`.

**Step 9: Run all strategy tests**

Run: `cargo test -p ucode-context`
Run: `cargo clippy -p ucode-context -- -D warnings`
Expected: all pass.

**Step 10: Update lib.rs exports**

Add `pub mod dedup;`, `pub mod supersede;`, `pub mod purge;` to lib.rs.
Export the strategy structs.

**Step 11: Commit**

```
feat(context): implement dedup, supersede, and purge-errors strategies
```

---

### Task 3: Sandbox execution (8.8.3)

**Files:**
- Create: `crates/ucode-context/src/sandbox.rs`
- Modify: `crates/ucode-context/src/lib.rs`

**Step 1: Write SandboxInterceptor tests**

- `sandbox_replaces_large_output_with_summary` — ToolResult > 2000 chars gets
  replaced with metadata summary (line count, first/last 3 lines)
- `sandbox_keeps_small_output` — ToolResult < 2000 chars, unchanged
- `sandbox_respects_custom_threshold` — configurable threshold
- `sandbox_summary_format` — verify the summary format includes line count,
  content type, first/last lines, and knowledge base note

**Step 2: Implement SandboxInterceptor**

```rust
pub struct SandboxInterceptor {
    pub threshold_chars: usize,
    // Note: knowledge_base storage is deferred to Task 5.
    // For now, just replace content with summary. Task 5 will add KB storage.
}
```

Implements `ContextStrategy`. For each `ToolResult` whose
`result.to_string().len() > threshold_chars`:
1. Build metadata summary (line count, first 3 lines, last 3 lines)
2. Replace `tr.result` with the summary string

The actual knowledge base storage will be wired in Task 5 when KB exists.

**Step 3: Run tests**

Run: `cargo test -p ucode-context sandbox`
Expected: all pass.

**Step 4: Add pipeline builder**

Add `build_pipeline(config: &ContextConfig) -> ContextPipeline` function to
`lib.rs` that constructs the pipeline from config, adding only enabled strategies
in the correct order: dedup -> supersede -> purge -> sandbox.

Test: `pipeline_from_config_respects_toggles` — disable dedup, verify pipeline
only has 3 strategies.

**Step 5: Run all tests**

Run: `cargo test -p ucode-context`
Run: `cargo clippy -p ucode-context -- -D warnings`
Expected: all pass.

**Step 6: Commit**

```
feat(context): implement sandbox interceptor for large tool outputs
```

---

### Task 4: Hybrid knowledge base — FTS5 + sqlite-vec (8.8.5)

Note: Task 4 is 8.8.5 (knowledge base) before 8.8.4 (pruning tools) because
the tools depend on the knowledge base.

**Files:**
- Create: `crates/ucode-context/src/knowledge.rs`
- Create: `crates/ucode-context/src/embedder.rs`
- Modify: `crates/ucode-context/src/lib.rs`
- Modify: `crates/ucode-context/src/sandbox.rs` (wire KB storage)

**Step 1: Write Embedder trait**

Create `crates/ucode-context/src/embedder.rs`:
```rust
use crate::error::ContextError;

/// Abstraction for embedding generation.
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, ContextError>;
    fn dimensions(&self) -> usize;
}
```

No implementations yet — those come later. For now, the knowledge base works
with `Option<Box<dyn Embedder>>`.

**Step 2: Write KnowledgeBase tests (FTS5 only)**

In `crates/ucode-context/src/knowledge.rs`:
- `kb_open_creates_database` — open in temp dir, verify file exists
- `kb_store_and_search_keyword` — store content, search by keyword, find it
- `kb_search_returns_ranked_results` — store multiple, verify BM25 ranking
- `kb_search_no_results` — search for nonexistent term, empty results
- `kb_store_with_metadata` — store with metadata, verify it's returned
- `kb_porter_stemming` — search "running" finds content with "run"

**Step 3: Implement KnowledgeBase (FTS5 only)**

```rust
use std::path::Path;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use crate::embedder::Embedder;
use crate::error::ContextError;

pub struct KnowledgeBase {
    conn: Connection,
    embedder: Option<Box<dyn Embedder>>,
}

pub struct KnowledgeEntry {
    pub id: i64,
    pub source: String,
    pub content: String,
    pub metadata: Option<String>,
    pub created_at: DateTime<Utc>,
    pub score: f64,
}
```

Implement `open()`, `store()`, `search_keyword()`, `search()` (delegates to
keyword-only when no embedder).

Schema creation in `open()`:
- `knowledge` table
- `knowledge_fts` FTS5 virtual table with Porter tokenizer
- Insert/delete triggers for FTS sync

**Step 4: Run FTS5 tests**

Run: `cargo test -p ucode-context knowledge`
Expected: all pass.

**Step 5: Add sqlite-vec vector search**

Add to `open()`: if embedder is Some, create `knowledge_vec` virtual table.

Add `store()` logic: if embedder available, embed content and insert into
`knowledge_vec`.

Add `search_vector()`: embed query, KNN search via sqlite-vec.

Add `search()` hybrid: run both FTS5 and vector, combine via RRF.

Register sqlite-vec extension in `open()`:
```rust
unsafe {
    rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
        sqlite_vec::sqlite3_vec_init as *const (),
    )));
}
```

Tests:
- `kb_vector_search_finds_similar` — with a mock embedder that returns
  predictable vectors, verify KNN search works
- `kb_hybrid_search_combines_results` — verify RRF combination
- `kb_no_embedder_skips_vector` — without embedder, search() = search_keyword()

**Step 6: Wire sandbox to knowledge base**

Update `SandboxInterceptor` to accept an `Arc<KnowledgeBase>` reference.
When sandboxing content, call `kb.store()` before replacing.

Update `StrategyContext` to include `knowledge_base: Option<&'a KnowledgeBase>`.

**Step 7: Run all tests**

Run: `cargo test -p ucode-context`
Run: `cargo clippy -p ucode-context -- -D warnings`
Expected: all pass.

**Step 8: Commit**

```
feat(context): implement hybrid FTS5 + sqlite-vec knowledge base
```

---

### Task 5: LLM pruning tools (8.8.4)

**Files:**
- Create: `crates/ucode-context/src/tools.rs`
- Modify: `crates/ucode-context/src/lib.rs`

**Step 1: Write tool handler tests**

In `crates/ucode-context/src/tools.rs`:
- `knowledge_search_returns_results` — invoke handler with query, verify results
- `knowledge_store_indexes_content` — invoke handler, verify content searchable
- `context_prune_removes_messages` — invoke with indices, verify messages removed
- `context_compress_replaces_output` — invoke with message index + key findings
- `context_distill_summarizes_range` — invoke with range, verify digest created

**Step 2: Implement tool handlers**

5 tool handlers, each implementing `ToolHandler`:

```rust
use std::sync::Arc;
use tokio::sync::RwLock;
use ucode_core::Message;
use ucode_tools::{ToolHandler, ToolSpec, ToolRegistry};

pub struct KnowledgeSearchHandler {
    kb: Arc<KnowledgeBase>,
}

pub struct KnowledgeStoreHandler {
    kb: Arc<KnowledgeBase>,
}

pub struct ContextPruneHandler {
    messages: Arc<RwLock<Vec<Message>>>,
}

pub struct ContextCompressHandler {
    messages: Arc<RwLock<Vec<Message>>>,
}

pub struct ContextDistillHandler {
    messages: Arc<RwLock<Vec<Message>>>,
}
```

Each handler:
- Parses args from `serde_json::Value`
- Performs the operation
- Returns `Result<serde_json::Value, CoreError>`

**Step 3: Implement register_context_tools()**

```rust
pub fn register_context_tools(
    registry: &mut ToolRegistry,
    kb: Arc<KnowledgeBase>,
    messages: Arc<RwLock<Vec<Message>>>,
) -> Result<(), CoreError> {
    // Register all 5 tools with their specs and handlers
}
```

Each tool gets a `ToolSpec` with name, description, and JSON schema for
parameters (matching the schemas in the design doc).

**Step 4: Run tests**

Run: `cargo test -p ucode-context tools`
Expected: all pass.

**Step 5: Add system prompt injection logic**

Add function to generate the pruning system prompt fragment:
```rust
pub fn pruning_system_prompt(
    context_usage_pct: u8,
    threshold_pct: u8,
) -> Option<String>
```

Returns `None` if usage < threshold. Returns the prompt fragment if above.

Test: `pruning_prompt_only_above_threshold`

**Step 6: Add pruning config resolution**

Add function to check if pruning is enabled for a given model:
```rust
pub fn is_pruning_enabled(config: &PruningConfig, model_name: &str) -> bool
```

Checks overrides first, falls back to global `enabled`.

Test: `pruning_disabled_for_opus_by_default`

**Step 7: Run all tests**

Run: `cargo test -p ucode-context`
Run: `cargo clippy -p ucode-context -- -D warnings`
Expected: all pass.

**Step 8: Commit**

```
feat(context): implement 5 LLM pruning/knowledge tools with smart triggering
```

---

### Task 6: Session continuity (8.8.6)

**Files:**
- Create: `crates/ucode-context/src/continuity.rs`
- Modify: `crates/ucode-context/src/lib.rs`

**Step 1: Write continuity event types and tests**

In `crates/ucode-context/src/continuity.rs`:

Types: `ContinuityEvent`, `ContinuityEventType` (8 variants), `CompactionSnapshot`,
`ErrorRecord`, `GitState`.

Tests:
- `capture_event_adds_to_log` — capture events, verify log grows
- `event_types_serialize_roundtrip` — serde roundtrip for all 8 variants
- `snapshot_includes_all_fields` — create snapshot, verify all fields populated

**Step 2: Implement SessionContinuity**

```rust
pub struct SessionContinuity {
    event_log: Vec<ContinuityEvent>,
    snapshot: Option<CompactionSnapshot>,
    session_dir: PathBuf,
}

impl SessionContinuity {
    pub fn new(session_dir: PathBuf) -> Self;
    pub fn capture_event(&mut self, event_type: ContinuityEventType, summary: String);
    pub fn create_snapshot(&mut self, /* session state fields */) -> CompactionSnapshot;
    pub fn save(&self) -> Result<(), ContextError>;
    pub fn load(session_dir: &Path) -> Result<Self, ContextError>;
    pub fn restore_prompt(&self) -> Option<String>;
}
```

`save()` writes:
- `{session_dir}/continuity_events.json` — event log
- `{session_dir}/continuity.json` — latest snapshot

`load()` reads both files. Missing files = empty state (not an error).

`restore_prompt()` returns the system message prefix from the snapshot, or None
if no snapshot exists.

**Step 3: Write snapshot creation tests**

- `create_snapshot_captures_goals` — events with GoalEstablished type appear in
  snapshot.user_goals
- `create_snapshot_captures_errors` — ErrorEncountered events appear in
  snapshot.error_history
- `create_snapshot_captures_git_state` — GitCommit events appear in
  snapshot.git_state
- `create_snapshot_captures_working_set` — FileChanged events appear in
  snapshot.working_set

**Step 4: Write persistence tests**

- `save_and_load_roundtrip` — save to temp dir, load back, verify equal
- `load_missing_files_returns_empty` — load from empty dir, no error
- `restore_prompt_format` — verify the system message prefix format

**Step 5: Implement snapshot creation logic**

`create_snapshot()` scans the event log and populates:
- `user_goals` from `GoalEstablished` events
- `working_set` from `FileChanged` events (deduplicated)
- `error_history` from `ErrorEncountered` events
- `git_state` from `GitCommit` events
- `key_decisions` from `Decision` events
- `pending_tasks` — empty for now (would need LLM to determine)

**Step 6: Run all tests**

Run: `cargo test -p ucode-context`
Run: `cargo clippy -p ucode-context -- -D warnings`
Expected: all pass.

**Step 7: Final integration — update lib.rs with all exports**

Update `lib.rs` to export all modules and key types. Add `build_pipeline()`
function that constructs the full pipeline from config.

**Step 8: Run full workspace build**

Run: `cargo build --workspace`
Run: `cargo test -p ucode-context`
Run: `cargo clippy --workspace -- -D warnings`
Expected: all pass.

**Step 9: Commit**

```
feat(context): implement session continuity with event log and compaction snapshots
```

---

## Verification Checklist

After all 6 tasks:

- [ ] `cargo build -p ucode-context` succeeds
- [ ] `cargo test -p ucode-context` — all tests pass
- [ ] `cargo clippy -p ucode-context -- -D warnings` — 0 warnings
- [ ] `cargo build --workspace` — no breakage in other crates
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — 0 warnings
- [ ] Design doc exists: `docs/plans/2026-03-06-context-management-design.md`
- [ ] PLANS.md updated with design decisions
- [ ] EPIC.md updated with design decisions

## Test Count Target

Minimum tests per task:
- Task 1 (scaffold): ~8 tests (config defaults, roundtrip, pipeline ordering)
- Task 2 (strategies): ~12 tests (4 dedup + 3 supersede + 4 purge + 1 integration)
- Task 3 (sandbox): ~5 tests (threshold, summary format, pipeline builder)
- Task 4 (knowledge base): ~9 tests (6 FTS5 + 3 vector/hybrid)
- Task 5 (tools): ~7 tests (5 tool handlers + prompt + config resolution)
- Task 6 (continuity): ~7 tests (events + snapshot + persistence)

Total: ~48+ tests
