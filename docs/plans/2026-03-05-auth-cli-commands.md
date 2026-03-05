# Auth CLI Commands (ISSUE 0202) Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Implement `ucode auth` subcommands (status, set-key, logout, login stubs) wired to the existing CredentialStore.

**Architecture:** clap derive-based CLI with `auth` subcommand group. Commands delegate to `ucode-auth` CredentialStore trait. Login commands are stubs (actual OAuth/device flows are 0203-0205). Tests use InMemoryStore for deterministic verification. ProviderId gets FromStr/ValueEnum for clap parsing.

**Tech Stack:** clap 4 (derive), ucode-auth (CredentialStore, InMemoryStore, ProviderId, AuthMaterial), tokio, anyhow

---

### Task 1: Add FromStr + clap ValueEnum to ProviderId

**Files:**
- Modify: `crates/ucode-auth/src/credential.rs` (add FromStr impl + all_providers pub)
- Modify: `crates/ucode-auth/Cargo.toml` (add clap dep for ValueEnum)
- Test: `crates/ucode-auth/tests/credential_tests.rs` (add FromStr tests)

**Step 1: Add clap dependency to ucode-auth**

```bash
cargo add clap --features derive -p ucode-auth
```

**Step 2: Add FromStr, Display improvements, and clap::ValueEnum to ProviderId**

In `crates/ucode-auth/src/credential.rs`:

- Derive `clap::ValueEnum` on `ProviderId`
- Implement `std::str::FromStr` for ProviderId (accepts "openai", "anthropic", "ollama")
- Make `all_providers()` pub

**Step 3: Write tests for FromStr**

```rust
#[test]
fn provider_id_from_str_valid() {
    assert_eq!("openai".parse::<ProviderId>().unwrap(), ProviderId::OpenAi);
    assert_eq!("anthropic".parse::<ProviderId>().unwrap(), ProviderId::Anthropic);
    assert_eq!("ollama".parse::<ProviderId>().unwrap(), ProviderId::Ollama);
}

#[test]
fn provider_id_from_str_invalid() {
    assert!("unknown".parse::<ProviderId>().is_err());
}
```

**Step 4: Run tests**

```bash
cargo test -p ucode-auth
```

Expected: All pass (existing 14 + 2 new = 16).

---

### Task 2: Define CLI structure with clap derive

**Files:**
- Modify: `crates/ucode-cli/Cargo.toml` (add ucode-auth dep)
- Create: `crates/ucode-cli/src/cmd_auth.rs` (auth subcommand definitions)
- Modify: `crates/ucode-cli/src/main.rs` (wire up clap + auth subcommand)

**Step 1: Add ucode-auth dependency to ucode-cli**

Add `ucode-auth = { path = "../ucode-auth" }` to workspace deps and ucode-cli Cargo.toml.

**Step 2: Define clap structs in cmd_auth.rs**

```rust
use clap::Subcommand;
use ucode_auth::ProviderId;

#[derive(Debug, Subcommand)]
pub enum AuthCmd {
    /// Show credential status for all providers
    Status,
    /// Set an API key for a provider
    SetKey {
        /// Provider to set key for
        provider: ProviderId,
    },
    /// Remove stored credentials for a provider
    Logout {
        /// Provider to log out from
        provider: ProviderId,
    },
    /// Login to a provider (interactive)
    Login {
        /// Provider to log in to
        provider: ProviderId,
        /// Use device-code flow (OpenAI only)
        #[arg(long)]
        device: bool,
        /// Use subscription login (Anthropic only)
        #[arg(long)]
        subscription: bool,
    },
}
```

**Step 3: Wire up main.rs with top-level Cli struct**

```rust
use clap::{Parser, Subcommand};

mod cmd_auth;

#[derive(Parser)]
#[command(name = "ucode", version, about = "AI-driven code generation")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage authentication credentials
    Auth {
        #[command(subcommand)]
        cmd: cmd_auth::AuthCmd,
    },
}
```

**Step 4: Verify it compiles**

```bash
cargo build -p ucode-cli
```

---

### Task 3: Implement auth command handlers

**Files:**
- Create: `crates/ucode-cli/src/auth_handler.rs` (handler functions)
- Modify: `crates/ucode-cli/src/main.rs` (call handlers)

**Step 1: Implement handler functions**

Each handler takes a `&dyn CredentialStore` and returns `anyhow::Result<()>`:

- `handle_status(store)` - prints status for all providers
- `handle_set_key(store, provider)` - reads key from stdin, stores it
- `handle_logout(store, provider)` - deletes credential
- `handle_login(store, provider, device, subscription)` - prints stub message (not implemented yet)

**Step 2: Wire handlers into main.rs dispatch**

Match on `Commands::Auth { cmd }` and dispatch to handler functions with a KeyringStore instance.

**Step 3: Verify it compiles and runs**

```bash
cargo build -p ucode-cli
cargo run -p ucode-cli -- auth status
```

---

### Task 4: Add integration tests

**Files:**
- Create: `crates/ucode-cli/tests/auth_cli_tests.rs`

**Step 1: Write tests using InMemoryStore**

Test the handler functions directly with InMemoryStore:

- `test_status_empty` - no creds configured, all show "not configured"
- `test_set_key_and_status` - set key, verify status shows configured
- `test_logout` - set key, logout, verify removed
- `test_logout_not_found` - logout without creds returns error
- `test_login_stub` - login returns "not yet implemented" message

**Step 2: Run tests**

```bash
cargo test -p ucode-cli
```

Expected: All pass.

---

### Task 5: Full workspace verification + commit

**Step 1: Run full workspace checks**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

**Step 2: Commit**

```bash
git add -A
git commit -m "feat(cli): add auth subcommands (status, set-key, logout, login stub)

Clap-based CLI with auth subcommand group wired to CredentialStore.
ProviderId gains FromStr + clap::ValueEnum for CLI parsing.
Login commands are stubs pending OAuth/device-code flows (0203-0205).

Closes ISSUE 0202."
```

---

### Task 6: Mark docs as done

Update EPIC.md and PLANS.md with `[DONE]` tags for ISSUE 0202.
