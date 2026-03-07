use ucode_core::{Message, Part};

use crate::strategy::{ContextStrategy, StrategyContext, StrategyResult};

/// Replaces large tool outputs with a metadata summary to reduce context size.
pub struct SandboxInterceptor {
    pub threshold_chars: usize,
}

impl SandboxInterceptor {
    pub fn new(threshold_chars: usize) -> Self {
        Self { threshold_chars }
    }
}

impl ContextStrategy for SandboxInterceptor {
    fn name(&self) -> &str {
        "sandbox"
    }

    fn apply(&self, messages: &mut Vec<Message>, _ctx: &StrategyContext) -> StrategyResult {
        let mut chars_saved = 0usize;
        let mut messages_modified = 0usize;

        for msg in messages.iter_mut() {
            for part in msg.parts.iter_mut() {
                let Part::ToolResult(tr) = part else {
                    continue;
                };
                let content = tr.result.to_string();
                let char_count = content.len();
                if char_count <= self.threshold_chars {
                    continue;
                }

                let lines: Vec<&str> = content.split('\n').collect();
                let line_count = lines.len();

                let first: Vec<&str> = lines.iter().take(3).copied().collect();
                let last: Vec<&str> = lines.iter().rev().take(3).rev().copied().collect();

                let summary = format!(
                    "[Large output ({line_count} lines, {char_count} chars) from {tool_name}]\nFirst lines:\n{first_lines}\n...\nLast lines:\n{last_lines}\n[Full content stored in knowledge base]",
                    tool_name = tr.name,
                    first_lines = first.join("\n"),
                    last_lines = last.join("\n"),
                );

                chars_saved += char_count.saturating_sub(summary.len());
                tr.result = serde_json::Value::String(summary);
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

    fn large_output(n: usize) -> serde_json::Value {
        // Build a multi-line string that exceeds n chars
        let line = "x".repeat(100);
        let lines: Vec<String> = (0..((n / 100) + 2))
            .map(|i| format!("{i}: {line}"))
            .collect();
        serde_json::Value::String(lines.join("\n"))
    }

    fn tool_result_str(msg: &Message) -> &str {
        match &msg.parts[0] {
            Part::ToolResult(tr) => tr.result.as_str().unwrap(),
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn sandbox_replaces_large_output_with_summary() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        let mut messages = vec![Message::tool_result(
            "id1",
            "read_file",
            large_output(2000),
            false,
        )];

        let result = SandboxInterceptor::new(2000).apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 1);

        let text = tool_result_str(&messages[0]);
        assert!(text.contains("Large output"));
        assert!(text.contains("lines"));
        assert!(text.contains("chars"));
        assert!(text.contains("knowledge base"));
    }

    #[test]
    fn sandbox_keeps_small_output() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        let small = serde_json::Value::String("short output".to_string());
        let mut messages = vec![Message::tool_result(
            "id1",
            "read_file",
            small.clone(),
            false,
        )];

        let result = SandboxInterceptor::new(2000).apply(&mut messages, &ctx);
        assert_eq!(result.messages_modified, 0);

        match &messages[0].parts[0] {
            Part::ToolResult(tr) => assert_eq!(tr.result, small),
            _ => panic!("expected ToolResult"),
        }
    }

    #[test]
    fn sandbox_respects_custom_threshold() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        // 600-char output: exceeds 500 threshold but not 2000
        let output = serde_json::Value::String("a".repeat(600));
        let mut messages_low = vec![Message::tool_result("id1", "cmd", output.clone(), false)];
        let mut messages_high = vec![Message::tool_result("id2", "cmd", output, false)];

        let r_low = SandboxInterceptor::new(500).apply(&mut messages_low, &ctx);
        assert_eq!(r_low.messages_modified, 1);

        let r_high = SandboxInterceptor::new(2000).apply(&mut messages_high, &ctx);
        assert_eq!(r_high.messages_modified, 0);
    }

    #[test]
    fn sandbox_summary_format() {
        let budget = TokenBudget::new(128_000, 4_096);
        let counter = CharEstimator::default();
        let ctx = ctx(&budget, &counter);

        // Build a known multi-line string with identifiable first/last lines
        let content = (0..50)
            .map(|i| format!("line_{i:02}: {}", "data".repeat(10)))
            .collect::<Vec<_>>()
            .join("\n");
        let char_count = serde_json::Value::String(content.clone()).to_string().len();

        let mut messages = vec![Message::tool_result(
            "id1",
            "my_tool",
            serde_json::Value::String(content),
            false,
        )];

        SandboxInterceptor::new(100).apply(&mut messages, &ctx);

        let text = tool_result_str(&messages[0]);

        // line count and char count present
        assert!(text.contains("lines"), "missing 'lines': {text}");
        assert!(
            text.contains(&char_count.to_string()),
            "missing char count: {text}"
        );

        // tool name present
        assert!(text.contains("my_tool"), "missing tool name: {text}");

        // first and last line markers
        assert!(
            text.contains("First lines:"),
            "missing 'First lines:': {text}"
        );
        assert!(
            text.contains("Last lines:"),
            "missing 'Last lines:': {text}"
        );
        assert!(text.contains("line_00:"), "missing first line: {text}");
        assert!(text.contains("line_49:"), "missing last line: {text}");

        // knowledge base note
        assert!(
            text.contains("Full content stored in knowledge base"),
            "missing kb note: {text}"
        );
    }
}
