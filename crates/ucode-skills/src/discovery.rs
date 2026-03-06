use std::path::{Path, PathBuf};

/// Discover all `SKILL.md` files from the standard search paths.
///
/// Search order:
/// 1. `{project_root}/.claude/skills/*/SKILL.md`
/// 2. `{project_root}/.agents/skills/*/SKILL.md`
/// 3. `{project_root}/skills/*/SKILL.md`
/// 4. `{user_config}/skills/*/SKILL.md`
///
/// Each path is scanned by reading the `skills/` subdirectory and collecting
/// any `SKILL.md` file one level deep.  Non-existent directories are silently
/// skipped.
pub fn discover_skills(project_root: &Path, user_config: &Path) -> Vec<PathBuf> {
    let roots = [
        project_root.join(".claude").join("skills"),
        project_root.join(".agents").join("skills"),
        project_root.join("skills"),
        user_config.join("skills"),
    ];

    let mut found = Vec::new();
    for skills_dir in &roots {
        collect_skill_files(skills_dir, &mut found);
    }
    found
}

/// Scan `skills_dir` for `*/SKILL.md` entries (one level deep).
fn collect_skill_files(skills_dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(_) => return, // directory absent or unreadable — skip silently
    };

    for entry in entries.flatten() {
        let skill_file = entry.path().join("SKILL.md");
        if skill_file.is_file() {
            out.push(skill_file);
        }
    }
}
