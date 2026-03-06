use std::fs;
use std::path::Path;

use tempfile::TempDir;
use ucode_skills::{Skill, SkillError, discover_skills, load_all_skills, parse_skill};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn write_skill(dir: &Path, subdir: &str, content: &str) {
    let skill_dir = dir.join(subdir);
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(skill_dir.join("SKILL.md"), content).unwrap();
}

fn minimal_skill_md() -> &'static str {
    "---\nname: minimal\ndescription: A minimal skill\n---\n"
}

fn full_skill_md() -> &'static str {
    "---\nname: full-skill\ndescription: A fully specified skill\nucode:\n  tool_allowlist:\n    - read_file\n    - search\n  routing_hints:\n    model_group: fast\n---\n\n# Instructions\n\nDo something useful here.\n"
}

// ---------------------------------------------------------------------------
// Parser tests
// ---------------------------------------------------------------------------

#[test]
fn parse_minimal_skill() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("SKILL.md");
    fs::write(&path, minimal_skill_md()).unwrap();

    let skill = parse_skill(&path).unwrap();
    assert_eq!(skill.name, "minimal");
    assert_eq!(skill.description, "A minimal skill");
    assert!(skill.instructions.is_empty());
    assert!(skill.ucode.is_none());
    assert_eq!(skill.source, path);
}

#[test]
fn parse_full_skill() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("SKILL.md");
    fs::write(&path, full_skill_md()).unwrap();

    let skill = parse_skill(&path).unwrap();
    assert_eq!(skill.name, "full-skill");
    assert_eq!(skill.description, "A fully specified skill");
    assert!(skill.instructions.contains("Do something useful here."));

    let ucode = skill.ucode.expect("ucode config should be present");
    assert_eq!(ucode.tool_allowlist, vec!["read_file", "search"]);
    assert_eq!(
        ucode.routing_hints.get("model_group").map(String::as_str),
        Some("fast")
    );
}

#[test]
fn parse_unknown_keys_ignored() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("SKILL.md");
    fs::write(
        &path,
        "---\nname: test\ndescription: desc\nfuture_key: whatever\nanother: 42\n---\n",
    )
    .unwrap();

    // Must not error on unknown keys.
    let skill = parse_skill(&path).unwrap();
    assert_eq!(skill.name, "test");
}

#[test]
fn parse_missing_name_fails() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("SKILL.md");
    fs::write(&path, "---\ndescription: no name here\n---\n").unwrap();

    let err = parse_skill(&path).unwrap_err();
    assert!(
        matches!(&err, SkillError::MissingField { field, .. } if field == "name"),
        "expected MissingField(name), got: {err}"
    );
}

#[test]
fn parse_missing_description_fails() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("SKILL.md");
    fs::write(&path, "---\nname: no-desc\n---\n").unwrap();

    let err = parse_skill(&path).unwrap_err();
    assert!(
        matches!(&err, SkillError::MissingField { field, .. } if field == "description"),
        "expected MissingField(description), got: {err}"
    );
}

#[test]
fn parse_no_frontmatter_fails() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("SKILL.md");
    fs::write(&path, "# Just a markdown file\n\nNo frontmatter at all.\n").unwrap();

    let err = parse_skill(&path).unwrap_err();
    assert!(
        matches!(err, SkillError::Parse { .. }),
        "expected Parse error, got: {err}"
    );
}

#[test]
fn parse_empty_body() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("SKILL.md");
    // Frontmatter only — no body text.
    fs::write(&path, "---\nname: empty-body\ndescription: no body\n---\n").unwrap();

    let skill = parse_skill(&path).unwrap();
    assert!(
        skill.instructions.is_empty(),
        "instructions should be empty"
    );
}

// ---------------------------------------------------------------------------
// Discovery tests
// ---------------------------------------------------------------------------

#[test]
fn discover_finds_skills_in_all_paths() {
    let project = TempDir::new().unwrap();
    let user_cfg = TempDir::new().unwrap();

    let content = minimal_skill_md();

    // Path 1: .claude/skills/
    write_skill(
        &project.path().join(".claude").join("skills"),
        "skill-a",
        content,
    );
    // Path 2: .agents/skills/
    write_skill(
        &project.path().join(".agents").join("skills"),
        "skill-b",
        content,
    );
    // Path 3: skills/
    write_skill(&project.path().join("skills"), "skill-c", content);
    // Path 4: user_config/skills/
    write_skill(&user_cfg.path().join("skills"), "skill-d", content);

    let found = discover_skills(project.path(), user_cfg.path());
    assert_eq!(found.len(), 4, "expected 4 skill files, got: {found:?}");

    // Verify each expected file is present.
    let names: Vec<_> = found
        .iter()
        .filter_map(|p| p.parent()?.file_name()?.to_str())
        .collect();
    for expected in ["skill-a", "skill-b", "skill-c", "skill-d"] {
        assert!(names.contains(&expected), "missing {expected} in {names:?}");
    }
}

#[test]
fn discover_empty_project() {
    let project = TempDir::new().unwrap();
    let user_cfg = TempDir::new().unwrap();

    let found = discover_skills(project.path(), user_cfg.path());
    assert!(found.is_empty(), "expected no skills, got: {found:?}");
}

#[test]
fn discover_partial_paths() {
    // Only one of the four paths has skills — others absent.
    let project = TempDir::new().unwrap();
    let user_cfg = TempDir::new().unwrap();

    write_skill(
        &project.path().join("skills"),
        "only-skill",
        minimal_skill_md(),
    );

    let found = discover_skills(project.path(), user_cfg.path());
    assert_eq!(found.len(), 1);
    assert!(found[0].ends_with("SKILL.md"));
}

// ---------------------------------------------------------------------------
// load_all_skills tests
// ---------------------------------------------------------------------------

#[test]
fn load_all_skills_mixed() {
    let project = TempDir::new().unwrap();
    let user_cfg = TempDir::new().unwrap();

    let skills_dir = project.path().join("skills");

    // Valid skill.
    write_skill(&skills_dir, "good-skill", full_skill_md());

    // Invalid skill — missing required fields.
    write_skill(&skills_dir, "bad-skill", "---\nsome_key: value\n---\n");

    let results = load_all_skills(project.path(), user_cfg.path());
    assert_eq!(results.len(), 2, "expected 2 results (1 ok + 1 err)");

    let ok_count = results.iter().filter(|r| r.is_ok()).count();
    let err_count = results.iter().filter(|r| r.is_err()).count();
    assert_eq!(ok_count, 1, "expected 1 successful parse");
    assert_eq!(err_count, 1, "expected 1 parse failure");

    // The successful one should be the full skill.
    let skill: &Skill = results.iter().find_map(|r| r.as_ref().ok()).unwrap();
    assert_eq!(skill.name, "full-skill");
}

#[test]
fn load_all_skills_empty() {
    let project = TempDir::new().unwrap();
    let user_cfg = TempDir::new().unwrap();

    let results = load_all_skills(project.path(), user_cfg.path());
    assert!(results.is_empty());
}
