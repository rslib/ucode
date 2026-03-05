# Token/Cost Governance (ISSUE 0111) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add per-request and per-session token/cost tracking with configurable soft/hard budget limits and policy actions (warn, downgrade, block).

**Architecture:** Extend `budget.rs` with `UsageRecord` (per-request), `SessionUsage` (accumulator), `CostBudget` (soft/hard limits), `BudgetAction` (policy response), and `CostGovernor` (evaluator). Add `SessionUsage` to `Session` for persistence. Add `CoreError::BudgetExceeded` for hard-limit enforcement.

**Tech Stack:** Rust, serde, chrono

---

### Task 1: Add UsageRecord and SessionUsage types

**Files:**
- Modify: `crates/ucode-core/src/budget.rs`

**Types:**

```rust
/// Token and cost usage for a single model request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub timestamp: DateTime<Utc>,
    pub model: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub estimated_cost_usd: f64,
}

/// Accumulated usage across a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionUsage {
    pub total_input_tokens: usize,
    pub total_output_tokens: usize,
    pub total_estimated_cost_usd: f64,
    pub records: Vec<UsageRecord>,
}

impl SessionUsage {
    pub fn record(&mut self, rec: UsageRecord) {
        self.total_input_tokens += rec.input_tokens;
        self.total_output_tokens += rec.output_tokens;
        self.total_estimated_cost_usd += rec.estimated_cost_usd;
        self.records.push(rec);
    }

    pub fn total_tokens(&self) -> usize {
        self.total_input_tokens + self.total_output_tokens
    }
}
```

Needs `use chrono::{DateTime, Utc};` import at top of budget.rs.

---

### Task 2: Add CostBudget, BudgetAction, CostGovernor

**Files:**
- Modify: `crates/ucode-core/src/budget.rs`

**Types:**

```rust
/// Configurable soft/hard budget limits.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostBudget {
    /// Soft limit in USD — triggers warning.
    #[serde(default)]
    pub soft_limit_usd: Option<f64>,
    /// Hard limit in USD — triggers block/downgrade.
    #[serde(default)]
    pub hard_limit_usd: Option<f64>,
    /// Soft limit in total tokens — triggers warning.
    #[serde(default)]
    pub soft_limit_tokens: Option<usize>,
    /// Hard limit in total tokens — triggers block/downgrade.
    #[serde(default)]
    pub hard_limit_tokens: Option<usize>,
}

/// Policy action returned by the cost governor.
#[derive(Debug, Clone, PartialEq)]
pub enum BudgetAction {
    /// Within budget, proceed normally.
    Allow,
    /// Soft limit reached — warn but continue.
    Warn { message: String },
    /// Hard limit reached — block the request.
    Block { message: String },
}

/// Evaluates session usage against a cost budget.
pub struct CostGovernor {
    pub budget: CostBudget,
}

impl CostGovernor {
    pub fn new(budget: CostBudget) -> Self {
        Self { budget }
    }

    /// Check current usage against budget limits.
    /// Hard limits take precedence over soft limits.
    pub fn check(&self, usage: &SessionUsage) -> BudgetAction {
        // Check hard limits first
        if let Some(hard_usd) = self.budget.hard_limit_usd {
            if usage.total_estimated_cost_usd >= hard_usd {
                return BudgetAction::Block {
                    message: format!(
                        "Hard cost limit reached: ${:.4} >= ${:.4}",
                        usage.total_estimated_cost_usd, hard_usd
                    ),
                };
            }
        }
        if let Some(hard_tokens) = self.budget.hard_limit_tokens {
            if usage.total_tokens() >= hard_tokens {
                return BudgetAction::Block {
                    message: format!(
                        "Hard token limit reached: {} >= {}",
                        usage.total_tokens(),
                        hard_tokens
                    ),
                };
            }
        }

        // Check soft limits
        if let Some(soft_usd) = self.budget.soft_limit_usd {
            if usage.total_estimated_cost_usd >= soft_usd {
                return BudgetAction::Warn {
                    message: format!(
                        "Soft cost limit reached: ${:.4} >= ${:.4}",
                        usage.total_estimated_cost_usd, soft_usd
                    ),
                };
            }
        }
        if let Some(soft_tokens) = self.budget.soft_limit_tokens {
            if usage.total_tokens() >= soft_tokens {
                return BudgetAction::Warn {
                    message: format!(
                        "Soft token limit reached: {} >= {}",
                        usage.total_tokens(),
                        soft_tokens
                    ),
                };
            }
        }

        BudgetAction::Allow
    }
}
```

---

### Task 3: Add SessionUsage to Session

**Files:**
- Modify: `crates/ucode-core/src/session.rs`

Add field to Session struct:
```rust
    /// Accumulated token/cost usage for this session.
    #[serde(default)]
    pub usage: SessionUsage,
```

Add import: `use crate::budget::{SessionUsage, UsageRecord};`

Add method to Session impl:
```rust
    /// Record a usage entry and update totals.
    pub fn record_usage(&mut self, record: UsageRecord) {
        self.usage.record(record);
        self.meta.updated_at = Utc::now();
    }
```

Initialize in Session::new(): `usage: SessionUsage::default()`

Initialize in Session::fork(): `usage: SessionUsage::default()` (forked sessions start fresh usage)

---

### Task 4: Add CoreError::BudgetExceeded

**Files:**
- Modify: `crates/ucode-core/src/error.rs`

```rust
    #[error("budget exceeded: {message}")]
    BudgetExceeded { message: String },
```

---

### Task 5: Tests

**Unit tests in budget.rs:**
- usage_record_serde_roundtrip
- session_usage_accumulates
- session_usage_total_tokens
- cost_budget_default_is_empty
- cost_governor_allow_when_no_limits
- cost_governor_soft_usd_warns
- cost_governor_hard_usd_blocks
- cost_governor_soft_tokens_warns
- cost_governor_hard_tokens_blocks
- cost_governor_hard_takes_precedence_over_soft

**Integration test in session_tests.rs:**
- session_usage_persists_roundtrip

**Backward compat in session.rs tests:**
- Add `assert!(session.usage.records.is_empty());` to existing backward_compat test

---

### Task 6: Update lib.rs re-exports

Add to the budget re-export line:
```rust
pub use budget::{
    BudgetAction, BudgetCheck, BudgetManager, CharEstimator, CompactionPolicy, CompactionRecord,
    CompactionStep, CostBudget, CostGovernor, CountSource, SessionUsage, TokenBudget,
    TokenCounter, UsageRecord,
};
```

---

### Task 7: Full workspace verification + commit

```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Commit: `feat(budget): add token/cost governance with soft/hard limits (ISSUE 0111)`
Mark EPIC.md and PLANS.md [DONE].
