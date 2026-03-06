# Context Management Design (Task 8.8 / ISSUE 0808)

## Goal

Native context management as a core crate (`ucode-context`). Combines strategies
from opencode-dcp, rlm-skill, and context-mode. Runs directly in the LLM call
path with zero serialization overhead. Per-model configurable so users can tune
strategies per model (e.g., disable LLM pruning for Opus, enable for Sonnet).

Native, not plugin: automatic strategies are pure Rust message rewrites. Knowledge
base and session continuity are core infrastructure. No WASM boundary overhead.
External plugins (ISSUE 0806) can still add custom strategies via the transform
pipeline.

## Architecture: Strategy Pipeline (Approach B)

Each strategy implements a common `ContextStrategy` trait. A `ContextPipeline`
chains strategies in order. The knowledge base and session continuity are separate
modules -- they are infrastructure, not message transforms.

Why this approach:
- Strategies need per-instance state (dedup tracks file hashes, purge tracks turn
  counts) -- rules out pure functions.
- 3+ concrete implementations today (dedup, supersede, purge, sandbox) -- meets
  the threshold for a trait.
- Mirrors the existing transform pipeline pattern from ISSUE 0806.
- Knowledge base and session continuity are different concerns from message
  transforms -- keeping them separate avoids a monolith.

## Crate Structure

```
crates/ucode-context/
├── Cargo.toml
└── src/
    ├── lib.rs              # Public API: ContextPipeline, register_context_tools()
    ├── config.rs           # ContextConfig deserialization from ucode.toml
    ├── strategy.rs         # ContextStrategy trait + StrategyContext
    ├── dedup.rs            # DedupStrategy
    ├── supersede.rs        # SupersedeStrategy
    ├── purge.rs            # PurgeErrorsStrategy
    ├── sandbox.rs          # SandboxInterceptor (large output handling)
    ├── knowledge.rs        # KnowledgeBase (SQLite FTS5)
    ├── continuity.rs       # SessionContinuity (event log + compaction snapshots)
    └── tools.rs            # 5 built-in tool handlers
```

Dependencies: `ucode-core` (Message, Session, BudgetManager), `ucode-tools`
(ToolRegistry, ToolSpec, ToolHandler), `rusqlite` (bundled + FTS5), `sqlite-vec`,
`serde`, `serde_json`, `chrono`, `zerocopy`.

Optional dependencies (behind feature flags):
- `local-embeddings`: `fastembed` (ONNX Runtime + all-MiniLM-L6-v2 for local
  embedding generation). Adds ~50MB binary size. Not required -- FTS5 keyword
  search works without it.

## Core Types

```rust
/// Shared context passed to each strategy.
pub struct StrategyContext<'a> {
    pub session_id: &'a str,
    pub turn_count: usize,
    pub token_budget: &'a TokenBudget,
    pub counter: &'a dyn TokenCounter,
    pub knowledge_base: &'a KnowledgeBase,
}

/// Result of applying a strategy.
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

/// Ordered pipeline of strategies.
pub struct ContextPipeline {
    strategies: Vec<Box<dyn ContextStrategy>>,
    config: ContextConfig,
}

impl ContextPipeline {
    /// Build pipeline from config, only including enabled strategies.
    pub fn from_config(config: ContextConfig) -> Self;

    /// Run all strategies in order, returning aggregate results.
    pub fn transform(
        &self,
        messages: &mut Vec<Message>,
        ctx: &StrategyContext,
    ) -> Vec<StrategyResult>;
}
```

## Task 8.8.1: Configuration

Deserialized from `[context_management]` in `ucode.toml`:

