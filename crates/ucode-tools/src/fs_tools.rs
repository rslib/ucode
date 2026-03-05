use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use serde_json::{Value, json};
use ucode_core::CoreError;

use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

fn missing_path_error() -> CoreError {
    CoreError::Tool("missing required argument: 'path'".into())
}

fn extract_path(args: &Value) -> Result<PathBuf, CoreError> {
    args["path"]
        .as_str()
        .map(PathBuf::from)
        .ok_or_else(missing_path_error)
}

/// Reads a file and returns its UTF-8 content.
pub struct ReadFileTool;

impl ToolHandler for ReadFileTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = extract_path(&args)?;

            let metadata = tokio::fs::metadata(&path)
                .await
                .map_err(|e| CoreError::Tool(format!("cannot stat '{}': {}", path.display(), e)))?;

            if !metadata.is_file() {
                return Err(CoreError::Tool(format!(
                    "'{}' is not a regular file",
                    path.display()
                )));
            }

            let bytes = tokio::fs::read(&path)
                .await
                .map_err(|e| CoreError::Tool(format!("cannot read '{}': {}", path.display(), e)))?;

            let content = String::from_utf8(bytes)
                .map_err(|_| CoreError::Tool(format!("'{}' is not valid UTF-8", path.display())))?;

            let resolved = path
                .canonicalize()
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned();

            Ok(json!({ "content": content, "path": resolved }))
        })
    }
}

/// Lists directory entries with name, is_dir, and size.
pub struct ListDirTool;

impl ToolHandler for ListDirTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let path = extract_path(&args)?;

            let metadata = tokio::fs::metadata(&path)
                .await
                .map_err(|e| CoreError::Tool(format!("cannot stat '{}': {}", path.display(), e)))?;

            if !metadata.is_dir() {
                return Err(CoreError::Tool(format!(
                    "'{}' is not a directory",
                    path.display()
                )));
            }

            let mut read_dir = tokio::fs::read_dir(&path).await.map_err(|e| {
                CoreError::Tool(format!("cannot read dir '{}': {}", path.display(), e))
            })?;

            let mut entries = Vec::new();
            while let Some(entry) = read_dir
                .next_entry()
                .await
                .map_err(|e| CoreError::Tool(format!("error reading dir entry: {}", e)))?
            {
                let name = entry.file_name().to_string_lossy().into_owned();
                let meta = entry
                    .metadata()
                    .await
                    .map_err(|e| CoreError::Tool(format!("cannot stat entry '{}': {}", name, e)))?;
                entries.push(json!({
                    "name": name,
                    "is_dir": meta.is_dir(),
                    "size": meta.len(),
                }));
            }

            let resolved = path
                .canonicalize()
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned();

            Ok(json!({ "entries": entries, "path": resolved }))
        })
    }
}

/// Registers `read_file` and `list_dir` into the given registry.
pub fn register_fs_tools(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "read_file".into(),
                description: "Read the UTF-8 contents of a file at the given path.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute or relative path to the file."
                        }
                    },
                    "required": ["path"]
                }),
            },
            Box::new(ReadFileTool),
        )
        .expect("read_file already registered");

    registry
        .register(
            ToolSpec {
                name: "list_dir".into(),
                description: "List entries in a directory at the given path.".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute or relative path to the directory."
                        }
                    },
                    "required": ["path"]
                }),
            },
            Box::new(ListDirTool),
        )
        .expect("list_dir already registered");
}
