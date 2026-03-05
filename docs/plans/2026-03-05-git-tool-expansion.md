# Git Tool Suite Expansion — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Expand git tooling from 2 tools (status, diff) to 17 tools covering the full local git workflow — staging, commits, branches, history, stash, merge, rebase, cherry-pick.

**Architecture:** Restructure `git_tool.rs` into a `git/` module directory with logical groupings. All tools use `gix` (pure Rust, no git CLI). Sync gix calls wrapped in `spawn_blocking`. Conflict-producing operations return conflict info rather than erroring.

**Tech Stack:** `gix 0.80` (status, blob-diff, index, worktree-mutation), `tokio` (spawn_blocking), `serde_json` (tool I/O)

---

## Task 0: Module restructure — move git_tool.rs into git/ directory

**Files:**
- Delete: `crates/ucode-tools/src/git_tool.rs`
- Create: `crates/ucode-tools/src/git/mod.rs`
- Create: `crates/ucode-tools/src/git/status.rs`
- Create: `crates/ucode-tools/src/git/diff.rs`
- Modify: `crates/ucode-tools/src/lib.rs` — change `pub mod git_tool` to `pub mod git`, update re-exports

**Step 1:** Create `git/mod.rs` with shared helpers (`repo_path`, `open_repo`) and `pub mod status; pub mod diff;`

**Step 2:** Move `GitStatusTool`, `git_status_impl`, `tree_index_change_info`, `register_git_status_tool` into `git/status.rs`

**Step 3:** Move `GitDiffTool`, `git_diff_impl`, `diff_blobs`, `register_git_diff_tool` into `git/diff.rs`

**Step 4:** Update `lib.rs`: replace `pub mod git_tool` with `pub mod git`, update all re-exports

**Step 5:** Update test imports in `crates/ucode-tools/tests/git_tests.rs` if needed

**Step 6:** Run `cargo test -p ucode-tools` — all 12 existing git tests must pass

**Step 7:** Run `cargo clippy --workspace -- -D warnings` — 0 warnings

**Step 8:** Commit: `refactor(tools): restructure git_tool.rs into git/ module directory`

---

## Task 1: git_add — stage files (git/staging.rs)

**Files:**
- Create: `crates/ucode-tools/src/git/staging.rs`
- Modify: `crates/ucode-tools/src/git/mod.rs` — add `pub mod staging;`
- Test: `crates/ucode-tools/tests/git_tests.rs`

**Input:** `{ path?: string, files: string[] }`
**Output:** `{ added: string[] }`

**Implementation:**
- Open repo via `open_repo(path)`
- Get mutable index via `repo.index_or_empty()` then modify
- For each file in `files`: add entry to index from worktree
- Write index back
- Return list of added files

**Tests (~4):**
- `add_single_file` — create untracked file, add it, verify status shows staged
- `add_multiple_files` — add several files at once
- `add_missing_file` — error when file doesn't exist
- `add_missing_files_arg` — error when `files` arg missing

---

## Task 2: git_commit — create commit (git/commit.rs)

**Files:**
- Create: `crates/ucode-tools/src/git/commit.rs`
- Modify: `crates/ucode-tools/src/git/mod.rs` — add `pub mod commit;`
- Test: `crates/ucode-tools/tests/git_tests.rs`

**Input:** `{ path?: string, message: string, author?: string }`
**Output:** `{ hash: string, message: string }`

**Implementation:**
- Open repo, get index, write tree from index
- Create commit object with tree, parent (HEAD if exists), message, author/committer
- Update HEAD ref to new commit
- Return hash + message

**Tests (~4):**
- `commit_staged_changes` — add file, commit, verify hash returned and log shows it
- `commit_with_message` — verify message preserved
- `commit_empty_index_error` — nothing staged → error
- `commit_missing_message_error` — no message arg → error

---

## Task 3: git_log — walk commit history (git/commit.rs)

**Input:** `{ path?: string, max_count?: number (default 10), rev?: string (default "HEAD") }`
**Output:** `{ commits: [{ hash, author, date, message }] }`

**Implementation:**
- Open repo, resolve rev to commit
- Walk ancestors up to max_count
- Collect hash, author name+email, timestamp, message

**Tests (~3):**
- `log_shows_commits` — create 3 commits, verify log returns them in order
- `log_max_count` — limit to 2, verify only 2 returned
- `log_empty_repo` — no commits → empty list or error

---

## Task 4: git_show — view commit content (git/commit.rs)

**Input:** `{ path?: string, commit: string }`
**Output:** `{ hash, author, date, message, diff }`

**Implementation:**
- Open repo, resolve commit ref to commit object
- Read commit metadata
- Diff commit's tree against parent's tree (or empty tree for root commit)
- Return metadata + unified diff

**Tests (~3):**
- `show_commit` — create commit, show it, verify metadata and diff
- `show_root_commit` — first commit (no parent), verify diff shows all additions
- `show_invalid_ref` — bad ref → error