```rust
pub struct ContextConfig {
    pub enabled: bool,                    // global toggle
    pub strategies: StrategiesConfig,
    pub pruning: PruningConfig,
}

pub struct StrategiesConfig {
    pub dedup: bool,                      // default true
    pub supersede_writes: bool,           // default true
    pub purge_errors: bool,               // default true
    pub purge_errors_after_turns: usize,  // default 3
    pub sandbox_large_outputs: bool,      // default true
    pub sandbox_threshold_chars: usize,   // default 2000
    pub knowledge_base: bool,             // default true
    pub session_continuity: bool,         // default true
}

pub struct KnowledgeBaseConfig {
    pub enabled: bool,                    // default true
    pub embedding: EmbeddingMode,         // default Auto
    pub embedding_endpoint: Option<EmbeddingEndpointConfig>,
}

/// How to generate embeddings for vector search.
pub enum EmbeddingMode {
    /// Use custom endpoint if configured, else provider API if available, else FTS5 only.
    Auto,
    /// Use fastembed with local ONNX model (requires `local-embeddings` feature).
    Local,
    /// Use a custom OpenAI-compatible embedding API endpoint.
    Endpoint,
    /// FTS5 keyword search only, no vector search.
    None,
}

/// Custom embedding endpoint configuration (OpenAI-compatible API).
pub struct EmbeddingEndpointConfig {
    pub url: String,                      // e.g., "http://localhost:11434/v1/embeddings"
    pub model: String,                    // e.g., "all-minilm", "nomic-embed-text"
    pub dimensions: usize,               // e.g., 384, 768
    // API key sourced from UCODE_EMBEDDING_API_KEY env var or auth store
}

pub struct PruningConfig {
    pub enabled: bool,                    // default true
    pub trigger_threshold_pct: u8,        // default 60
    pub model: String,                    // "auto" or explicit model name
    pub overrides: HashMap<String, PruningOverride>,
}

pub struct PruningOverride {
    pub enabled: Option<bool>,
    pub trigger_threshold_pct: Option<u8>,
}
```

TOML example:

```toml
[context_management]
enabled = true

[context_management.strategies]
dedup = true
supersede_writes = true
purge_errors = true
purge_errors_after_turns = 3
sandbox_large_outputs = true
sandbox_threshold_chars = 2000
knowledge_base = true
session_continuity = true

[context_management.knowledge_base]
enabled = true
embedding = "auto"  # "auto" | "local" | "endpoint" | "none"

# Custom embedding endpoint (OpenAI-compatible API)
# Supports Ollama, LiteLLM, vLLM, or any OpenAI-compatible service
[context_management.knowledge_base.embedding_endpoint]
url = "http://localhost:11434/v1/embeddings"
model = "all-minilm"
dimensions = 384
# API key: set UCODE_EMBEDDING_API_KEY env var, or omit for local services

[context_management.pruning]
enabled = true
trigger_threshold_pct = 60
model = "auto"

[context_management.pruning.overrides."claude-opus-4"]
enabled = false

[context_management.pruning.overrides."claude-sonnet-4"]
enabled = true
trigger_threshold_pct = 50
```

Embedding resolution order for `"auto"`:
1. Custom `embedding_endpoint` if configured -- use it
2. Session provider has embedding API (OpenAI, etc.) -- use it
3. Fall back to FTS5 keyword search only

## Task 8.8.2: Automatic Zero-Cost Strategies

Three strategies, each implementing `ContextStrategy`. All are pure algorithmic --
no LLM calls, no I/O.

### DedupStrategy

Tracks `HashMap<(tool_name, file_path), content_hash>` across the message array.
When a `ToolResult` for a file-read tool has the same content hash as a previous
read, replaces the content with:

```
[already in context -- see earlier read of {path}]
```

Content hash: `DefaultHasher` of the result string (no crypto needed).

File-read tools detected by name: `read_file`, `list_dir`, `ripgrep_search`,
`ast_search`. Configurable via a set if needed later.

### SupersedeStrategy

Tracks file write/read ordering across the message array. When a file was written
(via `write_file` / `apply_patch` tool results) and later read, removes the
write's full content from the earlier message, replacing with:

```
[superseded by later read of {path}]
```

File path extracted from tool args (`path` or `file` field in the JSON args).

### PurgeErrorsStrategy

