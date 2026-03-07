use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use tokio::sync::RwLock;
use ucode_core::{CoreError, Message, Part};
use ucode_tools::{ToolHandler, ToolRegistry, ToolSpec};

use crate::config::PruningConfig;
use crate::knowledge::KnowledgeBase;

// ---------------------------------------------------------------------------
// KnowledgeSearchHandler
// ---------------------------------------------------------------------------

pub struct KnowledgeSearchHandler {
    kb: Arc<Mutex<KnowledgeBase>>,
}

impl KnowledgeSearchHandler {
    pub fn new(kb: Arc<Mutex<KnowledgeBase>>) -> Self {
        Self { kb }
    }
}

impl ToolHandler for KnowledgeSearchHandler {
    fn invoke(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, CoreError>> + Send>> {
        let kb = Arc::clone(&self.kb);
        Box::pin(async move {
            let query = args["query"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing 'query' argument".into()))?
                .to_string();
            let limit = args["limit"].as_u64().unwrap_or(10) as usize;

            let entries = kb
                .lock()
                .map_err(|e| CoreError::Tool(format!("kb lock poisoned: {e}")))?
                .search(&query, limit)
                .map_err(|e| CoreError::Tool(e.to_string()))?;

            let results: Vec<serde_json::Value> = entries
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "source": e.source,
                        "content": e.content,
                        "score": e.score,
                    })
                })
                .collect();

            Ok(serde_json::Value::Array(results))
        })
    }
}

// ---------------------------------------------------------------------------
// KnowledgeStoreHandler
// ---------------------------------------------------------------------------

pub struct KnowledgeStoreHandler {
    kb: Arc<Mutex<KnowledgeBase>>,
}

impl KnowledgeStoreHandler {
    pub fn new(kb: Arc<Mutex<KnowledgeBase>>) -> Self {
        Self { kb }
    }
}

