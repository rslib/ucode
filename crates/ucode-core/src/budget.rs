use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::CoreError;
use crate::message::{Message, Part, Role};

// ── Budget types ──────────────────────────────────────────────────────────────

/// Budget envelope for a single model request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudget {
    pub max_context: usize,
    pub reserved_output: usize,
}

impl TokenBudget {
    pub fn new(max_context: usize, reserved_output: usize) -> Self {
        Self {
            max_context,
            reserved_output,
        }
    }

    pub fn available_input(&self) -> usize {
        self.max_context.saturating_sub(self.reserved_output)
    }
}

/// How tokens were counted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum CountSource {
    ProviderCount,
    LocalEstimate { safety_margin_pct: u8 },
}

/// Result of a preflight budget check.
#[derive(Debug, Clone)]
pub enum BudgetCheck {
    Fits {
        used: usize,
        available: usize,
        source: CountSource,
    },
    OverBudget {
        used: usize,
        available: usize,
        overage: usize,
        source: CountSource,
    },
}

// ── TokenCounter trait + CharEstimator ───────────────────────────────────────

/// Abstraction for counting tokens in messages.
pub trait TokenCounter: Send + Sync {
    fn count_messages(&self, messages: &[Message]) -> (usize, CountSource);
    fn count_text(&self, text: &str) -> usize;
}

/// Simple character-based token estimator: chars/4 + safety margin.
#[derive(Debug, Clone)]
pub struct CharEstimator {
    pub safety_margin_pct: u8,
}

impl Default for CharEstimator {
    fn default() -> Self {
        Self {
            safety_margin_pct: 10,
        }
    }
}

impl CharEstimator {
    pub fn new(safety_margin_pct: u8) -> Self {
        Self { safety_margin_pct }
    }
}

impl TokenCounter for CharEstimator {
    fn count_text(&self, text: &str) -> usize {
        let base = text.len() / 4 + 1; // +1 avoids zero for short strings
        let margin = base * self.safety_margin_pct as usize / 100;
        base + margin
    }

    fn count_messages(&self, messages: &[Message]) -> (usize, CountSource) {
        let mut total = 0usize;
        for msg in messages {
            // 4 tokens overhead per message (role, formatting)
            total += 4;
            for part in &msg.parts {
                total += match part {
                    Part::Text(t) => self.count_text(t),
                    Part::ToolCall(tc) => {
                        self.count_text(&tc.name) + self.count_text(&tc.args.to_string())
                    }
                    Part::ToolResult(tr) => {
                        self.count_text(&tr.name) + self.count_text(&tr.result.to_string())
                    }
                };
            }
        }
        (
            total,
            CountSource::LocalEstimate {
                safety_margin_pct: self.safety_margin_pct,
            },
        )
    }
}

// ── Compaction types + policy ─────────────────────────────────────────────────

/// Which compaction step was applied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStep {
    TrimToolOutputs,
    CompactOlderTurns,
    DistillLongOutputs,
}

/// Audit record for one compaction step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionRecord {
    pub step: CompactionStep,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub messages_removed: usize,
    pub messages_added: usize,
}

/// Controls compaction behavior.
#[derive(Debug, Clone)]
pub struct CompactionPolicy {
    pub pinned_recent_turns: usize,
    pub max_tool_output_chars: usize,
    pub max_retries: usize,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            pinned_recent_turns: 4,
            max_tool_output_chars: 2000,
            max_retries: 3,
        }
    }
}

// ── Compaction functions ──────────────────────────────────────────────────────

/// Truncate oversized ToolResult payloads in-place.
///
/// Any `Part::ToolResult` whose `result.to_string()` exceeds
/// `policy.max_tool_output_chars` is replaced with a JSON string containing
/// the first 200 chars followed by `" [truncated, was N chars]"`.
fn trim_tool_outputs(
    messages: &mut [Message],
    policy: &CompactionPolicy,
    counter: &dyn TokenCounter,
) -> CompactionRecord {
    let (tokens_before, _) = counter.count_messages(messages);

    for msg in messages.iter_mut() {
        for part in msg.parts.iter_mut() {
            if let Part::ToolResult(tr) = part {
                let serialized = tr.result.to_string();
                if serialized.len() > policy.max_tool_output_chars {
                    let preview: String = serialized.chars().take(200).collect();
                    tr.result = serde_json::Value::String(format!(
                        "{preview} [truncated, was {} chars]",
                        serialized.len()
                    ));
                }
            }
        }
    }

    let (tokens_after, _) = counter.count_messages(messages);
    CompactionRecord {
        step: CompactionStep::TrimToolOutputs,
        tokens_before,
        tokens_after,
        messages_removed: 0,
        messages_added: 0,
    }
}