Scans for `ToolResult { is_error: true }` that are older than
`purge_errors_after_turns` turns from the end. Replaces their content with:

```
[error purged after {n} turns]
```

Also purges the corresponding `ToolCall` args (the request that produced the
error) to save additional tokens.

## Task 8.8.3: Sandbox Execution

`SandboxInterceptor` implements `ContextStrategy`. For any `ToolResult` whose
serialized content exceeds `sandbox_threshold_chars`:

1. Stores full content in `KnowledgeBase` with metadata (tool name, file path,
   timestamp, content type).
2. Replaces content in the message with a metadata summary:
   - Line count
   - Content type (inferred from tool name or file extension)
   - First 3 lines
   - Last 3 lines
   - Note: `"Full content indexed in knowledge base. Use knowledge_search to retrieve."`

Runs after dedup/supersede/purge in the pipeline (so we don't sandbox content
that would have been deduped anyway).

## Task 8.8.4: LLM Pruning Tools

Five tools registered as native built-in tools via `ToolRegistry`:

| Tool               | Purpose                                              |
| ------------------ | ---------------------------------------------------- |
| `context_distill`  | Summarize a range of messages into a compact digest  |
| `context_compress` | Replace verbose tool outputs with key findings       |
| `context_prune`    | Remove messages by index/range                       |
| `knowledge_search` | Query FTS5 knowledge base                            |
| `knowledge_store`  | Explicitly index content in knowledge base           |

### Smart triggering

System prompt injection for pruning tools only happens when context usage exceeds
`trigger_threshold_pct` of the model's context window. Below threshold, no system
prompt overhead -- the tools exist in the registry but the LLM is not told about
them.

Injected system prompt fragment (when triggered):

```
You have context management tools available. Your context is at {pct}% capacity.
Consider using context_distill, context_compress, or context_prune to free space
for continued work. Use knowledge_search to find previously indexed content.
```

### Per-model control

`context_distill`, `context_compress`, and `context_prune` are excluded from the
tool list sent to the provider when pruning is disabled for that model (via
`overrides`). `knowledge_search` and `knowledge_store` are always available
regardless of pruning config.

### Cheaper-model delegation

When `pruning.model` is not `"auto"`, pruning tool calls are routed to the
specified model for summarization. Initial implementation: `"auto"` only.
Cheaper-model delegation is a future enhancement.

### Tool schemas

```rust
// context_distill
{
    "start_index": usize,    // first message index to distill
    "end_index": usize,      // last message index (inclusive)
    "focus": Option<String>,  // optional: what to focus the summary on
}

// context_compress
{
    "message_index": usize,   // message containing verbose tool output
    "key_findings": String,   // what to keep from the output
}

// context_prune
{
    "indices": Vec<usize>,    // message indices to remove
    "reason": String,         // why these are no longer relevant
}

// knowledge_search
{
    "query": String,          // search query
    "limit": Option<usize>,   // max results (default 5)
}

// knowledge_store
{
    "content": String,        // content to index
    "source": String,         // where this came from
    "metadata": Option<String>, // optional metadata
}
```

## Task 8.8.5: Hybrid Knowledge Base (FTS5 + sqlite-vec)

Two search modes in one SQLite database:
- **FTS5** (always available): keyword search with Porter stemming and BM25
  ranking. Zero additional dependencies beyond rusqlite.
- **sqlite-vec** (when embeddings available): semantic vector search with cosine
  similarity. Finds conceptually related content even with different words.

Why FTS5 for keyword search (not Tantivy, not custom TF-IDF):
- Already in SQLite -- zero deps beyond rusqlite bundled feature
- Same database file as sqlite-vec vectors
- Porter stemming handles English well, BM25 ranking is solid
- Our scale (hundreds of entries per session) makes anything heavier overkill

When both are available, results are combined via Reciprocal Rank Fusion (RRF)
for best-of-both-worlds retrieval.

