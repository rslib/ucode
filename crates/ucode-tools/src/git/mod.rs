//! Git tools — pure-Rust git operations via `gix`.
//!
//! All tools use `gix` for git operations (no shelling out to git CLI).
//! Sync gix calls are wrapped in `tokio::task::spawn_blocking`.

pub mod commit;
pub mod diff;
pub mod staging;
pub mod status;

use std::path::{Path, PathBuf};

use serde_json::Value;
use ucode_core::CoreError;

// ── shared helpers ───────────────────────────────────────────────────────────

pub(crate) fn repo_path(args: &Value) -> PathBuf {
    args["path"]
        .as_str()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(crate) fn open_repo(path: &Path) -> Result<gix::Repository, CoreError> {
    gix::discover(path).map_err(|e| CoreError::Tool(format!("not a git repository: {e}")))
}

// ── re-exports ───────────────────────────────────────────────────────────────

pub use commit::{
    register_git_commit_tool, register_git_log_tool, register_git_show_tool, register_git_tag_tool,
};
pub use diff::{
    register_git_diff_commits_tool, register_git_diff_staged_tool, register_git_diff_tool,
};
pub use staging::register_git_add_tool;
pub use status::register_git_status_tool;
