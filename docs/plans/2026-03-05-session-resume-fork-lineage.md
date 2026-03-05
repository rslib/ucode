# Session Resume/Fork Lineage (ISSUE 0110) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add session resume and fork/branch workflows so users can restore previous sessions and create child sessions that preserve transcript lineage.

**Architecture:** Extend `SessionMeta` with optional `parent_session_id` and `fork_source_index` fields (backward-compatible via `#[serde(default)]`). Add `Session::fork()` to create a child session from a parent at a given transcript index. Add CLI commands `session fork` and `session resume`. Display lineage in `session list` and `session show`.

**Tech Stack:** Rust, serde, clap, chrono, tempfile (tests)

---

### Task 1: Add lineage fields to SessionMeta

**Files:**
- Modify: `crates/ucode-core/src/session.rs` (SessionMeta struct, ~line 24-44)

**Changes:**
Add two new fields to `SessionMeta` after `archived`:

```rust
/// Parent session ID (set when this session was forked from another).
#[serde(default)]
pub parent_session_id: Option<SessionId>,

/// Transcript index in the parent where this fork occurred.
#[serde(default)]
pub fork_source_index: Option<usize>,
```

**Verify:** `cargo test --workspace` — all 149 existing tests still pass (backward compat via `#[serde(default)]`).

---

### Task 2: Add backward compat test for new lineage fields

**Files:**
- Modify: `crates/ucode-core/src/session.rs` (tests module, ~line 415)

**Changes:**
The existing `backward_compat_no_title_fields` test already validates old JSON without title/archived. It will also validate the new fields are `None` by default. Add explicit assertions:

```rust
assert!(session.meta.parent_session_id.is_none());
assert!(session.meta.fork_source_index.is_none());
```

**Verify:** `cargo test --workspace`

---

### Task 3: Add Session::fork() method

**Files:**
- Modify: `crates/ucode-core/src/session.rs` (Session impl block)

**Changes:**
Add method to Session impl:

```rust
/// Fork this session at the given transcript index, creating a new child session.
/// The child gets a new ID, copies transcript up to `at_index` (exclusive),
/// inherits model/skill/working_dir, and records lineage metadata.
/// Returns the new child session (not yet persisted).
pub fn fork(&self, at_index: Option<usize>) -> Self {
    let idx = at_index.unwrap_or(self.transcript.len());
    let idx = idx.min(self.transcript.len());
    let now = Utc::now();
    Self {
        meta: SessionMeta {
            id: generate_session_id(),
            created_at: now,
            updated_at: now,
            active_model: self.meta.active_model.clone(),
            active_skill: self.meta.active_skill.clone(),
            working_dir: self.meta.working_dir.clone(),
            title: None,
            title_source: TitleSource::Auto,
            archived: false,
            parent_session_id: Some(self.meta.id.clone()),
            fork_source_index: Some(idx),
        },
        transcript: self.transcript[..idx].to_vec(),
        tool_audit: Vec::new(),
        compaction_log: Vec::new(),
    }
}
```

**Verify:** `cargo test --workspace`

---

### Task 4: Add fork/resume tests in session.rs unit tests

**Files:**
- Modify: `crates/ucode-core/src/session.rs` (tests module)

**Tests to add:**

```rust
#[test]
fn fork_creates_child_with_lineage() {
    let mut parent = Session::new(PathBuf::from("/project"));
    parent.set_active_model(Some("claude-3".into()));
    parent.set_active_skill(Some("tdd".into()));
    parent.push_message(Message::user("hello"));
    parent.push_message(Message::assistant("hi"));
    parent.push_message(Message::user("next"));

    let child = parent.fork(Some(2));

    assert_ne!(child.meta.id, parent.meta.id);
    assert_eq!(child.meta.parent_session_id.as_deref(), Some(parent.meta.id.as_str()));
    assert_eq!(child.meta.fork_source_index, Some(2));
    assert_eq!(child.transcript.len(), 2);
    assert_eq!(child.meta.active_model.as_deref(), Some("claude-3"));
    assert_eq!(child.meta.active_skill.as_deref(), Some("tdd"));
    assert_eq!(child.meta.working_dir, parent.meta.working_dir);
    assert!(child.tool_audit.is_empty());
    assert!(child.compaction_log.is_empty());
    assert!(child.meta.title.is_none());
}

#[test]
fn fork_none_index_copies_full_transcript() {
    let mut parent = Session::new(PathBuf::from("/tmp"));
    parent.push_message(Message::user("a"));
    parent.push_message(Message::assistant("b"));

    let child = parent.fork(None);
    assert_eq!(child.transcript.len(), 2);
    assert_eq!(child.meta.fork_source_index, Some(2));
}

#[test]
fn fork_index_clamped_to_transcript_len() {
    let mut parent = Session::new(PathBuf::from("/tmp"));
    parent.push_message(Message::user("a"));

    let child = parent.fork(Some(999));
    assert_eq!(child.transcript.len(), 1);
    assert_eq!(child.meta.fork_source_index, Some(1));
}

#[test]
fn fork_no_state_bleed() {
    let mut parent = Session::new(PathBuf::from("/tmp"));
    parent.push_message(Message::user("hello"));
    let mut child = parent.fork(None);

    // Mutate child — parent must not change
    child.push_message(Message::user("child msg"));
    child.set_active_model(Some("gpt-4".into()));

    assert_eq!(parent.transcript.len(), 1);
    assert_eq!(child.transcript.len(), 2);
    assert!(parent.meta.active_model.is_none());
}
```

**Verify:** `cargo test --workspace`

---

### Task 5: Add fork/resume integration tests in session_tests.rs