```rust
pub struct KnowledgeBase {
    conn: rusqlite::Connection,
    embedder: Option<Box<dyn Embedder>>,
}

/// Abstraction for embedding generation.
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, ContextError>;
    fn dimensions(&self) -> usize;
}
```

SQLite database at `{session_dir}/knowledge.db`.

### Schema

```sql
CREATE TABLE knowledge (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    content TEXT NOT NULL,
    metadata TEXT,
    created_at TEXT NOT NULL
);

-- FTS5 keyword search (always available)
CREATE VIRTUAL TABLE knowledge_fts USING fts5(
    content,
    source,
    content='knowledge',
    content_rowid='id',
    tokenize='porter'
);

-- Triggers to keep FTS in sync
CREATE TRIGGER knowledge_ai AFTER INSERT ON knowledge BEGIN
    INSERT INTO knowledge_fts(rowid, content, source)
    VALUES (new.id, new.content, new.source);
END;

CREATE TRIGGER knowledge_ad AFTER DELETE ON knowledge BEGIN
    INSERT INTO knowledge_fts(knowledge_fts, rowid, content, source)
    VALUES ('delete', old.id, old.content, old.source);
END;

-- Vector search (created only when embedder is available)
CREATE VIRTUAL TABLE knowledge_vec USING vec0(
    embedding float[{dimensions}]
);
```

### Embedder Implementations

```rust
/// OpenAI-compatible embedding API (works with Ollama, LiteLLM, vLLM, etc.)
pub struct EndpointEmbedder {
    url: String,
    model: String,
    dimensions: usize,
    api_key: Option<String>,
}

/// Provider-native embedding (uses session provider's embedding API)
pub struct ProviderEmbedder {
    // Delegates to the active provider's embedding endpoint
}

/// Local ONNX model via fastembed (behind `local-embeddings` feature flag)
#[cfg(feature = "local-embeddings")]
pub struct LocalEmbedder {
    model: fastembed::TextEmbedding,
}
```

### API

```rust
impl KnowledgeBase {
    /// Open or create the knowledge base for a session.
    /// If embedder is Some, vector search is enabled.
    pub fn open(
        session_dir: &Path,
        embedder: Option<Box<dyn Embedder>>,
    ) -> Result<Self, ContextError>;

    /// Store content with source and optional metadata.
    /// If embedder is available, also stores embedding vector.
    pub fn store(
        &self,
        source: &str,
        content: &str,
        metadata: Option<&str>,
    ) -> Result<i64, ContextError>;

    /// Hybrid search: FTS5 keyword + vector similarity combined via RRF.
    /// Falls back to FTS5-only when embedder is not available.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>, ContextError>;

    /// FTS5 keyword search only.
    pub fn search_keyword(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>, ContextError>;

    /// Vector similarity search only (returns empty if no embedder).
    pub fn search_vector(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<KnowledgeEntry>, ContextError>;
}

pub struct KnowledgeEntry {
    pub id: i64,
    pub source: String,
    pub content: String,
    pub metadata: Option<String>,
    pub created_at: DateTime<Utc>,
    pub score: f64,  // Combined RRF score, or FTS5/vector score
}
```

### Reciprocal Rank Fusion (RRF)

When both FTS5 and vector results are available, combine them:

```
RRF_score(doc) = 1/(k + rank_fts5) + 1/(k + rank_vector)
```

Where `k = 60` (standard constant). This gives equal weight to both retrieval
methods and handles cases where a document appears in only one result set.

## Task 8.8.6: Session Continuity

Captures significant events during a session and creates compaction snapshots so
sessions survive compaction cycles without losing critical state.

### Event Types

```rust
pub struct ContinuityEvent {
    pub timestamp: DateTime<Utc>,
    pub event_type: ContinuityEventType,
    pub summary: String,
}

pub enum ContinuityEventType {
    /// User stated or refined their goal
    GoalEstablished,
    /// File was created or modified during the session
    FileChanged,
    /// Test/build/lint result
    TestResult,
    /// Architectural or implementation decision made
    Decision,
    /// Significant tool output worth remembering
    ToolOutput,
    /// An approach was tried and failed (with reason)
    ErrorEncountered,
    /// A git commit was made during the session
    GitCommit,
    /// Model or skill was switched
    ConfigChanged,
}
```