---

## Task 5: git_tag — create/list tags (git/commit.rs)

**Input:** `{ path?: string, name?: string, delete?: bool, list?: bool (default true), commit?: string (default "HEAD") }`
**Output:** `{ tags: [...] }` or `{ created: "v1.0" }` or `{ deleted: "v1.0" }`

**Implementation:**
- List: enumerate refs under `refs/tags/`
- Create: create lightweight tag ref pointing to commit
- Delete: remove tag ref

**Tests (~4):**
- `tag_list_empty` — no tags → empty list
- `tag_create` — create tag, verify it appears in list
- `tag_delete` — create then delete, verify gone
- `tag_missing_name_error` — create without name → error

---

## Task 6: git_diff_staged — HEAD vs index diff (git/diff.rs)

**Input:** `{ path?: string, file?: string }`
**Output:** `{ diff: "..." }` or `{ diff: "", message: "no staged changes" }`

**Implementation:**
- Open repo, get HEAD tree and index
- For each index entry, compare against HEAD tree blob
- Generate unified diff for differences
- Reuse existing `diff_blobs` helper

**Tests (~3):**
- `diff_staged_shows_changes` — stage a modification, verify diff output
- `diff_staged_no_changes` — clean index → empty diff
- `diff_staged_specific_file` — filter to one file

---

## Task 7: git_diff_commits — diff between two refs (git/diff.rs)

**Input:** `{ path?: string, from: string, to: string, file?: string }`
**Output:** `{ diff: "..." }`

**Implementation:**
- Open repo, resolve both refs to commits, get their trees
- Walk both trees, diff blobs for changed entries
- Generate unified diff

**Tests (~3):**
- `diff_commits_shows_changes` — two commits with different content
- `diff_commits_same` — same commit → empty diff
- `diff_commits_invalid_ref` — bad ref → error

---

## Task 8: git_branch — create/list/delete branches (git/branch.rs)

**Files:**
- Create: `crates/ucode-tools/src/git/branch.rs`
- Modify: `crates/ucode-tools/src/git/mod.rs` — add `pub mod branch;`
- Test: `crates/ucode-tools/tests/git_tests.rs`

**Input:** `{ path?: string, name?: string, delete?: bool, list?: bool (default true), start_point?: string }`
**Output:** `{ branches: [...], current: "main" }` or `{ created: "feat-x" }` or `{ deleted: "feat-x" }`

**Implementation:**
- List: enumerate refs under `refs/heads/`, identify HEAD
- Create: create ref `refs/heads/<name>` pointing to start_point or HEAD
- Delete: remove ref (refuse to delete current branch)

**Tests (~5):**
- `branch_list` — shows current branch
- `branch_create` — create branch, verify in list
- `branch_create_from_commit` — create from specific commit
- `branch_delete` — delete non-current branch
- `branch_delete_current_error` — refuse to delete current branch

---

## Task 9: git_checkout — switch branches or restore files (git/branch.rs)

**Input:** `{ path?: string, branch?: string, create?: bool (default false), files?: string[] }`
**Output:** `{ switched_to: "feat-x" }` or `{ restored: ["a.rs"] }`

**Implementation:**
- If `branch` provided: update HEAD to point to branch ref, update worktree
- If `create` true: create branch first, then switch
- If `files` provided: restore specific files from HEAD to worktree (no branch switch)

**Tests (~4):**
- `checkout_branch` — switch to existing branch
- `checkout_create_branch` — create and switch
- `checkout_nonexistent_error` — branch doesn't exist → error
- `checkout_restore_files` — restore specific files from HEAD

---

## Task 10: git_reset — unstage or reset (git/staging.rs)

**Input:** `{ path?: string, files?: string[], mode?: "soft"|"mixed"|"hard" (default "mixed"), commit?: string (default "HEAD") }`
**Output:** `{ reset_to: "abc123", unstaged?: [...] }`

**Implementation:**
- If `files` provided: unstage specific files (reset index entries to HEAD tree)
- If no files: reset HEAD ref to commit, update index (mixed) or index+worktree (hard)
- Soft: only move HEAD, don't touch index or worktree

**Tests (~4):**
- `reset_unstage_file` — stage file, reset it, verify unstaged
- `reset_mixed` — reset to previous commit, verify index matches
- `reset_soft` — move HEAD but index unchanged
- `reset_hard` — move HEAD, index, and worktree all match

---

## Task 11: git_restore — discard working tree changes (git/staging.rs)

**Input:** `{ path?: string, files: string[], staged?: bool (default false), source?: string (default "HEAD") }`
**Output:** `{ restored: ["a.rs", "b.rs"] }`

**Implementation:**
- If `staged` false: restore worktree files from index (discard unstaged changes)
- If `staged` true: restore index entries from source commit (unstage)
- Read blob from source, write to worktree or index