impl ToolHandler for KnowledgeStoreHandler {
    fn invoke(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, CoreError>> + Send>> {
        let kb = Arc::clone(&self.kb);
        Box::pin(async move {
            let source = args["source"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing 'source' argument".into()))?
                .to_string();
            let content = args["content"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing 'content' argument".into()))?
                .to_string();
            let metadata = args["metadata"].as_str().map(str::to_string);

            let id = kb
                .lock()
                .map_err(|e| CoreError::Tool(format!("kb lock poisoned: {e}")))?
                .store(&source, &content, metadata.as_deref())
                .map_err(|e| CoreError::Tool(e.to_string()))?;

            Ok(serde_json::json!({
                "id": id,
                "message": "Stored successfully",
            }))
        })
    }
}

// ---------------------------------------------------------------------------
// ContextPruneHandler
// ---------------------------------------------------------------------------

pub struct ContextPruneHandler {
    messages: Arc<RwLock<Vec<Message>>>,
}

impl ContextPruneHandler {
    pub fn new(messages: Arc<RwLock<Vec<Message>>>) -> Self {
        Self { messages }
    }
}

impl ToolHandler for ContextPruneHandler {
    fn invoke(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, CoreError>> + Send>> {
        let messages = Arc::clone(&self.messages);
        Box::pin(async move {
            let indices_val = args["indices"]
                .as_array()
                .ok_or_else(|| CoreError::Tool("missing 'indices' argument".into()))?;

            let mut indices: Vec<usize> = indices_val
                .iter()
                .map(|v| {
                    v.as_u64().map(|n| n as usize).ok_or_else(|| {
                        CoreError::Tool("indices must be non-negative integers".into())
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            // Sort descending so we can remove by index without shifting.
            indices.sort_unstable_by(|a, b| b.cmp(a));
            indices.dedup();

            let mut msgs = messages.write().await;
            let len = msgs.len();

            let mut removed = 0usize;
            for idx in &indices {
                if *idx < msgs.len() {
                    msgs.remove(*idx);
                    removed += 1;
                }
            }

            Ok(serde_json::json!({
                "removed": removed,
                "remaining": len - removed,
            }))
        })
    }
}

// ---------------------------------------------------------------------------
// ContextCompressHandler
// ---------------------------------------------------------------------------

pub struct ContextCompressHandler {
    messages: Arc<RwLock<Vec<Message>>>,
}

impl ContextCompressHandler {
    pub fn new(messages: Arc<RwLock<Vec<Message>>>) -> Self {
        Self { messages }
    }
}

impl ToolHandler for ContextCompressHandler {
    fn invoke(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, CoreError>> + Send>> {
        let messages = Arc::clone(&self.messages);
        Box::pin(async move {
            let message_index = args["message_index"]
                .as_u64()
                .ok_or_else(|| CoreError::Tool("missing 'message_index' argument".into()))?
                as usize;
            let key_findings = args["key_findings"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing 'key_findings' argument".into()))?
                .to_string();

            let mut msgs = messages.write().await;
            let msg = msgs.get_mut(message_index).ok_or_else(|| {
                CoreError::Tool(format!("message_index {} out of range", message_index))
            })?;

            // Replace the first ToolResult part's content with key_findings.
            let mut replaced = false;
            for part in &mut msg.parts {
                if let Part::ToolResult(tr) = part {
                    tr.result = serde_json::Value::String(key_findings.clone());
                    replaced = true;
                    break;
                }
            }

            if !replaced {
                return Err(CoreError::Tool(format!(
                    "no ToolResult part found at message_index {}",
                    message_index
                )));
            }

            Ok(serde_json::json!({
                "compressed": true,
                "message_index": message_index,
            }))
        })
    }
}

// ---------------------------------------------------------------------------
// ContextDistillHandler
// ---------------------------------------------------------------------------

pub struct ContextDistillHandler {
    messages: Arc<RwLock<Vec<Message>>>,
}

impl ContextDistillHandler {
    pub fn new(messages: Arc<RwLock<Vec<Message>>>) -> Self {
        Self { messages }
    }
}

impl ToolHandler for ContextDistillHandler {
    fn invoke(
        &self,
        args: serde_json::Value,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, CoreError>> + Send>> {
        let messages = Arc::clone(&self.messages);
        Box::pin(async move {
            let start_index = args["start_index"]
                .as_u64()
                .ok_or_else(|| CoreError::Tool("missing 'start_index' argument".into()))?
                as usize;
            let end_index = args["end_index"]
                .as_u64()
                .ok_or_else(|| CoreError::Tool("missing 'end_index' argument".into()))?
                as usize;
            let digest = args["digest"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing 'digest' argument".into()))?
                .to_string();

            let mut msgs = messages.write().await;

            if start_index > end_index || end_index > msgs.len() {
                return Err(CoreError::Tool(format!(
                    "invalid range [{}, {}), len={}",
                    start_index,
                    end_index,
                    msgs.len()
                )));
            }

            let messages_replaced = end_index - start_index;
            msgs.drain(start_index..end_index);
            msgs.insert(start_index, Message::assistant(digest));

            Ok(serde_json::json!({
                "distilled": true,
                "messages_replaced": messages_replaced,
            }))
        })
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Register all 5 context management tools into `registry`.
pub fn register_context_tools(
    registry: &mut ToolRegistry,
    kb: Arc<Mutex<KnowledgeBase>>,
    messages: Arc<RwLock<Vec<Message>>>,
) -> Result<(), CoreError> {
    registry.register(
        ToolSpec {
            name: "knowledge_search".into(),
            description: "Search the knowledge base for relevant entries.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query" },
                    "limit": { "type": "integer", "description": "Max results (default 10)" }
                },
                "required": ["query"]
            }),
        },
        Box::new(KnowledgeSearchHandler::new(Arc::clone(&kb))),
    )?;

    registry.register(
        ToolSpec {
            name: "knowledge_store".into(),
            description: "Store content in the knowledge base for later retrieval.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Source identifier" },
                    "content": { "type": "string", "description": "Content to store" },
                    "metadata": { "type": "string", "description": "Optional metadata JSON" }
                },
                "required": ["source", "content"]
            }),
        },
        Box::new(KnowledgeStoreHandler::new(Arc::clone(&kb))),
    )?;

    registry.register(
        ToolSpec {
            name: "context_prune".into(),
            description: "Remove specific messages by index from the conversation context.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "indices": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "Message indices to remove"
                    }
                },
                "required": ["indices"]
            }),
        },
        Box::new(ContextPruneHandler::new(Arc::clone(&messages))),
    )?;

    registry.register(
        ToolSpec {
            name: "context_compress".into(),
            description: "Replace a message's tool output with a concise summary.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "message_index": { "type": "integer", "description": "Index of the message to compress" },
                    "key_findings": { "type": "string", "description": "Summary text to replace the tool output" }
                },
                "required": ["message_index", "key_findings"]
            }),
        },
        Box::new(ContextCompressHandler::new(Arc::clone(&messages))),
    )?;

    registry.register(
        ToolSpec {
            name: "context_distill".into(),
            description: "Summarize a range of messages into a single digest message.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "start_index": { "type": "integer", "description": "Start of range (inclusive)" },
                    "end_index": { "type": "integer", "description": "End of range (exclusive)" },
                    "digest": { "type": "string", "description": "Summary text to replace the range" }
                },
                "required": ["start_index", "end_index", "digest"]
            }),
        },
        Box::new(ContextDistillHandler::new(messages)),
    )?;

    Ok(())
}

