use anyhow::Result;
use ucode_core::SessionStore;

pub fn handle_list(store: &SessionStore, all: bool) -> Result<()> {
    let sessions = store.list(all)?;
    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }
    for meta in &sessions {
        let title = meta.title.as_deref().unwrap_or("(untitled)");
        let archived = if meta.archived { " [archived]" } else { "" };
        let lineage = if meta.parent_session_id.is_some() {
            " (fork)"
        } else {
            ""
        };
        println!("{} -- {}{}{}", meta.id, title, archived, lineage);
    }
    Ok(())
}

pub fn handle_show(store: &SessionStore, id: &str) -> Result<()> {
    let session = store.load(id)?;
    let m = &session.meta;
    println!("Session: {}", m.id);
    println!("Title:   {}", m.title.as_deref().unwrap_or("(untitled)"));
    println!("Source:  {:?}", m.title_source);
    println!("Created: {}", m.created_at);
    println!("Updated: {}", m.updated_at);
    println!("Model:   {}", m.active_model.as_deref().unwrap_or("(none)"));
    println!("Skill:   {}", m.active_skill.as_deref().unwrap_or("(none)"));
    println!("Archived: {}", m.archived);
    println!("Messages: {}", session.transcript.len());
    println!("Tool calls: {}", session.tool_audit.len());
    println!("Compaction steps: {}", session.compaction_log.len());
    if let Some(ref parent) = m.parent_session_id {
        println!("Parent:  {}", parent);
    }
    if let Some(idx) = m.fork_source_index {
        println!("Fork at: turn {}", idx);
    }
    Ok(())
}

pub fn handle_rename(store: &SessionStore, id: &str, title: String) -> Result<()> {
    let mut session = store.load(id)?;
    session.rename(title);
    store.save(&session)?;
    println!(
        "Session {} renamed to: {}",
        id,
        session.meta.title.as_deref().unwrap_or("")
    );
    Ok(())
}

pub fn handle_archive(store: &SessionStore, id: &str) -> Result<()> {
    let mut session = store.load(id)?;
    session.archive();
    store.save(&session)?;
    println!("Session {} archived.", id);
    Ok(())
}

pub fn handle_unarchive(store: &SessionStore, id: &str) -> Result<()> {
    let mut session = store.load(id)?;
    session.unarchive();
    store.save(&session)?;
    println!("Session {} unarchived.", id);
    Ok(())
}

pub fn handle_fork(store: &SessionStore, id: &str, at_turn: Option<usize>) -> Result<()> {
    let child = store.fork(id, at_turn)?;
    println!(
        "Forked session {} -> {} (transcript: {} messages)",
        id,
        child.meta.id,
        child.transcript.len()
    );
    Ok(())
}

pub fn handle_resume(store: &SessionStore, id: &str) -> Result<()> {
    let session = store.load(id)?;
    let m = &session.meta;
    println!("Resuming session: {}", m.id);
    println!("Title:   {}", m.title.as_deref().unwrap_or("(untitled)"));
    println!("Model:   {}", m.active_model.as_deref().unwrap_or("(none)"));
    println!("Skill:   {}", m.active_skill.as_deref().unwrap_or("(none)"));
    println!("Messages: {}", session.transcript.len());
    if let Some(ref parent) = m.parent_session_id {
        println!("Parent:  {}", parent);
    }
    Ok(())
}

pub fn handle_continue(store: &SessionStore) -> Result<()> {
    let sessions = store.list(false)?;
    let most_recent = sessions
        .first()
        .ok_or_else(|| anyhow::anyhow!("No active sessions to continue."))?;
    handle_resume(store, &most_recent.id)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn test_store() -> (tempfile::TempDir, SessionStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore::new(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    #[test]
    fn list_empty() {
        let (_dir, store) = test_store();
        handle_list(&store, false).unwrap();
    }

    #[test]
    fn list_with_sessions() {
        let (_dir, store) = test_store();
        store.create(PathBuf::from("/tmp")).unwrap();
        store.create(PathBuf::from("/tmp")).unwrap();
        handle_list(&store, false).unwrap();
    }

    #[test]
    fn show_session() {
        let (_dir, store) = test_store();
        let s = store.create(PathBuf::from("/tmp")).unwrap();
        handle_show(&store, &s.meta.id).unwrap();
    }

    #[test]
    fn rename_session() {
        let (_dir, store) = test_store();
        let s = store.create(PathBuf::from("/tmp")).unwrap();
        handle_rename(&store, &s.meta.id, "New name".into()).unwrap();
        let loaded = store.load(&s.meta.id).unwrap();
        assert_eq!(loaded.meta.title.as_deref(), Some("New name"));
    }

    #[test]
    fn archive_and_unarchive() {
        let (_dir, store) = test_store();
        let s = store.create(PathBuf::from("/tmp")).unwrap();
        handle_archive(&store, &s.meta.id).unwrap();
        let loaded = store.load(&s.meta.id).unwrap();
        assert!(loaded.meta.archived);
        handle_unarchive(&store, &s.meta.id).unwrap();
        let loaded = store.load(&s.meta.id).unwrap();
        assert!(!loaded.meta.archived);
    }

    #[test]
    fn fork_session() {
        let (_dir, store) = test_store();
        let mut s = store.create(PathBuf::from("/tmp")).unwrap();
        s.push_message(ucode_core::Message::user("hello"));
        s.push_message(ucode_core::Message::assistant("world"));
        store.save(&s).unwrap();
        handle_fork(&store, &s.meta.id, Some(1)).unwrap();
        let all = store.list(false).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn resume_session() {
        let (_dir, store) = test_store();
        let s = store.create(PathBuf::from("/tmp")).unwrap();
        handle_resume(&store, &s.meta.id).unwrap();
    }

    #[test]
    fn fork_nonexistent_session_errors() {
        let (_dir, store) = test_store();
        let result = handle_fork(&store, "nonexistent", None);
        assert!(result.is_err());
    }

    #[test]
    fn continue_session() {
        let (_dir, store) = test_store();
        store.create(PathBuf::from("/tmp")).unwrap();
        handle_continue(&store).unwrap();
    }

    #[test]
    fn continue_no_sessions_errors() {
        let (_dir, store) = test_store();
        let result = handle_continue(&store);
        assert!(result.is_err());
    }
}
