# Token Budget Manager + Context Compaction Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep conversations reliable when nearing model context limits by applying deterministic compaction before hard failure.

**Architecture:** A `BudgetManager` in ucode-core checks message token counts against a per-model budget envelope, then applies a progressive compaction pipeline (trim tool outputs -> compact older turns -> distill long outputs) until the transcript fits. All compaction steps are auditable via `CompactionRecord` stored in the session.

**Tech Stack:** Rust, serde, tracing, existing ucode-core types (Message, Session, Event, CoreError)

---

### Task 1: Budget types + TokenCounter trait + CharEstimator

**Files:**
- Create: `crates/ucode-core/src/budget.rs`
- Modify: `crates/ucode-core/src/lib.rs` (add `pub mod budget` + re-exports)

**Types to create in `budget.rs`:**

```rust
// TokenBudget — envelope for a single model request
pub struct TokenBudget {
    pub max_context: usize,
    pub reserved_output: usize,
}
// Methods: new(), available_input()

// CountSource — how tokens were counted
pub enum CountSource {
    ProviderCount,
    LocalEstimate { safety_margin_pct: u8 },
}

// BudgetCheck — result of a preflight check
pub enum BudgetCheck {
    Fits { used: usize, available: usize, source: CountSource },
    OverBudget { used: usize, available: usize, overage: usize, source: CountSource },
}

// TokenCounter trait — abstraction for counting
pub trait TokenCounter: Send + Sync {
    fn count_messages(&self, messages: &[Message]) -> (usize, CountSource);
    fn count_text(&self, text: &str) -> usize;
}

// CharEstimator — simple chars/4 heuristic with safety margin
pub struct CharEstimator {
    pub safety_margin_pct: u8,
}
// Default safety_margin_pct = 10
// count_text: (text.len() / 4) then apply margin
// count_messages: sum of per-message counts + 4 tokens overhead per message
```