### Compaction Snapshot

Created before each compaction. Contains enough information for the LLM to
continue working effectively without re-reading everything.

```rust
pub struct CompactionSnapshot {
    pub created_at: DateTime<Utc>,
    /// What the user is trying to accomplish
    pub user_goals: Vec<String>,
    /// Human-readable session state summary
    pub summary: String,
    /// Files actively being edited (not just read)
    pub working_set: Vec<String>,
    /// Files that were read for reference
    pub reference_files: Vec<String>,
    /// Unfinished work items
    pub pending_tasks: Vec<String>,
    /// Important decisions made and their rationale
    pub key_decisions: Vec<String>,
    /// Approaches that failed and why (avoid repeating mistakes)
    pub error_history: Vec<ErrorRecord>,
    /// Git state: branch, commits made during session
    pub git_state: Option<GitState>,
}

pub struct ErrorRecord {
    pub description: String,
    pub reason: String,
}

pub struct GitState {
    pub branch: String,
    pub commits_made: Vec<String>,  // commit message summaries
}
```

### Lifecycle

1. **During session:** `SessionContinuity::capture_event()` records significant
   events after each tool call. Events are classified by type automatically:
   - `after_file_write` / `after_apply_patch` -> `FileChanged`
   - `after_run_cmd` with test/build commands -> `TestResult`
   - `tool_error` -> `ErrorEncountered`
   - `user_message_received` (first message or goal refinement) -> `GoalEstablished`

2. **Before compaction:** `create_snapshot()` builds a `CompactionSnapshot` from
   the event log. The snapshot distills the event log into structured fields.

3. **Persistence:** Snapshot saved to `{session_dir}/continuity.json`. Event log
   saved to `{session_dir}/continuity_events.json`.

4. **On session resume:** Snapshot loaded and injected as a system message prefix:

```
[Session restored from compaction snapshot]
Goals: {user_goals}
Working on: {working_set}
Key decisions: {key_decisions}
Failed approaches: {error_history}
Git branch: {branch}, commits: {commits_made}
Pending: {pending_tasks}
```

## Data Flow

```
User message -> [existing transcript]
    |
    v
ContextPipeline::transform()
    |-- DedupStrategy::apply()
    |-- SupersedeStrategy::apply()
    |-- PurgeErrorsStrategy::apply()
    '-- SandboxInterceptor::apply()  (stores to KB)
    |
    v
PluginHost::dispatch_transform()  (external plugins, if any)
    |
    v
BudgetManager::check()  (preflight)
    |
    v
[If over threshold] -> inject pruning tool instructions in system prompt
    |
    v
Provider::stream_chat()
    |
    v
[If LLM calls context_distill/compress/prune] -> modify messages in-place
[If LLM calls knowledge_search/store] -> query/update KB
    |
    v
SessionContinuity::capture_event()  (after tool calls)
```

## Integration Points

| Existing system      | Integration                                           |
| -------------------- | ----------------------------------------------------- |
| Message model        | Strategies operate on `Vec<Message>` directly         |
| Token budget manager | `StrategyContext` carries `TokenBudget` + `TokenCounter` |
| Tool registry        | `register_context_tools()` adds 5 tools at startup   |
| Session persistence  | KB + continuity files in session directory             |
| Transform pipeline   | Native pipeline is the `"native"` entry in transform order |
| Hook system          | Continuity captures events from hook dispatch          |

## Error Handling

- Strategy errors are logged and skipped (fail-open). A broken dedup strategy
  should not prevent the LLM call.
- Knowledge base errors are logged. Store failures mean content stays in context
  (no data loss). Search failures return empty results.
- Tool handler errors are captured as `ToolResult { is_error: true }` per
  existing convention.
- Continuity persistence errors are logged but do not block session operation.
