# Design: `/connect` UI (ISSUE 0705)

**Date:** 2026-03-07
**Status:** Approved
**Owner:** TUI/Auth

## Goal

In-TUI provider connect flow for API keys, browser OAuth login, and device
code login. Triggered by `/connect` slash command.

## Decisions

1. **Two-section provider list:** "Quick Connect" (providers with built-in login
   flows: Anthropic, OpenAI, GitHub Copilot, Google Gemini) at top, "API Key"
   section for the rest. Ollama excluded (no auth).
2. **Browser OAuth:** auto-open browser + show URL as fallback (handles SSH/headless).
3. **Auth only + verification ping:** no model selection (defer to `/models`).
   After auth succeeds, fire a lightweight API call to verify the credential works.
4. **Status badges + detail footer:** `[connected]`/`[expired]`/blank badges in
   the list. 2-3 line detail section at bottom shows auth method, expiry, env var
   status for the highlighted provider.

## Modal Layout

```
┌─────────── Connect ───────────┐
│ Filter: _                     │
│                               │
│ Quick Connect                 │
│ > Anthropic        [connected]│
│   OpenAI           [expired]  │
│   GitHub Copilot              │
│   Google Gemini               │
│                               │
│ API Key                       │
│   Groq                        │
│   DeepSeek         [connected]│
│   OpenRouter                  │
│   Together                    │
│   Fireworks                   │
│   Mistral                     │
│   Azure OpenAI                │
│   AWS Bedrock                 │
│   Google Vertex AI            │
│                               │
│───────────────────────────────│
│ Anthropic  [connected]        │
│ Method: OAuth  Expires: 23h   │
│ Env: ANTHROPIC_API_KEY (unset)│
└───────────────────────────────┘
```

## Interaction Flow

### Phase 1: Provider Selection

User types `/connect` or selects from command palette. Modal opens with
provider list. Arrow keys navigate, typing filters, Enter selects.

### Phase 2: Auth Method Picker (conditional)

If the selected provider supports multiple auth methods, show a submenu:

```
┌── Anthropic: Auth Method ──┐
│ > Browser login (Max)      │
│   Browser login (Console)  │
│   API key                  │
└────────────────────────────┘
```

Providers with a single method skip this step.

### Phase 3: Auth Flow

Depends on the method chosen:

**Browser OAuth:**
```
┌──── Authenticating... ─────┐
│                            │
│ Opening browser...         │
│                            │
│ If browser didn't open:    │
│ https://auth.openai.com/.. │
│                            │
│ Waiting for redirect...    │
│                            │
│              [Cancel: Esc] │
└────────────────────────────┘
```

Auto-opens browser via `open::that()`. Shows URL for manual copy. Runs
`browser_oauth_authorize()` in a spawned tokio task. Sends result back
via `TuiEvent` channel.

**Device Code (GitHub Copilot):**
```
┌──── GitHub Copilot ────────┐
│                            │
│ Open: github.com/login/    │
│       device               │
│ Code: XXXX-XXXX            │
│                            │
│ Waiting for authorization..│
│                            │
│              [Cancel: Esc] │
└────────────────────────────┘
```

Calls `request_device_code()`, displays code, then `poll_for_token()` in
a spawned task.

**API Key:**
```
┌──── Groq: API Key ────────┐
│                            │
│ Paste API key:             │
│ > sk-****_                 │
│                            │
│ Env: GROQ_API_KEY (unset)  │
│                            │
│    [Enter: Save] [Esc: ←]  │
└────────────────────────────┘
```

Inline text input. On Enter, store via `CredentialStore::store()`.

### Phase 4: Verification

After credential stored, spawn a lightweight API call to verify. Show
toast on completion:
- Success: `"Anthropic connected"` (Info level)
- Verification failed: `"Anthropic connected (verification failed)"` (Warn level)
- Auth failed: `"Anthropic auth failed: {error}"` (Error level)

## Architecture

### New Files

- `crates/ucode-tui/src/overlays/connect_modal.rs` — `ConnectModalState` + `ConnectModal` widget