**Tests (in budget.rs #[cfg(test)]):**
- `token_budget_available_input` — 128k context, 4k reserved → 124k available
- `char_estimator_count_text` — known string → expected count with margin
- `char_estimator_count_messages` — vec of messages → expected total
- `budget_check_fits` — small transcript within budget
- `budget_check_over_budget` — large transcript exceeds budget

**Verify:** `cargo test -p ucode-core -- budget`

---

### Task 2: Compaction types + policy

**Files:**
- Modify: `crates/ucode-core/src/budget.rs` (append)

**Types to add:**

```rust
// CompactionStep — which step was applied
pub enum CompactionStep {
    TrimToolOutputs,
    CompactOlderTurns,
    DistillLongOutputs,
}

// CompactionRecord — audit record for one step
pub struct CompactionRecord {
    pub step: CompactionStep,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub messages_removed: usize,
    pub messages_added: usize,
}

// CompactionPolicy — controls compaction behavior
pub struct CompactionPolicy {
    pub pinned_recent_turns: usize,     // default: 4
    pub max_tool_output_chars: usize,   // default: 2000
    pub max_retries: usize,             // default: 3
}
```

All types: `Debug, Clone, Serialize, Deserialize`.

**Tests:**
- `compaction_policy_defaults` — verify default values
- `compaction_record_serde_roundtrip` — serialize/deserialize

**Verify:** `cargo test -p ucode-core -- budget`

---

### Task 3: Compaction functions

**Files:**
- Modify: `crates/ucode-core/src/budget.rs` (append)

**Functions (module-private, called by BudgetManager):**

```rust
// trim_tool_outputs: find ToolResult parts with result JSON > max_chars,
// replace result with truncated version + "[truncated]" marker.
// Returns CompactionRecord.
fn trim_tool_outputs(messages: &mut Vec<Message>, policy: &CompactionPolicy, counter: &dyn TokenCounter) -> CompactionRecord;

// compact_older_turns: replace messages outside pinned window (excluding
// system messages at index 0) with a single assistant summary message
// "[Compacted N earlier turns]". Returns CompactionRecord.
fn compact_older_turns(messages: &mut Vec<Message>, policy: &CompactionPolicy, counter: &dyn TokenCounter) -> CompactionRecord;

// distill_long_outputs: more aggressive truncation — any ToolResult > 500 chars
// gets truncated. Returns CompactionRecord.
fn distill_long_outputs(messages: &mut Vec<Message>, counter: &dyn TokenCounter) -> CompactionRecord;
```

**Tests:**
- `trim_tool_outputs_truncates_long_results` — message with 5000-char tool result → truncated
- `trim_tool_outputs_preserves_short_results` — message with 100-char tool result → unchanged
- `compact_older_turns_preserves_pinned` — 10 messages, pinned=4 → first 6 compacted, last 4 intact
- `compact_older_turns_preserves_system` — system message at index 0 survives compaction
- `distill_long_outputs_aggressive` — remaining long outputs get truncated

**Verify:** `cargo test -p ucode-core -- budget`

---

### Task 4: BudgetManager with check-compact-retry loop

**Files:**
- Modify: `crates/ucode-core/src/budget.rs` (append)

```rust
pub struct BudgetManager {
    pub budget: TokenBudget,
    pub policy: CompactionPolicy,
}

impl BudgetManager {
    pub fn new(budget: TokenBudget, policy: CompactionPolicy) -> Self;

    /// Preflight check: does the transcript fit?
    pub fn check(&self, messages: &[Message], counter: &dyn TokenCounter) -> BudgetCheck;

    /// Run progressive compaction pipeline. Returns records of all steps applied.
    /// Errors with ContextTooLarge if compaction cannot bring transcript within budget.
    pub fn ensure_fits(
        &self,
        messages: &mut Vec<Message>,
        counter: &dyn TokenCounter,
    ) -> Result<Vec<CompactionRecord>, CoreError>;
}
```

`ensure_fits` logic:
1. check() — if Fits, return Ok(vec![])
2. Step 1: trim_tool_outputs → recheck
3. Step 2: compact_older_turns → recheck
4. Step 3: distill_long_outputs → recheck
5. If still over → Err(CoreError::ContextTooLarge)
6. Emit tracing::info! for each step applied

**Tests:**
- `ensure_fits_already_within_budget` — no compaction needed
- `ensure_fits_trim_sufficient` — trim alone brings within budget
- `ensure_fits_needs_compact` — trim not enough, compact needed
- `ensure_fits_needs_all_steps` — all three steps needed
- `ensure_fits_terminal_error` — even after all steps, still over → ContextTooLarge error
- `ensure_fits_preserves_recent_turns` — pinned turns survive all compaction

**Verify:** `cargo test -p ucode-core -- budget`

---

### Task 5: Session + Event integration

**Files:**
- Modify: `crates/ucode-core/src/session.rs` — add `compaction_log` field
- Modify: `crates/ucode-core/src/event.rs` — add `Compaction` variant
- Modify: `crates/ucode-core/src/lib.rs` — update re-exports

**Session changes:**
```rust
pub struct Session {
    pub meta: SessionMeta,
    pub transcript: Vec<Message>,
    pub tool_audit: Vec<ToolAuditEntry>,
    #[serde(default)]  // backward compat
    pub compaction_log: Vec<CompactionRecord>,
}
// Add method: record_compaction(&mut self, records: Vec<CompactionRecord>)
```

**Event changes:**
```rust
pub enum Event {
    // ... existing variants ...
    Compaction(CompactionRecord),
}
```

**Tests:**
- `session_compaction_log_persists` — save/load session with compaction records
- `session_backward_compat` — load old session JSON without compaction_log field
- `compaction_event_serde` — Event::Compaction roundtrip

**Verify:** `cargo test -p ucode-core`

---

### Task 6: Integration test

**Files:**
- Create: `crates/ucode-core/tests/budget_tests.rs`

**Tests:**
- `oversized_transcript_compacts_and_fits` — build 100-message transcript that exceeds a small budget, run ensure_fits, verify it now fits and compaction records are non-empty
- `pinned_turns_preserved_after_compaction` — verify last N turns are identical after compaction
- `compaction_records_are_auditable` — verify records have correct step types and token counts

**Verify:** `cargo test -p ucode-core -- budget`

---

### Task 7: Workspace verification + commit + mark docs

- `cargo fmt --all -- --check`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
- Commit with message referencing ISSUE 0108
- Mark EPIC.md and PLANS.md with [DONE]