/// Replace non-pinned, non-system messages with a single compaction placeholder.
///
/// The "pinned window" is the last `policy.pinned_recent_turns` messages.
/// The first message is also preserved when its role is `Role::System`.
fn compact_older_turns(
    messages: &mut Vec<Message>,
    policy: &CompactionPolicy,
    counter: &dyn TokenCounter,
) -> CompactionRecord {
    let (tokens_before, _) = counter.count_messages(messages);
    let original_len = messages.len();

    let has_system_prefix = messages
        .first()
        .map(|m| m.role == Role::System)
        .unwrap_or(false);

    // Index of the first pinned message.
    let pin_start = original_len.saturating_sub(policy.pinned_recent_turns);

    // Collect indices that are neither the system prefix nor in the pinned window.
    let compactable_count = messages[..pin_start]
        .iter()
        .enumerate()
        .filter(|(i, m)| !(has_system_prefix && *i == 0) && m.role != Role::System)
        .count();

    if compactable_count == 0 {
        let (tokens_after, _) = counter.count_messages(messages);
        return CompactionRecord {
            step: CompactionStep::CompactOlderTurns,
            tokens_before,
            tokens_after,
            messages_removed: 0,
            messages_added: 0,
        };
    }

    // Build the replacement: system prefix (if any) + placeholder + pinned window.
    let pinned: Vec<Message> = messages.drain(pin_start..).collect();
    let system_prefix: Option<Message> = if has_system_prefix {
        Some(messages.remove(0))
    } else {
        None
    };

    let placeholder = Message::assistant(format!("[Compacted {compactable_count} earlier turns]"));

    messages.clear();
    if let Some(sys) = system_prefix {
        messages.push(sys);
    }
    messages.push(placeholder);
    messages.extend(pinned);

    let (tokens_after, _) = counter.count_messages(messages);
    CompactionRecord {
        step: CompactionStep::CompactOlderTurns,
        tokens_before,
        tokens_after,
        messages_removed: compactable_count,
        messages_added: 1,
    }
}

/// Aggressively truncate any ToolResult whose serialized length exceeds 500 chars.
///
/// Truncates to the first 100 chars followed by `" [distilled]"`.
fn distill_long_outputs(messages: &mut [Message], counter: &dyn TokenCounter) -> CompactionRecord {
    let (tokens_before, _) = counter.count_messages(messages);

    for msg in messages.iter_mut() {
        for part in msg.parts.iter_mut() {
            if let Part::ToolResult(tr) = part {
                let serialized = tr.result.to_string();
                if serialized.len() > 500 {
                    let preview: String = serialized.chars().take(100).collect();
                    tr.result = serde_json::Value::String(format!("{preview} [distilled]"));
                }
            }
        }
    }

    let (tokens_after, _) = counter.count_messages(messages);
    CompactionRecord {
        step: CompactionStep::DistillLongOutputs,
        tokens_before,
        tokens_after,
        messages_removed: 0,
        messages_added: 0,
    }
}

// ── BudgetManager ─────────────────────────────────────────────────────────────

pub struct BudgetManager {
    pub budget: TokenBudget,
    pub policy: CompactionPolicy,
}

impl BudgetManager {
    pub fn new(budget: TokenBudget, policy: CompactionPolicy) -> Self {
        Self { budget, policy }
    }

    /// Preflight check — does the transcript fit within the available input budget?
    pub fn check(&self, messages: &[Message], counter: &dyn TokenCounter) -> BudgetCheck {
        let available = self.budget.available_input();
        let (used, source) = counter.count_messages(messages);
        if used <= available {
            BudgetCheck::Fits {
                used,
                available,
                source,
            }
        } else {
            BudgetCheck::OverBudget {
                used,
                available,
                overage: used - available,
                source,
            }
        }
    }

