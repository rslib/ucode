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
        println!("{} — {}{}", meta.id, title, archived);
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
}
