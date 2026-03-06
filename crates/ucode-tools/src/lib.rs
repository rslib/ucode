//! ucode-tools: built-in tools, registry, permissions, sandbox policy engine

pub mod approval;
pub mod ast_tool;
pub mod cmd_tool;
pub mod fs_tools;
pub mod git;
pub mod patch_tool;
pub mod policy;
pub mod registry;
pub mod search_tool;

pub use approval::{
    ApprovalAction, ApprovalDecision, ApprovalRecord, ApprovalScope, ApprovalStore, BoundaryGate,
};
pub use ast_tool::{register_ast_rewrite_tool, register_ast_search_tool};
pub use cmd_tool::register_cmd_tool;
pub use fs_tools::register_fs_tools;
pub use git::{
    register_all_git_tools, register_git_add_tool, register_git_branch_tool,
    register_git_checkout_tool, register_git_cherry_pick_tool, register_git_commit_tool,
    register_git_diff_commits_tool, register_git_diff_staged_tool, register_git_diff_tool,
    register_git_log_tool, register_git_merge_tool, register_git_rebase_tool,
    register_git_reset_tool, register_git_restore_tool, register_git_show_tool,
    register_git_stash_tool, register_git_status_tool, register_git_tag_tool,
};
pub use patch_tool::register_patch_tool;
pub use policy::{
    Capabilities, EffectivePolicy, NetworkCheckResult, NetworkPolicy, PolicyLayer, PolicyStack,
    SandboxTier, check_path_within_workspace,
};
pub use registry::{RegisteredTool, ToolHandler, ToolRegistry, ToolSpec};
pub use search_tool::register_search_tool;