    /// Run progressive compaction until the transcript fits or all steps are exhausted.
    ///
    /// Returns the audit trail of applied steps. Returns `CoreError::ContextTooLarge`
    /// when the transcript still exceeds the budget after all three compaction passes.
    pub fn ensure_fits(
        &self,
        messages: &mut Vec<Message>,
        counter: &dyn TokenCounter,
    ) -> Result<Vec<CompactionRecord>, CoreError> {
        let mut records = Vec::new();

        if matches!(self.check(messages, counter), BudgetCheck::Fits { .. }) {
            return Ok(records);
        }

        // Step 1: trim oversized tool outputs
        let rec = trim_tool_outputs(messages, &self.policy, counter);
        tracing::info!(
            step = "trim_tool_outputs",
            before = rec.tokens_before,
            after = rec.tokens_after,
            "compaction step"
        );
        records.push(rec);
        if matches!(self.check(messages, counter), BudgetCheck::Fits { .. }) {
            return Ok(records);
        }

        // Step 2: compact older turns into a placeholder
        let rec = compact_older_turns(messages, &self.policy, counter);
        tracing::info!(
            step = "compact_older_turns",
            before = rec.tokens_before,
            after = rec.tokens_after,
            "compaction step"
        );
        records.push(rec);
        if matches!(self.check(messages, counter), BudgetCheck::Fits { .. }) {
            return Ok(records);
        }

        // Step 3: aggressively distill remaining long outputs
        let rec = distill_long_outputs(messages, counter);
        tracing::info!(
            step = "distill_long_outputs",
            before = rec.tokens_before,
            after = rec.tokens_after,
            "compaction step"
        );
        records.push(rec);
        if matches!(self.check(messages, counter), BudgetCheck::Fits { .. }) {
            return Ok(records);
        }

        let (used, _) = counter.count_messages(messages);
        let available = self.budget.available_input();
        Err(CoreError::ContextTooLarge {
            limit: available,
            actual: used,
        })
    }
}

// ── Cost governance types ────────────────────────────────────────────────────

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
    /// Record a usage entry and update running totals.
    pub fn record(&mut self, rec: UsageRecord) {
        self.total_input_tokens += rec.input_tokens;
        self.total_output_tokens += rec.output_tokens;
        self.total_estimated_cost_usd += rec.estimated_cost_usd;
        self.records.push(rec);
    }

    /// Total tokens (input + output) across all requests.
    pub fn total_tokens(&self) -> usize {
        self.total_input_tokens + self.total_output_tokens
    }
}

