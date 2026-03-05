use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use ast_grep_language::{LanguageExt, SupportLang};
use ignore::WalkBuilder;
use serde_json::{Value, json};
use ucode_core::CoreError;

use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

const MAX_MATCHES: usize = 100;

fn lang_extensions(lang: &str) -> &'static [&'static str] {
    match lang {
        "rust" | "rs" => &[".rs"],
        "python" | "py" => &[".py"],
        "javascript" | "js" => &[".js", ".mjs", ".cjs"],
        "typescript" | "ts" => &[".ts", ".mts", ".cts"],
        "tsx" => &[".tsx"],
        "go" => &[".go"],
        "c" => &[".c", ".h"],
        "cpp" | "c++" => &[".cpp", ".cc", ".cxx", ".hpp", ".hh", ".hxx"],
        "java" => &[".java"],
        "json" => &[".json"],
        "yaml" => &[".yml", ".yaml"],
        "bash" | "sh" => &[".sh", ".bash"],
        _ => &[],
    }
}

fn parse_lang(lang_str: &str) -> Result<SupportLang, CoreError> {
    lang_str
        .parse::<SupportLang>()
        .map_err(|_| CoreError::Tool(format!("unsupported language: '{lang_str}'")))
}

fn file_matches_lang(path: &Path, lang_str: &str) -> bool {
    let exts = lang_extensions(lang_str);
    if exts.is_empty() {
        // Unknown lang string — fall back to accepting all files so the
        // tree-sitter parser can decide.
        return true;
    }
    let name = path.to_string_lossy();
    exts.iter().any(|ext| name.ends_with(ext))
}

fn collect_files(path: &str, lang_str: &str) -> Result<Vec<PathBuf>, CoreError> {
    let root = PathBuf::from(path);
    if root.is_file() {
        return Ok(vec![root]);
    }
    if !root.is_dir() {
        return Err(CoreError::Tool(format!(
            "path '{}' is not a file or directory",
            path
        )));
    }

    let mut files = Vec::new();
    for entry in WalkBuilder::new(&root)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .require_git(false)
        .build()
        .flatten()
    {
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }
        let p = entry.into_path();
        if file_matches_lang(&p, lang_str) {
            files.push(p);
        }
    }
    Ok(files)
}

fn search_ast(pattern: &str, path: &str, lang_str: &str) -> Result<Value, CoreError> {
    let lang = parse_lang(lang_str)?;
    let files = collect_files(path, lang_str)?;

    let mut matches = Vec::new();
    let mut total_matches: usize = 0;
    let mut truncated = false;

    for file in files {
        let src = match std::fs::read_to_string(&file) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let root = lang.ast_grep(&src);
        for m in root.root().find_all(pattern) {
            total_matches += 1;
            if matches.len() >= MAX_MATCHES {
                truncated = true;
                continue;
            }
            let (start_line, start_col) = m.start_pos().byte_point();
            let (end_line, end_col) = m.end_pos().byte_point();
            matches.push(json!({
                "file": file.to_string_lossy(),
                // Convert to 1-based line numbers for consistency with editors.
                "line": start_line + 1,
                "column": start_col,
                "end_line": end_line + 1,
                "end_column": end_col,
                "text": m.text(),
            }));
        }
    }

    Ok(json!({
        "matches": matches,
        "total_matches": total_matches,
        "truncated": truncated,
    }))
}

fn rewrite_ast(
    pattern: &str,
    replacement: &str,
    path: &str,
    lang_str: &str,
) -> Result<Value, CoreError> {
    let lang = parse_lang(lang_str)?;
    let files = collect_files(path, lang_str)?;

    let mut files_changed = Vec::new();
    let mut total_replacements: usize = 0;

    for file in files {
        let src = match std::fs::read_to_string(&file) {
            Ok(s) => s,
            Err(_) => continue,
        };

        // Collect edits using a read-only root, then apply to a mutable root.
        // Edits must be applied in reverse position order so earlier byte
        // offsets remain valid after each splice.
        let read_root = lang.ast_grep(&src);
        let mut edits = read_root.root().replace_all(pattern, replacement);

        if edits.is_empty() {
            continue;
        }

        let count = edits.len();
        edits.sort_by(|a, b| b.position.cmp(&a.position));

        let mut write_root = lang.ast_grep(&src);
        for edit in edits {
            write_root.edit(edit).map_err(|e| {
                CoreError::Tool(format!("edit failed on '{}': {e}", file.display()))
            })?;
        }

        let new_src = write_root.generate();
        if new_src == src {
            continue;
        }

        std::fs::write(&file, &new_src)
            .map_err(|e| CoreError::Tool(format!("cannot write '{}': {e}", file.display())))?;

        total_replacements += count;
        files_changed.push(file.to_string_lossy().into_owned());
    }

    Ok(json!({
        "files_changed": files_changed,
        "total_replacements": total_replacements,
    }))
}

pub struct AstSearchTool;

impl ToolHandler for AstSearchTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let pattern = args["pattern"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing required argument: 'pattern'".into()))?
                .to_string();
            let path = args["path"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing required argument: 'path'".into()))?
                .to_string();
            let lang = args["lang"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing required argument: 'lang'".into()))?
                .to_string();

            tokio::task::spawn_blocking(move || search_ast(&pattern, &path, &lang))
                .await
                .map_err(|e| CoreError::Tool(format!("ast_search task panicked: {e}")))?
        })
    }
}

pub struct AstRewriteTool;

impl ToolHandler for AstRewriteTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let pattern = args["pattern"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing required argument: 'pattern'".into()))?
                .to_string();
            let replacement = args["replacement"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing required argument: 'replacement'".into()))?
                .to_string();
            let path = args["path"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing required argument: 'path'".into()))?
                .to_string();
            let lang = args["lang"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing required argument: 'lang'".into()))?
                .to_string();

            tokio::task::spawn_blocking(move || rewrite_ast(&pattern, &replacement, &path, &lang))
                .await
                .map_err(|e| CoreError::Tool(format!("ast_rewrite task panicked: {e}")))?
        })
    }
}

pub fn register_ast_search_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "ast_search".into(),
                description: "Search source files for an AST pattern using tree-sitter. \
                    Supports meta-variables like $VAR and $$$VARS."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "AST pattern with $VAR meta-variables (e.g. 'console.log($MSG)')"
                        },
                        "path": {
                            "type": "string",
                            "description": "File or directory path to search"
                        },
                        "lang": {
                            "type": "string",
                            "description": "Language identifier: rust, python, javascript, typescript, go, c, cpp, java, json, yaml, bash, tsx"
                        }
                    },
                    "required": ["pattern", "path", "lang"]
                }),
            },
            Box::new(AstSearchTool),
        )
        .expect("ast_search already registered");
}

pub fn register_ast_rewrite_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "ast_rewrite".into(),
                description: "Rewrite source files by replacing an AST pattern with a template. \
                    Meta-variables captured in the pattern are available in the replacement."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "AST pattern to match (e.g. 'println!($MSG)')"
                        },
                        "replacement": {
                            "type": "string",
                            "description": "Replacement template; may reference captured $VARs"
                        },
                        "path": {
                            "type": "string",
                            "description": "File or directory path to rewrite"
                        },
                        "lang": {
                            "type": "string",
                            "description": "Language identifier: rust, python, javascript, typescript, go, c, cpp, java, json, yaml, bash, tsx"
                        }
                    },
                    "required": ["pattern", "replacement", "path", "lang"]
                }),
            },
            Box::new(AstRewriteTool),
        )
        .expect("ast_rewrite already registered");
}
