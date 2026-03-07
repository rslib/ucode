use ucode_core::{Message, Part};

use crate::strategy::{ContextStrategy, StrategyContext, StrategyResult};

const WRITE_TOOLS: &[&str] = &["write_file", "apply_patch", "create_file"];
const READ_TOOLS: &[&str] = &["read_file"];

/// Replaces write tool results that are later superseded by a read of the same file.
pub struct SupersedeStrategy;

fn extract_path_from_value(v: &serde_json::Value) -> Option<String> {
    if let Some(p) = v.get("path").and_then(|x| x.as_str()) {
        return Some(p.to_string());
    }
    if let Some(p) = v.get("file").and_then(|x| x.as_str()) {
        return Some(p.to_string());
    }
    None
}

impl ContextStrategy for SupersedeStrategy {
    fn name(&self) -> &str {
        "supersede"
    }

    fn apply(&self, messages: &mut Vec<Message>, _ctx: &StrategyContext) -> StrategyResult {
        // Pass 1: collect (path, msg_idx, part_idx) for writes and reads.
        struct Entry {
            path: String,
            msg_idx: usize,
            part_idx: usize,
        }

        let mut writes: Vec<Entry> = Vec::new();
        let mut reads: Vec<Entry> = Vec::new();

        for (msg_idx, msg) in messages.iter().enumerate() {
            for (part_idx, part) in msg.parts.iter().enumerate() {
                match part {
                    Part::ToolCall(tc) if WRITE_TOOLS.contains(&tc.name.as_str()) => {
                        if let Some(path) = extract_path_from_value(&tc.args) {
                            writes.push(Entry {
                                path,
                                msg_idx,
                                part_idx,
                            });
                        }
                    }
                    Part::ToolResult(tr) if WRITE_TOOLS.contains(&tr.name.as_str()) => {
                        if let Some(path) = extract_path_from_value(&tr.result) {
                            writes.push(Entry {
                                path,
                                msg_idx,
                                part_idx,
                            });
                        }
                    }
                    Part::ToolResult(tr) if READ_TOOLS.contains(&tr.name.as_str()) => {
                        if let Some(path) = extract_path_from_value(&tr.result) {
                            reads.push(Entry {
                                path,
                                msg_idx,
                                part_idx,
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

        // Pass 2: for each write ToolResult, check if a later read of the same path exists.
        let mut messages_modified = 0usize;
        let mut chars_saved = 0usize;

        for write in &writes {
            let superseded = reads
                .iter()
                .any(|r| r.path == write.path && r.msg_idx > write.msg_idx);

            if !superseded {
                continue;
            }

            let part = &mut messages[write.msg_idx].parts[write.part_idx];
            if let Part::ToolResult(tr) = part
                && WRITE_TOOLS.contains(&tr.name.as_str())
            {
                let placeholder = format!("[superseded by later read of {}]", write.path);
                let old_len = tr.result.to_string().len();
                chars_saved += old_len.saturating_sub(placeholder.len());
                tr.result = serde_json::Value::String(placeholder);
                messages_modified += 1;
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

    fn write_msg(tool: &str, path: &str) -> Message {
        Message::tool_result(
            "w1",
            tool,
            serde_json::json!({ "path": path, "bytes_written": 42 }),
            false,
        )
    }

    fn read_msg(path: &str) -> Message {
        Message::tool_result(
            "r1",
            "read_file",
            serde_json::json!({ "path": path, "content": "file content" }),
            false,
        )
    }

    fn tool_result_value(msg: &Message) -> &serde_json::Value {
        match &msg.parts[0] {
            Part::ToolResult(tr) => &tr.result,
            _ => panic!("expected ToolResult"),
        }
    }

    /// Build a message that contains both a ToolCall and a ToolResult for a write.
    fn write_msg_with_call(tool: &str, path: &str) -> Message {
        Message::new(
            Role::Tool,
            vec![
                Part::ToolCall(ToolCall::new(
                    "w2",
                    tool,
                    serde_json::json!({ "path": path, "content": "new content" }),
                )),
                Part::ToolResult(ToolResult::new(
                    "w2",
                    tool,
                    serde_json::json!({ "path": path, "bytes_written": 11 }),
                    false,
                )),
            ],
        )
    }

    #[test]
    fn supersede_replaces_write_when_later_read_exists() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        let mut messages = vec![
            write_msg("write_file", "/src/lib.rs"),
            read_msg("/src/lib.rs"),
        ];

        let result = SupersedeStrategy.apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 1);

        let text = tool_result_value(&messages[0]).as_str().unwrap();
        assert!(text.contains("superseded"));
        assert!(text.contains("/src/lib.rs"));
    }

    #[test]
    fn supersede_keeps_write_without_later_read() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        let mut messages = vec![write_msg("write_file", "/src/lib.rs")];

        let result = SupersedeStrategy.apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 0);

        // Content unchanged
        assert!(
            tool_result_value(&messages[0])
                .get("bytes_written")
                .is_some()
        );
    }

    #[test]
    fn supersede_handles_apply_patch() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        let mut messages = vec![
            write_msg("apply_patch", "/src/main.rs"),
            read_msg("/src/main.rs"),
        ];

        let result = SupersedeStrategy.apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 1);

        let text = tool_result_value(&messages[0]).as_str().unwrap();
        assert!(text.contains("superseded"));
        assert!(text.contains("/src/main.rs"));
    }

    #[test]
    fn supersede_does_not_replace_write_when_read_is_earlier() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        // Read comes before write -- should NOT supersede
        let mut messages = vec![
            read_msg("/src/lib.rs"),
            write_msg("write_file", "/src/lib.rs"),
        ];

        let result = SupersedeStrategy.apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 0);
    }

    #[test]
    fn supersede_write_with_call_part() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        let mut messages = vec![
            write_msg_with_call("write_file", "/src/lib.rs"),
            read_msg("/src/lib.rs"),
        ];

        let result = SupersedeStrategy.apply(&mut messages, &ctx);
        // The ToolResult part (part_idx 1) should be replaced
        assert_eq!(result.messages_modified, 1);

        let tr_val = match &messages[0].parts[1] {
            Part::ToolResult(tr) => &tr.result,
            _ => panic!("expected ToolResult at part 1"),
        };
        assert!(tr_val.as_str().unwrap().contains("superseded"));
    }
}
