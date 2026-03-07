use ucode_core::{Message, Part};

use crate::strategy::{ContextStrategy, StrategyContext, StrategyResult};

/// Removes error tool results that are older than `after_turns` turns from the end.
pub struct PurgeErrorsStrategy {
    pub after_turns: usize,
}

impl PurgeErrorsStrategy {
    pub fn new(after_turns: usize) -> Self {
        Self { after_turns }
    }
}

impl ContextStrategy for PurgeErrorsStrategy {
    fn name(&self) -> &str {
        "purge_errors"
    }

    fn apply(&self, messages: &mut Vec<Message>, _ctx: &StrategyContext) -> StrategyResult {
        let total = messages.len();
        let mut messages_modified = 0usize;
        let mut chars_saved = 0usize;

        // Collect (msg_idx, part_idx, tool_id) for error ToolResults that are old enough.
        let mut to_purge: Vec<(usize, usize, String)> = Vec::new();

        for (msg_idx, msg) in messages.iter().enumerate() {
            // Distance from end: 0 = last message, 1 = second-to-last, etc.
            let distance = total - 1 - msg_idx;
            if distance <= self.after_turns {
                continue;
            }
            for (part_idx, part) in msg.parts.iter().enumerate() {
                if let Part::ToolResult(tr) = part
                    && tr.is_error
                {
                    to_purge.push((msg_idx, part_idx, tr.id.clone()));
                }
            }
        }

        // Collect tool_ids to purge so we can also blank the matching ToolCall args.
        let purge_ids: Vec<String> = to_purge.iter().map(|(_, _, id)| id.clone()).collect();

        // Replace error ToolResult content.
        for (msg_idx, part_idx, _) in &to_purge {
            let distance = total - 1 - msg_idx;
            let placeholder = format!("[error purged after {distance} turns]");
            if let Part::ToolResult(tr) = &mut messages[*msg_idx].parts[*part_idx] {
                let old_len = tr.result.to_string().len();
                chars_saved += old_len.saturating_sub(placeholder.len());
                tr.result = serde_json::Value::String(placeholder);
                messages_modified += 1;
            }
        }

        // Replace matching ToolCall args anywhere in the message list.
        for msg in messages.iter_mut() {
            for part in msg.parts.iter_mut() {
                if let Part::ToolCall(tc) = part
                    && purge_ids.contains(&tc.id)
                {
                    tc.args = serde_json::json!({});
                }
            }
        }

        StrategyResult {
            messages_modified,
            chars_saved,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use ucode_core::{CharEstimator, Part, Role, TokenBudget, ToolCall, ToolResult};

    use super::*;
    use crate::strategy::StrategyContext;

    fn ctx<'a>(budget: &'a TokenBudget, counter: &'a CharEstimator) -> StrategyContext<'a> {
        StrategyContext {
            session_id: "test",
            turn_count: 0,
            token_budget: budget,
            counter,
        }
    }

    fn error_msg(id: &str) -> Message {
        Message::tool_result(
            id,
            "run_cmd",
            serde_json::json!({ "stderr": "command not found", "exit_code": 127 }),
            true,
        )
    }

    fn ok_msg(id: &str) -> Message {
        Message::tool_result(id, "run_cmd", serde_json::json!({ "stdout": "ok" }), false)
    }

    fn tool_result_value(msg: &Message) -> &serde_json::Value {
        match &msg.parts[0] {
            Part::ToolResult(tr) => &tr.result,
            _ => panic!("expected ToolResult"),
        }
    }

    /// Build a message with a ToolCall + ToolResult (error) pair.
    fn error_msg_with_call(id: &str) -> Message {
        Message::new(
            Role::Tool,
            vec![
                Part::ToolCall(ToolCall::new(
                    id,
                    "run_cmd",
                    serde_json::json!({ "cmd": "bad_command" }),
                )),
                Part::ToolResult(ToolResult::new(
                    id,
                    "run_cmd",
                    serde_json::json!({ "stderr": "not found" }),
                    true,
                )),
            ],
        )
    }

    #[test]
    fn purge_removes_old_errors() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        // 5 messages: error at index 0, then 4 more (distance = 4 > 3)
        let mut messages = vec![
            error_msg("e1"),
            ok_msg("o1"),
            ok_msg("o2"),
            ok_msg("o3"),
            ok_msg("o4"),
        ];

        let result = PurgeErrorsStrategy::new(3).apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 1);

        let text = tool_result_value(&messages[0]).as_str().unwrap();
        assert!(text.contains("error purged after"));
        assert!(text.contains("4 turns"));
    }

    #[test]
    fn purge_keeps_recent_errors() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        // Error at index 2 (last), distance = 0 <= 3
        let mut messages = vec![ok_msg("o1"), ok_msg("o2"), error_msg("e1")];

        let result = PurgeErrorsStrategy::new(3).apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 0);

        // Error content unchanged
        assert!(tool_result_value(&messages[2]).get("stderr").is_some());
    }

    #[test]
    fn purge_respects_configurable_turn_count() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        // Error at index 0, distance = 2. With after_turns=1, should be purged.
        // With after_turns=3, should be kept.
        let mut messages_purge = vec![error_msg("e1"), ok_msg("o1"), ok_msg("o2")];
        let mut messages_keep = messages_purge.clone();

        let r1 = PurgeErrorsStrategy::new(1).apply(&mut messages_purge, &ctx);
        assert_eq!(r1.messages_modified, 1);

        let r2 = PurgeErrorsStrategy::new(3).apply(&mut messages_keep, &ctx);
        assert_eq!(r2.messages_modified, 0);
    }

    #[test]
    fn purge_also_purges_corresponding_tool_call() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        // Error with matching ToolCall at index 0, distance = 4 > 3
        let mut messages = vec![
            error_msg_with_call("e1"),
            ok_msg("o1"),
            ok_msg("o2"),
            ok_msg("o3"),
            ok_msg("o4"),
        ];

        let result = PurgeErrorsStrategy::new(3).apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 1);

        // ToolCall args should be blanked
        match &messages[0].parts[0] {
            Part::ToolCall(tc) => {
                assert_eq!(tc.args, serde_json::json!({}));
            }
            _ => panic!("expected ToolCall at part 0"),
        }

        // ToolResult should be replaced
        match &messages[0].parts[1] {
            Part::ToolResult(tr) => {
                assert!(tr.result.as_str().unwrap().contains("error purged after"));
            }
            _ => panic!("expected ToolResult at part 1"),
        }
    }
}
