use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use ignore::WalkBuilder;
use ignore::overrides::OverrideBuilder;
use regex::Regex;
use serde_json::{Value, json};
use ucode_core::CoreError;

use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

/// Ripgrep-like file search tool: walks a directory tree respecting .gitignore,
/// matches lines against a regex, and returns structured results.
pub struct SearchTool;

impl ToolHandler for SearchTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let query = args["query"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing required argument: 'query'".into()))?
                .to_string();
            let path_str = args["path"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing required argument: 'path'".into()))?
                .to_string();
            let max_results = args["max_results"].as_u64().unwrap_or(50) as usize;
            let include_glob = args["include_glob"].as_str().map(|s| s.to_string());
            let context_lines = args["context_lines"].as_u64().unwrap_or(0) as usize;

            tokio::task::spawn_blocking(move || {
                search_files(
                    &query,
                    &path_str,
                    max_results,
                    include_glob.as_deref(),
                    context_lines,
                )
            })
            .await
            .map_err(|e| CoreError::Tool(format!("search task panicked: {}", e)))?
        })
    }
}

fn search_files(
    query: &str,
    path: &str,
    max_results: usize,
    include_glob: Option<&str>,
    context_lines: usize,
) -> Result<Value, CoreError> {
    let root = PathBuf::from(path);
    if !root.is_dir() {
        return Err(CoreError::Tool(format!("'{}' is not a directory", path)));
    }

    let re = Regex::new(query).map_err(|e| CoreError::Tool(format!("invalid regex: {}", e)))?;

    let mut walk = WalkBuilder::new(&root);
    // require_git(false): honour .gitignore even outside a git repository.
    walk.hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false);

    if let Some(glob) = include_glob {
        let mut overrides = OverrideBuilder::new(&root);
        overrides
            .add(glob)
            .map_err(|e| CoreError::Tool(format!("invalid glob '{}': {}", glob, e)))?;
        let built = overrides
            .build()
            .map_err(|e| CoreError::Tool(format!("glob build error: {}", e)))?;
        walk.overrides(built);
    }

    let mut matches = Vec::new();
    let mut total_matches: usize = 0;
    let mut truncated = false;

    for entry in walk.build().flatten() {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        let file_path = entry.path().to_path_buf();
        let content = match std::fs::read_to_string(&file_path) {
            Ok(c) => c,
            Err(_) => continue, // skip binary or unreadable files
        };

        let lines: Vec<&str> = content.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !re.is_match(line) {
                continue;
            }

            total_matches += 1;

            if matches.len() >= max_results {
                truncated = true;
                continue;
            }

            let ctx_before: Vec<&str> = lines[i.saturating_sub(context_lines)..i].to_vec();
            let ctx_after_start = (i + 1).min(lines.len());
            let ctx_after_end = (i + 1 + context_lines).min(lines.len());
            let ctx_after: Vec<&str> = lines[ctx_after_start..ctx_after_end].to_vec();

            matches.push(json!({
                "path": file_path.to_string_lossy(),
                "line_number": i + 1,
                "line": line,
                "context_before": ctx_before,
                "context_after": ctx_after,
            }));
        }
    }

    Ok(json!({
        "matches": matches,
        "total_matches": total_matches,
        "truncated": truncated,
    }))
}

/// Registers the `search` tool into the given registry.
pub fn register_search_tool(registry: &mut ToolRegistry) {
    let spec = ToolSpec {
        name: "search".into(),
        description: "Search files for a regex pattern, respecting .gitignore".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Regex pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "Root directory to search"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum matches to return (default 50)"
                },
                "include_glob": {
                    "type": "string",
                    "description": "File glob filter (e.g., '*.rs')"
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Lines of context around matches (default 0)"
                }
            },
            "required": ["query", "path"]
        }),
    };
    registry
        .register(spec, Box::new(SearchTool))
        .expect("search tool registration should not fail");
}