/// Configurable soft/hard budget limits for cost governance.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostBudget {
    /// Soft limit in USD — triggers warning.
    #[serde(default)]
    pub soft_limit_usd: Option<f64>,
    /// Hard limit in USD — triggers block.
    #[serde(default)]
    pub hard_limit_usd: Option<f64>,
    /// Soft limit in total tokens — triggers warning.
    #[serde(default)]
    pub soft_limit_tokens: Option<usize>,
    /// Hard limit in total tokens — triggers block.
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
        if let Some(hard_usd) = self.budget.hard_limit_usd
            && usage.total_estimated_cost_usd >= hard_usd
        {
            return BudgetAction::Block {
                message: format!(
                    "Hard cost limit reached: ${:.4} >= ${:.4}",
                    usage.total_estimated_cost_usd, hard_usd
                ),
            };
        }
        if let Some(hard_tokens) = self.budget.hard_limit_tokens
            && usage.total_tokens() >= hard_tokens
        {
            return BudgetAction::Block {
                message: format!(
                    "Hard token limit reached: {} >= {}",
                    usage.total_tokens(),
                    hard_tokens
                ),
            };
        }

        // Check soft limits
        if let Some(soft_usd) = self.budget.soft_limit_usd
            && usage.total_estimated_cost_usd >= soft_usd
        {
            return BudgetAction::Warn {
                message: format!(
                    "Soft cost limit reached: ${:.4} >= ${:.4}",
                    usage.total_estimated_cost_usd, soft_usd
                ),
            };
        }
        if let Some(soft_tokens) = self.budget.soft_limit_tokens
            && usage.total_tokens() >= soft_tokens
        {
            return BudgetAction::Warn {
                message: format!(
                    "Soft token limit reached: {} >= {}",
                    usage.total_tokens(),
                    soft_tokens
                ),
            };
        }

        BudgetAction::Allow
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ──────────────────────────────────────────────────────────────

    fn make_user_msg(text: &str) -> Message {
        Message::user(text)
    }

    fn make_tool_result(size: usize) -> Message {
        let big_result = "x".repeat(size);
        Message::tool_result("id", "tool", serde_json::Value::String(big_result), false)
    }

    fn estimator() -> CharEstimator {
        CharEstimator::default()
    }

    // ── TokenBudget ───────────────────────────────────────────────────────────

    #[test]
    fn token_budget_available_input() {
        let b = TokenBudget::new(128_000, 4_096);
        assert_eq!(b.available_input(), 123_904);
    }

    #[test]
    fn token_budget_saturating() {
        let b = TokenBudget::new(1_000, 5_000);
        assert_eq!(b.available_input(), 0);
    }

    // ── CharEstimator ─────────────────────────────────────────────────────────

    #[test]
    fn char_estimator_count_text() {
        let est = estimator();
        let count = est.count_text("hello world"); // 11 chars
        assert!(count > 0, "count must be positive");
        // base = 11/4+1 = 3, margin = 3*10/100 = 0, total = 3
        assert_eq!(count, 3);
    }

    #[test]
    fn char_estimator_count_messages() {
        let est = estimator();
        let msgs = vec![
            make_user_msg("Hello there"),
            Message::assistant("Hi! How can I help?"),
            make_user_msg("What is 2+2?"),
        ];
        let (count, source) = est.count_messages(&msgs);
        assert!(count > 0);
        assert!(matches!(
            source,
            CountSource::LocalEstimate {
                safety_margin_pct: 10
            }
        ));
    }

    // ── BudgetCheck ───────────────────────────────────────────────────────────

    #[test]
    fn budget_check_fits() {
        let budget = TokenBudget::new(128_000, 4_096);
        let manager = BudgetManager::new(budget, CompactionPolicy::default());
        let msgs = vec![make_user_msg("short message")];
        let check = manager.check(&msgs, &estimator());
        assert!(matches!(check, BudgetCheck::Fits { .. }));
    }

    #[test]
    fn budget_check_over_budget() {
        // Tiny budget: 10 tokens available
        let budget = TokenBudget::new(20, 10);
        let manager = BudgetManager::new(budget, CompactionPolicy::default());
        // A message that will definitely exceed 10 tokens
        let msgs = vec![make_user_msg(
            "this is a fairly long message that exceeds the budget",
        )];
        let check = manager.check(&msgs, &estimator());
        match check {
            BudgetCheck::OverBudget {
                used,
                available,
                overage,
                ..
            } => {
                assert_eq!(available, 10);
                assert_eq!(overage, used - available);
            }
            BudgetCheck::Fits { .. } => panic!("expected OverBudget"),
        }
    }

    // ── CompactionPolicy ──────────────────────────────────────────────────────

    #[test]
    fn compaction_policy_defaults() {
        let p = CompactionPolicy::default();
        assert_eq!(p.pinned_recent_turns, 4);
        assert_eq!(p.max_tool_output_chars, 2000);
        assert_eq!(p.max_retries, 3);
    }

    // ── CompactionRecord serde ────────────────────────────────────────────────

    #[test]
    fn compaction_record_serde_roundtrip() {
        let rec = CompactionRecord {
            step: CompactionStep::TrimToolOutputs,
            tokens_before: 1000,
            tokens_after: 800,
            messages_removed: 0,
            messages_added: 0,
        };
        let json = serde_json::to_string(&rec).unwrap();
        let back: CompactionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.step, CompactionStep::TrimToolOutputs);
        assert_eq!(back.tokens_before, 1000);
        assert_eq!(back.tokens_after, 800);
    }

    // ── trim_tool_outputs ─────────────────────────────────────────────────────

    #[test]
    fn trim_tool_outputs_truncates_long() {
        let policy = CompactionPolicy::default(); // max_tool_output_chars = 2000
        let est = estimator();
        let mut msgs = vec![make_tool_result(5000)];

        let rec = trim_tool_outputs(&mut msgs, &policy, &est);

        assert_eq!(rec.step, CompactionStep::TrimToolOutputs);
        assert!(rec.tokens_after < rec.tokens_before);

        // Verify the result was actually truncated
        if let Part::ToolResult(tr) = &msgs[0].parts[0] {
            let s = tr.result.as_str().unwrap();
            assert!(s.contains("[truncated, was 5002 chars]"), "got: {s}");
            // 5000 x's + surrounding JSON quotes = 5002 chars in to_string()
        } else {
            panic!("expected ToolResult part");
        }
    }

    #[test]
    fn trim_tool_outputs_preserves_short() {
        let policy = CompactionPolicy::default();
        let est = estimator();
        let mut msgs = vec![make_tool_result(100)];
        let original_result = if let Part::ToolResult(tr) = &msgs[0].parts[0] {
            tr.result.clone()
        } else {
            panic!("expected ToolResult");
        };

        trim_tool_outputs(&mut msgs, &policy, &est);

        if let Part::ToolResult(tr) = &msgs[0].parts[0] {
            assert_eq!(tr.result, original_result);
        }
    }

    // ── compact_older_turns ───────────────────────────────────────────────────

    #[test]
    fn compact_older_turns_preserves_pinned() {
        let policy = CompactionPolicy {
            pinned_recent_turns: 4,
            ..Default::default()
        };
        let est = estimator();

        let mut msgs: Vec<Message> = (0..10)
            .map(|i| make_user_msg(&format!("message {i}")))
            .collect();

        // Capture the last 4 messages before compaction
        let pinned_before: Vec<Message> = msgs[6..].to_vec();

        let rec = compact_older_turns(&mut msgs, &policy, &est);

        assert_eq!(rec.step, CompactionStep::CompactOlderTurns);
        assert!(rec.messages_removed > 0);

        // Last 4 messages must be identical
        let pinned_after: Vec<Message> = msgs[msgs.len() - 4..].to_vec();
        assert_eq!(pinned_before, pinned_after);
    }

    #[test]
    fn compact_older_turns_preserves_system() {
        let policy = CompactionPolicy {
            pinned_recent_turns: 2,
            ..Default::default()
        };
        let est = estimator();

        let mut msgs = vec![
            Message::system("You are a helpful assistant."),
            make_user_msg("turn 1"),
            Message::assistant("response 1"),
            make_user_msg("turn 2"),
            Message::assistant("response 2"),
            make_user_msg("turn 3"),
        ];

        compact_older_turns(&mut msgs, &policy, &est);

        // System message must still be first
        assert_eq!(msgs[0].role, Role::System);
        if let Part::Text(t) = &msgs[0].parts[0] {
            assert_eq!(t, "You are a helpful assistant.");
        }
    }

    // ── distill_long_outputs ──────────────────────────────────────────────────

    #[test]
    fn distill_long_outputs_aggressive() {
        let est = estimator();
        let mut msgs = vec![make_tool_result(1000)];

        let rec = distill_long_outputs(&mut msgs, &est);

        assert_eq!(rec.step, CompactionStep::DistillLongOutputs);
        assert!(rec.tokens_after < rec.tokens_before);

        if let Part::ToolResult(tr) = &msgs[0].parts[0] {
            let s = tr.result.as_str().unwrap();
            assert!(s.ends_with("[distilled]"), "got: {s}");
        }
    }

    // ── ensure_fits ───────────────────────────────────────────────────────────

    #[test]
    fn ensure_fits_already_within_budget() {
        let budget = TokenBudget::new(128_000, 4_096);
        let manager = BudgetManager::new(budget, CompactionPolicy::default());
        let mut msgs = vec![make_user_msg("hello")];

        let records = manager.ensure_fits(&mut msgs, &estimator()).unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn ensure_fits_trim_sufficient() {
        // Budget tight enough that trim alone fixes it.
        // One large tool result + a few small messages.
        let est = estimator();

        // Measure a small transcript to set budget just above it
        let small_msgs = vec![make_user_msg("hi"), make_tool_result(100)];
        let (small_tokens, _) = est.count_messages(&small_msgs);

        // Large tool result that will be trimmed down to fit
        let mut msgs = vec![make_user_msg("hi"), make_tool_result(5000)];
        let (large_tokens, _) = est.count_messages(&msgs);

        // Budget: fits small but not large
        let available = small_tokens + 50;
        let manager =
            BudgetManager::new(TokenBudget::new(available, 0), CompactionPolicy::default());

        assert!(
            large_tokens > available,
            "pre-condition: must be over budget"
        );

        let records = manager.ensure_fits(&mut msgs, &est).unwrap();
        // At least trim step was applied
        assert!(!records.is_empty());
        assert_eq!(records[0].step, CompactionStep::TrimToolOutputs);
    }

    #[test]
    fn ensure_fits_needs_all_steps() {
        // Extremely tight budget forces all three steps.
        let budget = TokenBudget::new(50, 0);
        let policy = CompactionPolicy {
            pinned_recent_turns: 1,
            ..Default::default()
        };
        let manager = BudgetManager::new(budget, policy);
        let est = estimator();

        let mut msgs: Vec<Message> = (0..8).map(|i| make_tool_result(3000 + i * 100)).collect();

        // May succeed or fail depending on final size; we just verify all steps ran
        let result = manager.ensure_fits(&mut msgs, &est);
        match result {
            Ok(records) => {
                let steps: Vec<_> = records.iter().map(|r| &r.step).collect();
                assert!(steps.contains(&&CompactionStep::TrimToolOutputs));
                assert!(steps.contains(&&CompactionStep::CompactOlderTurns));
                assert!(steps.contains(&&CompactionStep::DistillLongOutputs));
            }
            Err(CoreError::ContextTooLarge { .. }) => {
                // Also acceptable — all steps were tried
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn ensure_fits_terminal_error() {
        // Budget so small nothing can fit.
        let budget = TokenBudget::new(5, 0);
        let manager = BudgetManager::new(budget, CompactionPolicy::default());
        let est = estimator();

        let mut msgs = vec![
            make_user_msg("even this tiny message exceeds 5 tokens"),
            make_tool_result(500),
        ];

        let err = manager.ensure_fits(&mut msgs, &est).unwrap_err();
        assert!(matches!(err, CoreError::ContextTooLarge { .. }));
    }

    #[test]
    fn ensure_fits_preserves_recent_turns() {
        // Budget forces compaction but the last N turns must survive intact.
        let policy = CompactionPolicy {
            pinned_recent_turns: 3,
            ..Default::default()
        };
        let est = estimator();

        // Build a transcript: many large tool results + 3 small recent turns
        let mut msgs: Vec<Message> = (0..6).map(|_| make_tool_result(3000)).collect();
        let recent = vec![
            make_user_msg("recent turn 1"),
            Message::assistant("recent response 1"),
            make_user_msg("recent turn 2"),
        ];
        msgs.extend(recent.clone());

        // Budget: fits the 3 recent turns but not the whole transcript
        let (recent_tokens, _) = est.count_messages(&recent);
        let budget = TokenBudget::new(recent_tokens + 200, 0);
        let manager = BudgetManager::new(budget, policy);

        manager.ensure_fits(&mut msgs, &est).unwrap();

        // Last 3 messages must be the recent turns
        let tail: Vec<Message> = msgs[msgs.len() - 3..].to_vec();
        assert_eq!(tail, recent);
    }

    // ── UsageRecord / SessionUsage ────────────────────────────────────────────

    fn make_usage_record(input: usize, output: usize, cost: f64) -> UsageRecord {
        UsageRecord {
            timestamp: Utc::now(),
            model: "test-model".into(),
            input_tokens: input,
            output_tokens: output,
            estimated_cost_usd: cost,
        }
    }

    #[test]
    fn usage_record_serde_roundtrip() {
        let rec = make_usage_record(100, 50, 0.003);
        let json = serde_json::to_string(&rec).unwrap();
        let back: UsageRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back.input_tokens, 100);
        assert_eq!(back.output_tokens, 50);
        assert!((back.estimated_cost_usd - 0.003).abs() < f64::EPSILON);
    }

    #[test]
    fn session_usage_accumulates() {
        let mut usage = SessionUsage::default();
        usage.record(make_usage_record(100, 50, 0.003));
        usage.record(make_usage_record(200, 100, 0.006));

        assert_eq!(usage.total_input_tokens, 300);
        assert_eq!(usage.total_output_tokens, 150);
        assert!((usage.total_estimated_cost_usd - 0.009).abs() < 1e-10);
        assert_eq!(usage.records.len(), 2);
    }

    #[test]
    fn session_usage_total_tokens() {
        let mut usage = SessionUsage::default();
        usage.record(make_usage_record(100, 50, 0.0));
        assert_eq!(usage.total_tokens(), 150);
    }

    // ── CostBudget / CostGovernor ─────────────────────────────────────────────

    #[test]
    fn cost_budget_default_is_empty() {
        let b = CostBudget::default();
        assert!(b.soft_limit_usd.is_none());
        assert!(b.hard_limit_usd.is_none());
        assert!(b.soft_limit_tokens.is_none());
        assert!(b.hard_limit_tokens.is_none());
    }

    #[test]
    fn cost_governor_allow_when_no_limits() {
        let gov = CostGovernor::new(CostBudget::default());
        let usage = SessionUsage::default();
        assert_eq!(gov.check(&usage), BudgetAction::Allow);
    }

    #[test]
    fn cost_governor_soft_usd_warns() {
        let budget = CostBudget {
            soft_limit_usd: Some(0.01),
            ..Default::default()
        };
        let gov = CostGovernor::new(budget);
        let mut usage = SessionUsage::default();
        usage.record(make_usage_record(1000, 500, 0.015));

        match gov.check(&usage) {
            BudgetAction::Warn { message } => {
                assert!(message.contains("Soft cost limit"));
            }
            other => panic!("expected Warn, got {other:?}"),
        }
    }

    #[test]
    fn cost_governor_hard_usd_blocks() {
        let budget = CostBudget {
            hard_limit_usd: Some(0.01),
            ..Default::default()
        };
        let gov = CostGovernor::new(budget);
        let mut usage = SessionUsage::default();
        usage.record(make_usage_record(1000, 500, 0.015));

        match gov.check(&usage) {
            BudgetAction::Block { message } => {
                assert!(message.contains("Hard cost limit"));
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn cost_governor_soft_tokens_warns() {
        let budget = CostBudget {
            soft_limit_tokens: Some(1000),
            ..Default::default()
        };
        let gov = CostGovernor::new(budget);
        let mut usage = SessionUsage::default();
        usage.record(make_usage_record(800, 300, 0.0));

        match gov.check(&usage) {
            BudgetAction::Warn { message } => {
                assert!(message.contains("Soft token limit"));
            }
            other => panic!("expected Warn, got {other:?}"),
        }
    }

    #[test]
    fn cost_governor_hard_tokens_blocks() {
        let budget = CostBudget {
            hard_limit_tokens: Some(1000),
            ..Default::default()
        };
        let gov = CostGovernor::new(budget);
        let mut usage = SessionUsage::default();
        usage.record(make_usage_record(800, 300, 0.0));

        match gov.check(&usage) {
            BudgetAction::Block { message } => {
                assert!(message.contains("Hard token limit"));
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn cost_governor_hard_takes_precedence_over_soft() {
        let budget = CostBudget {
            soft_limit_usd: Some(0.005),
            hard_limit_usd: Some(0.01),
            ..Default::default()
        };
        let gov = CostGovernor::new(budget);
        let mut usage = SessionUsage::default();
        // Exceeds both soft and hard
        usage.record(make_usage_record(1000, 500, 0.015));

        // Hard should win
        match gov.check(&usage) {
            BudgetAction::Block { .. } => {}
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn cost_governor_below_soft_allows() {
        let budget = CostBudget {
            soft_limit_usd: Some(1.0),
            hard_limit_usd: Some(5.0),
            ..Default::default()
        };
        let gov = CostGovernor::new(budget);
        let mut usage = SessionUsage::default();
        usage.record(make_usage_record(100, 50, 0.001));

        assert_eq!(gov.check(&usage), BudgetAction::Allow);
    }
}