// ---------------------------------------------------------------------------
// System prompt injection
// ---------------------------------------------------------------------------

/// Generate the pruning system prompt fragment.
///
/// Returns `None` if `context_usage_pct` is below `threshold_pct`.
pub fn pruning_system_prompt(context_usage_pct: u8, threshold_pct: u8) -> Option<String> {
    if context_usage_pct < threshold_pct {
        return None;
    }
    Some(format!(
        "Context window is {context_usage_pct}% full. \
         You have access to context management tools: \
         knowledge_search, knowledge_store, context_prune, context_compress, context_distill. \
         Consider using them to manage context size."
    ))
}

// ---------------------------------------------------------------------------
// Pruning config helpers
// ---------------------------------------------------------------------------

/// Check if pruning is enabled for a given model.
pub fn is_pruning_enabled(config: &PruningConfig, model_name: &str) -> bool {
    let (enabled, _) = config.resolve(model_name);
    enabled
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tempfile::TempDir;
    use tokio::sync::RwLock;

    use ucode_core::{Message, Part};

    use crate::config::{PruningConfig, PruningOverride};
    use crate::knowledge::KnowledgeBase;

    use super::*;

    fn open_kb(dir: &TempDir) -> Arc<Mutex<KnowledgeBase>> {
        Arc::new(Mutex::new(
            KnowledgeBase::open(&dir.path().join("kb.db"), None).expect("open failed"),
        ))
    }

    fn make_messages(texts: &[&str]) -> Arc<RwLock<Vec<Message>>> {
        let msgs: Vec<Message> = texts.iter().map(|t| Message::user(*t)).collect();
        Arc::new(RwLock::new(msgs))
    }

    #[tokio::test]
    async fn knowledge_search_returns_results() {
        let dir = TempDir::new().unwrap();
        let kb = open_kb(&dir);
        kb.lock()
            .unwrap()
            .store("src1", "Rust ownership and borrowing rules", None)
            .unwrap();

        let handler = KnowledgeSearchHandler::new(Arc::clone(&kb));
        let result = handler
            .invoke(serde_json::json!({ "query": "ownership", "limit": 5 }))
            .await
            .unwrap();

        let arr = result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["source"], "src1");
        assert!(arr[0]["content"].as_str().unwrap().contains("ownership"));
        assert!(arr[0]["score"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn knowledge_store_indexes_content() {
        let dir = TempDir::new().unwrap();
        let kb = open_kb(&dir);

        let store_handler = KnowledgeStoreHandler::new(Arc::clone(&kb));
        let store_result = store_handler
            .invoke(serde_json::json!({
                "source": "notes",
                "content": "async await tokio runtime",
            }))
            .await
            .unwrap();

        assert_eq!(store_result["message"], "Stored successfully");
        assert!(store_result["id"].as_i64().unwrap() > 0);

        let search_handler = KnowledgeSearchHandler::new(Arc::clone(&kb));
        let search_result = search_handler
            .invoke(serde_json::json!({ "query": "tokio" }))
            .await
            .unwrap();

        let arr = search_result.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["source"], "notes");
    }

    #[tokio::test]
    async fn context_prune_removes_messages() {
        let messages = make_messages(&["msg0", "msg1", "msg2", "msg3", "msg4"]);
        let handler = ContextPruneHandler::new(Arc::clone(&messages));

        let result = handler
            .invoke(serde_json::json!({ "indices": [1, 3] }))
            .await
            .unwrap();

        assert_eq!(result["removed"], 2);
        assert_eq!(result["remaining"], 3);

        let msgs = messages.read().await;
        assert_eq!(msgs.len(), 3);
        // msg1 and msg3 removed; remaining: msg0, msg2, msg4
        assert_eq!(msgs[0].parts[0], Part::Text("msg0".into()));
        assert_eq!(msgs[1].parts[0], Part::Text("msg2".into()));
        assert_eq!(msgs[2].parts[0], Part::Text("msg4".into()));
    }

    #[tokio::test]
    async fn context_compress_replaces_output() {
        let tool_msg = Message::tool_result(
            "call-1",
            "some_tool",
            serde_json::Value::String("verbose output here".into()),
            false,
        );
        let messages = Arc::new(RwLock::new(vec![Message::user("do something"), tool_msg]));

        let handler = ContextCompressHandler::new(Arc::clone(&messages));
        let result = handler
            .invoke(serde_json::json!({
                "message_index": 1,
                "key_findings": "summary of output",
            }))
            .await
            .unwrap();

        assert_eq!(result["compressed"], true);
        assert_eq!(result["message_index"], 1);

        let msgs = messages.read().await;
        if let Part::ToolResult(tr) = &msgs[1].parts[0] {
            assert_eq!(
                tr.result,
                serde_json::Value::String("summary of output".into())
            );
        } else {
            panic!("expected ToolResult part");
        }
    }

    #[tokio::test]
    async fn context_distill_summarizes_range() {
        let messages = make_messages(&["msg0", "msg1", "msg2", "msg3", "msg4"]);
        let handler = ContextDistillHandler::new(Arc::clone(&messages));

        let result = handler
            .invoke(serde_json::json!({
                "start_index": 1,
                "end_index": 4,
                "digest": "summary of msgs 1-3",
            }))
            .await
            .unwrap();

        assert_eq!(result["distilled"], true);
        assert_eq!(result["messages_replaced"], 3);

        let msgs = messages.read().await;
        // 5 - 3 + 1 digest = 3 messages
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].parts[0], Part::Text("msg0".into()));
        assert_eq!(msgs[1].parts[0], Part::Text("summary of msgs 1-3".into()));
        assert_eq!(msgs[2].parts[0], Part::Text("msg4".into()));
    }

    #[test]
    fn pruning_prompt_only_above_threshold() {
        assert!(pruning_system_prompt(59, 60).is_none());
        assert!(pruning_system_prompt(60, 60).is_some());
        assert!(pruning_system_prompt(85, 60).is_some());

        let prompt = pruning_system_prompt(75, 60).unwrap();
        assert!(prompt.contains("75%"));
        assert!(prompt.contains("knowledge_search"));
    }

    #[test]
    fn pruning_disabled_for_model_override() {
        let mut config = PruningConfig::default(); // enabled=true
        config.overrides.insert(
            "gpt-4".to_string(),
            PruningOverride {
                enabled: Some(false),
                trigger_threshold_pct: None,
            },
        );

        assert!(is_pruning_enabled(&config, "claude-3-opus")); // no override
        assert!(!is_pruning_enabled(&config, "gpt-4")); // disabled by override
    }
}
