use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};

use ucode_core::{Message, Part};

use crate::strategy::{ContextStrategy, StrategyContext, StrategyResult};

const FILE_READ_TOOLS: &[&str] = &["read_file", "list_dir", "ripgrep_search", "ast_search"];

/// Detects duplicate file-read tool results and replaces later duplicates with a placeholder.
pub struct DedupStrategy;

fn extract_path(result: &serde_json::Value) -> String {
    if let Some(p) = result.get("path").and_then(|v| v.as_str()) {
        return p.to_string();
    }
    if let Some(p) = result.get("file").and_then(|v| v.as_str()) {
        return p.to_string();
    }
    result.to_string()
}

fn hash_value(v: &serde_json::Value) -> u64 {
    let mut h = DefaultHasher::new();
    v.to_string().hash(&mut h);
    h.finish()
}

impl ContextStrategy for DedupStrategy {
    fn name(&self) -> &str {
        "dedup"
    }

    fn apply(&self, messages: &mut Vec<Message>, _ctx: &StrategyContext) -> StrategyResult {
        // (tool_name, path) -> content_hash
        let mut seen: HashMap<(String, String), u64> = HashMap::new();
        let mut chars_saved = 0usize;
        let mut messages_modified = 0usize;

        for msg in messages.iter_mut() {
            for part in msg.parts.iter_mut() {
                let Part::ToolResult(tr) = part else {
                    continue;
                };
                if !FILE_READ_TOOLS.contains(&tr.name.as_str()) {
                    continue;
                }
                let path = extract_path(&tr.result);
                let hash = hash_value(&tr.result);
                let key = (tr.name.clone(), path.clone());

                match seen.get(&key) {
                    Some(&prev_hash) if prev_hash == hash => {
                        let placeholder =
                            format!("[already in context -- see earlier read of {path}]");
                        let old_len = tr.result.to_string().len();
                        chars_saved += old_len.saturating_sub(placeholder.len());
                        tr.result = serde_json::Value::String(placeholder);
                        messages_modified += 1;
                    }
                    _ => {
                        seen.insert(key, hash);
                    }
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
    use ucode_core::{CharEstimator, Part, TokenBudget};

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

    fn read_result(path: &str, content: &str) -> Message {
        Message::tool_result(
            "id1",
            "read_file",
            serde_json::json!({ "path": path, "content": content }),
            false,
        )
    }

    fn tool_result_value(msg: &Message) -> &serde_json::Value {
        match &msg.parts[0] {
            Part::ToolResult(tr) => &tr.result,
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn dedup_removes_duplicate_file_reads() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        let mut messages = vec![
            read_result("/foo/bar.rs", "fn main() {}"),
            read_result("/foo/bar.rs", "fn main() {}"),
        ];

        let result = DedupStrategy.apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 1);

        // First message unchanged
        assert!(tool_result_value(&messages[0]).get("content").is_some());

        // Second message replaced with placeholder
        let text = tool_result_value(&messages[1]).as_str().unwrap();
        assert!(text.contains("already in context"));
        assert!(text.contains("/foo/bar.rs"));
    }

    #[test]
    fn dedup_keeps_different_content() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        let mut messages = vec![
            read_result("/foo/bar.rs", "fn main() {}"),
            read_result("/foo/bar.rs", "fn main() { println!(\"updated\"); }"),
        ];

        let result = DedupStrategy.apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 0);

        // Both messages kept intact
        for msg in &messages {
            assert!(tool_result_value(msg).get("content").is_some());
        }
    }

    #[test]
    fn dedup_ignores_non_file_tools() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        let cmd_result = serde_json::json!({ "path": "/foo/bar.rs", "output": "hello" });
        let mut messages = vec![
            Message::tool_result("id1", "run_cmd", cmd_result.clone(), false),
            Message::tool_result("id2", "run_cmd", cmd_result, false),
        ];

        let result = DedupStrategy.apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 0);
    }

    #[test]
    fn dedup_replaces_with_correct_placeholder() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        let mut messages = vec![
            read_result("/src/main.rs", "content"),
            read_result("/src/main.rs", "content"),
        ];

        DedupStrategy.apply(&mut messages, &ctx);

        let text = tool_result_value(&messages[1]).as_str().unwrap();
        assert_eq!(
            text,
            "[already in context -- see earlier read of /src/main.rs]"
        );
    }
}
