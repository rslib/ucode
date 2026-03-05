//! ucode-tools: built-in tools, registry, permissions, sandbox policy engine

pub mod approval;
pub mod ast_tool;
pub mod cmd_tool;
pub mod fs_tools;
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
pub use patch_tool::register_patch_tool;
pub use policy::{
    Capabilities, EffectivePolicy, PolicyLayer, PolicyStack, SandboxTier,
    check_path_within_workspace,
};
pub use registry::{RegisteredTool, ToolHandler, ToolRegistry, ToolSpec};
pub use search_tool::register_search_tool;
