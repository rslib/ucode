use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use ucode_core::CoreError;

use crate::registry::{ToolHandler, ToolRegistry, ToolSpec};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reject {
    pub file: String,
    pub hunk: usize,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PatchResult {
    pub applied: bool,
    pub files_changed: Vec<String>,
    pub rejects: Vec<Reject>,
}

/// Tool handler for `apply_patch`.
pub struct PatchTool;

impl ToolHandler for PatchTool {
    fn invoke(
        &self,
        args: Value,
    ) -> Pin<Box<dyn Future<Output = Result<Value, CoreError>> + Send>> {
        Box::pin(async move {
            let diff = args["diff"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing 'diff' argument".into()))?
                .to_string();
            let base_dir = args["base_dir"]
                .as_str()
                .ok_or_else(|| CoreError::Tool("missing 'base_dir' argument".into()))?
                .to_string();

            tokio::task::spawn_blocking(move || apply_patch(&diff, &base_dir))
                .await
                .map_err(|e| CoreError::Tool(format!("patch task panicked: {e}")))?
        })
    }
}

fn apply_patch(diff: &str, base_dir: &str) -> Result<Value, CoreError> {
    let base = std::path::Path::new(base_dir);
    if !base.is_dir() {
        return Err(CoreError::Tool(format!(
            "'{}' is not a directory",
            base_dir
        )));
    }

    // parse_auto handles raw unified diffs, markdown-embedded diffs, and conflict markers.
    let patches = mpatch::parse_auto(diff)
        .map_err(|e| CoreError::Tool(format!("failed to parse diff: {e}")))?;

    if patches.is_empty() {
        return Err(CoreError::Tool("no patches found in diff input".into()));
    }

    let options = mpatch::ApplyOptions::new();
    let batch_result = mpatch::apply_patches_to_dir(&patches, base, options);

    let mut files_changed = Vec::new();
    let mut rejects = Vec::new();

    for (path, result) in &batch_result.results {
        let file_path = path.to_string_lossy().to_string();

        match result {
            Ok(patch_result) => {
                if patch_result.report.all_applied_cleanly() {
                    files_changed.push(file_path);
                } else {
                    for failure in patch_result.report.failures() {
                        rejects.push(Reject {
                            file: file_path.clone(),
                            hunk: failure.hunk_index,
                            reason: format!("{}", failure.reason),
                        });
                    }
                    // Partial success: some hunks applied.
                    if patch_result.report.success_count() > 0 {
                        files_changed.push(file_path);
                    }
                }
            }
            Err(e) => {
                rejects.push(Reject {
                    file: file_path,
                    hunk: 0,
                    reason: format!("{e}"),
                });
            }
        }
    }

    let applied = rejects.is_empty();
    Ok(json!(PatchResult {
        applied,
        files_changed,
        rejects,
    }))
}

/// Register the `apply_patch` tool into the given registry.
pub fn register_patch_tool(registry: &mut ToolRegistry) {
    registry
        .register(
            ToolSpec {
                name: "apply_patch".into(),
                description:
                    "Apply a unified diff patch to files. Supports fuzzy matching for AI-generated diffs."
                        .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "diff": {
                            "type": "string",
                            "description": "Unified diff string, markdown-embedded diff, or conflict markers."
                        },
                        "base_dir": {
                            "type": "string",
                            "description": "Absolute path to the workspace root."
                        }
                    },
                    "required": ["diff", "base_dir"]
                }),
            },
            Box::new(PatchTool),
        )
        .expect("apply_patch already registered");
}
