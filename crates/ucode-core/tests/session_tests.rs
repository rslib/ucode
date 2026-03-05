use std::path::PathBuf;

use ucode_core::{Message, Session};

fn wd() -> PathBuf {
    PathBuf::from("/tmp")
}

#[test]
fn new_session_has_id() {
    let s = Session::new(wd());
    assert!(!s.meta.id.is_empty());
    assert!(s.meta.id.starts_with("ses_"), "id was: {}", s.meta.id);
}

#[test]
fn push_message_appends() {
    let mut s = Session::new(wd());
    s.push_message(Message::user("first"));
    s.push_message(Message::assistant("second"));
    s.push_message(Message::user("third"));

    assert_eq!(s.transcript.len(), 3);

    // verify order by checking the text content
    let texts: Vec<&str> = s
        .transcript
        .iter()
        .map(|m| match &m.parts[0] {
            ucode_core::Part::Text(t) => t.as_str(),
            _ => panic!("expected text part"),
        })
        .collect();
    assert_eq!(texts, ["first", "second", "third"]);
}

#[test]
fn push_message_updates_timestamp() {
    let mut s = Session::new(wd());
    let before = s.meta.updated_at;
    // Sleep briefly so the clock can advance on fast machines.
    std::thread::sleep(std::time::Duration::from_millis(2));
    s.push_message(Message::user("hello"));
    assert!(s.meta.updated_at >= before);
}

#[test]
fn record_tool_use() {
    let mut s = Session::new(wd());
    s.record_tool_use("bash".into(), true, 42);
    s.record_tool_use("read_file".into(), false, 7);

    assert_eq!(s.tool_audit.len(), 2);
    assert_eq!(s.tool_audit[0].tool_name, "bash");
    assert!(s.tool_audit[0].approved);
    assert_eq!(s.tool_audit[0].duration_ms, 42);
    assert_eq!(s.tool_audit[1].tool_name, "read_file");
    assert!(!s.tool_audit[1].approved);
    assert_eq!(s.tool_audit[1].duration_ms, 7);
}

#[test]
fn set_active_model() {
    let mut s = Session::new(wd());
    assert!(s.meta.active_model.is_none());
    s.set_active_model(Some("gpt-4o".into()));
    assert_eq!(s.meta.active_model.as_deref(), Some("gpt-4o"));
    s.set_active_model(None);
    assert!(s.meta.active_model.is_none());
}

#[test]
fn set_active_skill() {
    let mut s = Session::new(wd());
    assert!(s.meta.active_skill.is_none());
    s.set_active_skill(Some("brainstorming".into()));
    assert_eq!(s.meta.active_skill.as_deref(), Some("brainstorming"));
    s.set_active_skill(None);
    assert!(s.meta.active_skill.is_none());
}

#[test]
fn save_and_load_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.json");

    let mut original = Session::new(PathBuf::from("/home/user/project"));
    original.push_message(Message::user("hello"));
    original.push_message(Message::assistant("world"));
    original.record_tool_use("bash".into(), true, 100);
    original.set_active_model(Some("claude-3".into()));
    original.set_active_skill(Some("tdd".into()));

    original.save(&path).expect("save");

    let loaded = Session::load(&path).expect("load");

    assert_eq!(loaded.meta.id, original.meta.id);
    assert_eq!(loaded.meta.active_model, original.meta.active_model);
    assert_eq!(loaded.meta.active_skill, original.meta.active_skill);
    assert_eq!(loaded.meta.working_dir, original.meta.working_dir);
    assert_eq!(loaded.transcript.len(), original.transcript.len());
    assert_eq!(loaded.tool_audit.len(), original.tool_audit.len());
    assert_eq!(loaded.tool_audit[0].tool_name, "bash");
    assert_eq!(loaded.tool_audit[0].duration_ms, 100);
}

#[test]
fn load_nonexistent_file_errors() {
    let result = Session::load(std::path::Path::new("/nonexistent/path/session.json"));
    assert!(result.is_err());
}