**Tests (~3):**
- `restore_worktree` — modify file, restore, verify original content
- `restore_staged` — stage file, restore --staged, verify unstaged
- `restore_missing_files_error` — no files arg → error

---

## Task 12: git_stash — save/restore WIP (git/stash.rs)

**Files:**
- Create: `crates/ucode-tools/src/git/stash.rs`
- Modify: `crates/ucode-tools/src/git/mod.rs` — add `pub mod stash;`
- Test: `crates/ucode-tools/tests/git_tests.rs`

**Input:** `{ path?: string, action: "push"|"pop"|"list"|"drop", message?: string, index?: number }`
**Output:** varies by action

**Implementation:**
- Push: create stash commit (index state + worktree state), update `refs/stash` reflog
- Pop: apply stash, drop it
- List: read stash reflog entries
- Drop: remove stash entry

**Tests (~4):**
- `stash_push_and_list` — modify file, stash, verify clean and stash listed
- `stash_pop` — stash then pop, verify changes restored
- `stash_drop` — stash then drop, verify stash gone
- `stash_empty_error` — pop with no stash → error

---

## Task 13: git_merge — three-way merge (git/merge.rs)

**Files:**
- Create: `crates/ucode-tools/src/git/merge.rs`
- Modify: `crates/ucode-tools/src/git/mod.rs` — add `pub mod merge;`
- Test: `crates/ucode-tools/tests/git_tests.rs`

**Input:** `{ path?: string, branch: string, message?: string }`
**Output:** `{ hash, conflicts: [] }` or `{ status: "conflict", conflicts: ["file.rs"] }`

**Implementation:**
- Find merge base between HEAD and branch
- Three-way merge of trees (base, ours, theirs)
- If clean: create merge commit with two parents
- If conflicts: write conflict markers to worktree, return conflict list

**Tests (~4):**
- `merge_fast_forward` — linear history, fast-forward merge
- `merge_clean` — diverged branches, no conflicts
- `merge_conflict` — both branches modify same lines, verify conflict markers
- `merge_invalid_branch_error` — branch doesn't exist → error

---

## Task 14: git_cherry_pick — apply single commit (git/merge.rs)

**Input:** `{ path?: string, commit: string }`
**Output:** `{ hash, conflicts: [] }` or `{ status: "conflict", conflicts: [...] }`

**Implementation:**
- Get commit and its parent
- Three-way merge: parent as base, HEAD as ours, commit as theirs
- If clean: create new commit with cherry-picked changes
- If conflicts: write markers, return conflict list

**Tests (~3):**
- `cherry_pick_clean` — pick a commit cleanly
- `cherry_pick_conflict` — pick conflicting commit
- `cherry_pick_invalid_ref_error` — bad ref → error

---

## Task 15: git_rebase — replay commits (git/merge.rs)

**Input:** `{ path?: string, onto: string, branch?: string, interactive?: bool, actions?: [{ action: "pick"|"squash"|"reword"|"drop", commit: string, message?: string }], continue?: bool, abort?: bool }`
**Output:** `{ status: "ok", rebased_commits: N }` or `{ status: "conflict", conflicts: [...], current_commit: "abc123" }`

**Implementation:**
- Non-interactive: collect commits from branch that aren't on onto, cherry-pick each
- Interactive: use `actions` list to determine what to do with each commit
  - pick: cherry-pick as-is
  - squash: cherry-pick but amend into previous
  - reword: cherry-pick with new message
  - drop: skip
- Continue: resume after conflict resolution
- Abort: restore original state

**Tests (~5):**
- `rebase_simple` — rebase branch onto updated main
- `rebase_conflict` — conflict during rebase
- `rebase_interactive_squash` — squash two commits
- `rebase_interactive_drop` — drop a commit
- `rebase_abort` — abort mid-rebase

---

## Task 16: Registration + lib.rs integration

**Files:**
- Modify: `crates/ucode-tools/src/git/mod.rs` — add `register_all_git_tools()` convenience function
- Modify: `crates/ucode-tools/src/lib.rs` — re-export all register functions

**Implementation:**
- Add `pub fn register_all_git_tools(registry: &mut ToolRegistry)` that calls all 17 register functions
- Update lib.rs re-exports

**Tests (~2):**
- `register_all_git_tools` — verify all 17 tools registered
- `registry_lookup_all` — verify each tool can be looked up by name

---

## Task 17: Update EPIC.md and PLANS.md

- Update EPIC.md ISSUE 0407 scope to reflect all 17 tools
- Update PLANS.md Task 4.5 with full tool list
- Final commit

---

## Execution order

Tasks 0 → 1 → 2 → 3 → 4 → 5 → 6 → 7 → 8 → 9 → 10 → 11 → 12 → 13 → 14 → 15 → 16 → 17

Commit after each task. Run `cargo test -p ucode-tools` + `cargo clippy --workspace -- -D warnings` before each commit.

**Expected final test count:** ~350+ (291 existing + ~60 new)