**Files:**
- Modify: `crates/ucode-core/tests/session_tests.rs`

**Tests to add:**

```rust
#[test]
fn fork_and_save_roundtrip() {
    let dir = tempfile::tempdir().expect("tempdir");
    let parent_path = dir.path().join("parent.json");
    let child_path = dir.path().join("child.json");

    let mut parent = Session::new(PathBuf::from("/project"));
    parent.push_message(Message::user("hello"));
    parent.push_message(Message::assistant("world"));
    parent.set_active_model(Some("claude-3".into()));
    parent.save(&parent_path).expect("save parent");

    let child = parent.fork(Some(1));
    child.save(&child_path).expect("save child");

    let loaded_child = Session::load(&child_path).expect("load child");
    assert_eq!(loaded_child.meta.parent_session_id.as_deref(), Some(parent.meta.id.as_str()));
    assert_eq!(loaded_child.meta.fork_source_index, Some(1));
    assert_eq!(loaded_child.transcript.len(), 1);
    assert_eq!(loaded_child.meta.active_model.as_deref(), Some("claude-3"));
}
```

**Verify:** `cargo test --workspace`

---

### Task 6: Add SessionStore::fork() helper

**Files:**
- Modify: `crates/ucode-core/src/session.rs` (SessionStore impl block)

**Changes:**

```rust
/// Fork an existing session at the given transcript index.
/// Creates and persists the child session, returns it.
pub fn fork(&self, parent_id: &str, at_index: Option<usize>) -> Result<Session, CoreError> {
    let parent = self.load(parent_id)?;
    let child = parent.fork(at_index);
    self.save(&child)?;
    Ok(child)
}
```

**Add test:**

```rust
#[test]
fn session_store_fork() {
    let dir = tempfile::tempdir().unwrap();
    let store = SessionStore::new(dir.path().to_path_buf()).unwrap();
    let mut parent = store.create(PathBuf::from("/tmp")).unwrap();
    parent.push_message(Message::user("hello"));
    parent.push_message(Message::assistant("world"));
    store.save(&parent).unwrap();

    let child = store.fork(&parent.meta.id, Some(1)).unwrap();
    assert_eq!(child.meta.parent_session_id.as_deref(), Some(parent.meta.id.as_str()));
    assert_eq!(child.transcript.len(), 1);

    // Verify child is persisted
    let loaded = store.load(&child.meta.id).unwrap();
    assert_eq!(loaded.meta.parent_session_id.as_deref(), Some(parent.meta.id.as_str()));
}
```

**Verify:** `cargo test --workspace`

---

### Task 7: Add Fork, Resume, and Continue CLI commands

**Files:**
- Modify: `crates/ucode-cli/src/cmd_session.rs`

**Changes:**
Add three new variants to `SessionCommand`:

```rust
/// Fork a session, creating a child with shared transcript history.
Fork {
    /// Parent session ID to fork from.
    id: String,
    /// Fork at this transcript turn index (default: end of transcript).
    #[arg(long)]
    at_turn: Option<usize>,
},

/// Resume a session by ID (print its details for now).
Resume {
    /// Session ID to resume.
    id: String,
},

/// Continue the most recently updated non-archived session.
Continue,
```

---

### Task 8: Add fork/resume/continue CLI handlers

**Files:**
- Modify: `crates/ucode-cli/src/session_handler.rs`

**Changes:**

Update `handle_show` to display lineage info:
```rust
// After existing println! lines, add:
if let Some(ref parent) = m.parent_session_id {
    println!("Parent:  {}", parent);
}
if let Some(idx) = m.fork_source_index {
    println!("Fork at: turn {}", idx);
}
```

Update `handle_list` to show lineage indicator:
```rust
// Change the list format line to:
let lineage = if meta.parent_session_id.is_some() { " (fork)" } else { "" };
println!("{} -- {}{}{}", meta.id, title, archived, lineage);
```

Add new handlers:
```rust
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
    let most_recent = sessions.first().ok_or_else(|| {
        anyhow::anyhow!("No active sessions to continue.")
    })?;
    handle_resume(store, &most_recent.id)
}
```

---

### Task 9: Wire new commands in main.rs

**Files:**
- Modify: `crates/ucode-cli/src/main.rs`

**Changes:**
Add match arms for the new commands in the session dispatch:

```rust
SessionCommand::Fork { id, at_turn } => session_handler::handle_fork(&store, &id, at_turn),
SessionCommand::Resume { id } => session_handler::handle_resume(&store, &id),
SessionCommand::Continue => session_handler::handle_continue(&store),
```

---

### Task 10: Add CLI handler tests for fork/resume/continue

**Files:**
- Modify: `crates/ucode-cli/src/session_handler.rs` (tests module)

**Tests:**

```rust
#[test]
fn fork_session() {
    let (_dir, store) = test_store();
    let mut s = store.create(PathBuf::from("/tmp")).unwrap();
    s.push_message(ucode_core::Message::user("hello"));
    s.push_message(ucode_core::Message::assistant("world"));
    store.save(&s).unwrap();
    handle_fork(&store, &s.meta.id, Some(1)).unwrap();
    // Verify a new session exists
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
```

---

### Task 11: Update lib.rs re-exports (if needed)

**Files:**
- Check: `crates/ucode-core/src/lib.rs` line 28

No new public types to export (lineage fields are on existing `SessionMeta`). No changes needed unless we add new types.

---

### Task 12: Full workspace verification + commit

**Commands:**
```bash
cargo fmt --all
cargo clippy --workspace -- -D warnings
cargo test --workspace
```

Then commit with: `feat(session): add resume/fork lineage model (ISSUE 0110)`

Mark EPIC.md and PLANS.md with [DONE].