### Modified Files

- `crates/ucode-tui/src/keybinds.rs` — add `Action::OpenConnect`
- `crates/ucode-tui/src/command_registry.rs` — wire `action: Some(Action::OpenConnect)` on `/connect`
- `crates/ucode-tui/src/app.rs` — add `connect_modal: ConnectModalState` field, handle `Action::OpenConnect`
- `crates/ucode-tui/src/event_loop.rs` — add `TuiEvent::AuthCompleted` / `TuiEvent::AuthFailed` variants, handle them; dispatch `Action::OpenConnect`; handle connect modal key events
- `crates/ucode-tui/src/overlays/mod.rs` — add `pub mod connect_modal;`
- `crates/ucode-tui/Cargo.toml` — add `ucode-auth` dependency

### State Machine

`ConnectModalState` has a phase enum:

```rust
enum ConnectPhase {
    ProviderList,           // Browsing providers
    MethodPicker,           // Choosing auth method (if multiple)
    BrowserOAuth {          // Waiting for browser OAuth
        url: String,
        cancel: CancellationToken,
    },
    DeviceCode {            // Waiting for device code auth
        user_code: String,
        verification_uri: String,
        cancel: CancellationToken,
    },
    ApiKeyEntry,            // Typing API key
    Verifying {             // Verification ping in progress
        provider: String,
    },
}
```

### Provider Info

```rust
struct ConnectProvider {
    id: &'static str,           // e.g. "anthropic"
    display_name: &'static str, // e.g. "Anthropic"
    section: ConnectSection,    // QuickConnect or ApiKey
    auth_methods: Vec<ConnectAuthMethod>,
    status: ProviderStatus,     // Connected { method, expires_at }, Expired, NotConfigured
    env_vars: &'static [&'static str],
}

enum ConnectSection { QuickConnect, ApiKey }

enum ConnectAuthMethod {
    BrowserOAuth { label: &'static str, config_fn: fn() -> BrowserOAuthConfig },
    DeviceCode { label: &'static str, config_fn: fn() -> DeviceCodeConfig },
    ApiKey,
}
```

Built from `provider_auth_info()` + `CredentialStore::status()` at modal open time.

### TuiEvent Extensions

```rust
// Add to TuiEvent enum:
AuthCompleted {
    provider: String,
    material: AuthMaterial,
},
AuthFailed {
    provider: String,
    error: String,
},
VerifyResult {
    provider: String,
    success: bool,
    message: Option<String>,
},
```

### Async Flow

1. User selects provider + method
2. `ConnectModalState` transitions to auth phase
3. Auth task spawned via `tokio::spawn` with cloned `event_tx: UnboundedSender<TuiEvent>`
4. Task runs auth flow (browser_oauth_authorize / poll_for_token)
5. On completion, sends `TuiEvent::AuthCompleted` or `TuiEvent::AuthFailed`
6. Event loop handler stores credential, transitions to `Verifying` phase
7. Verification task spawned, sends `TuiEvent::VerifyResult`
8. Handler shows toast, closes modal

### Cancellation

Browser OAuth and device code flows can be cancelled via Esc. Use
`tokio_util::sync::CancellationToken` — the spawned task checks
`token.is_cancelled()` in its polling loop.

### Verification Ping

Per-provider lightweight check:
- **OpenAI:** `GET /v1/models` with auth header
- **Anthropic:** `POST /v1/messages` with minimal payload (or `GET` if available)
- **GitHub Copilot:** token exchange endpoint
- **Others:** provider-specific model list or echo endpoint

If no verification endpoint is known, skip verification and show success toast.

## What This Does NOT Include

- Model selection (use `/models`)
- Provider configuration editing (base URLs, headers)
- Multi-account support
- Well-known endpoint discovery (can be added later)

## Testing Strategy

- Unit tests for `ConnectModalState` phase transitions
- Unit tests for provider list construction (sections, status badges)
- Unit tests for filter logic
- Integration test: `execute_command("connect", &[])` opens modal
- No mock auth flows in TUI tests — auth crate has its own test coverage
