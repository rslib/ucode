# End-to-End Scenario Test

This document describes the integrated "happy path" scenario test for ucode (ISSUE 0901).

## Overview

The end-to-end flow exercises the full pipeline:

```
User message -> Agent Loop -> Provider (LLM) -> Tool Calls -> Tool Results -> Follow-up -> TUI Display -> Session Save
```

## Happy Path Scenario

### Prerequisites

- An LLM provider API key (e.g., `ANTHROPIC_API_KEY` or `OPENAI_API_KEY`)
- OR a local Ollama instance running

### Steps

#### 1. Launch ucode

```bash
export ANTHROPIC_API_KEY=sk-ant-...
ucode
```

**Expected:** TUI launches. System message appears: "Using provider: anthropic (model: claude-sonnet-4-20250514)".

If no provider is configured, a warning is printed to stderr and the TUI launches without an agent (input is disabled).

#### 2. Send a prompt

Type a message in the input box and press Enter.

**Expected:**
- User message appears in the transcript
- Streaming tokens appear as the model responds
- The response is appended to the transcript

#### 3. Tool call execution

Send a prompt that triggers a tool call, e.g.:

```
What files are in the current directory?
```

**Expected:**
- Model requests a tool call (e.g., `ripgrep_search` or `list_files`)
- TUI shows "Tool call started: <tool_name>"
- Tool executes and result appears
- Model receives tool result and generates a follow-up response
- Follow-up response streams to the transcript

#### 4. Session persistence

After the conversation, the session is automatically saved.

**Expected:**
- Session file exists under `${UCODE_HOME}/sessions/`
- Session contains the full transcript (user messages, assistant responses, tool calls, tool results)

Verify with:
```bash
ucode session list
ucode session show <session-id>
```

#### 5. Provider error / fallback

If the provider returns an error (e.g., rate limit, auth expired):

**Expected:**
- Error toast appears in the TUI: "Agent Error: Provider error: ..."
- The agent loop continues -- user can send another message
- Session is preserved

## Headless Mode

The same flow works in headless (non-interactive) mode:

```bash
ucode run "What is 2+2?"
```

**Expected output (text mode):**
```
[system] Using provider: anthropic (model: claude-sonnet-4-20250514)
2 + 2 = 4.
```

**JSON output:**
```bash
ucode run --json-output "What is 2+2?"
```

Returns a JSON object with `events`, `usage`, `exit_code`, and `session_id`.

### Headless with timeout

```bash
ucode run --timeout 30 "Analyze this codebase"
```

If the operation exceeds 30 seconds, it terminates with a timeout message.

### Headless with session resume

```bash
ucode run --resume-session <id> "Continue from where we left off"
```

Loads the existing session transcript and appends the new prompt.

## Error Scenarios

### No provider configured

```bash
unset ANTHROPIC_API_KEY
unset OPENAI_API_KEY
ucode
```

**Expected:** Warning printed to stderr. TUI launches but without agent loop (messages are not sent to any provider).

### Invalid API key

```bash
export ANTHROPIC_API_KEY=invalid-key
ucode
```

**Expected:** TUI launches. When user sends a message, provider returns auth error. Error toast appears. User can fix credentials via `/connect` and retry.

### Malformed config file

```bash
echo "invalid toml [[[" > ~/.config/ucode/ucode.toml
ucode
```

**Expected:** Error message printed and ucode exits: "config error: failed to parse config file ...".

### Tool execution failure

When a tool call fails (e.g., file not found, permission denied):

**Expected:**
- Tool call shows as failed in the TUI
- Error result is sent back to the model
- Model can acknowledge the error and suggest alternatives
- Session records the failed tool use in the audit log

## Architecture

```
                    +------------------+
                    |   TUI / CLI      |
                    |  (event_loop.rs) |
                    +--------+---------+
                             |
                    message_tx (String)
                             |
                             v
                    +------------------+
                    |   Agent Loop     |
                    | (agent_loop.rs)  |
                    +--------+---------+
                             |
                    +--------+---------+
                    |                  |
              stream_chat()      invoke()
                    |                  |
                    v                  v
            +-------------+    +---------------+
            |  Provider   |    | ToolRegistry  |
            | (LLM API)   |    | (built-in +   |
            +-------------+    |  MCP tools)   |
                               +---------------+
                                       |
                               +-------+-------+
                               |               |
                          AgentEvent      Session
                          (back to TUI)   (save to disk)
```

## Running the Scenario

### Manual test (TUI mode)

```bash
# 1. Set up credentials
export ANTHROPIC_API_KEY=sk-ant-...

# 2. Launch
ucode

# 3. Type a message, observe streaming response
# 4. Type a message that triggers tools
# 5. Check session: ucode session list
```

### Manual test (headless mode)

```bash
# Simple prompt
ucode run "Hello, what can you do?"

# With JSON output
ucode run --json-output "List files in current directory"

# With session resume
ucode run "Follow up question" --resume-session <id>
```

### Automated test (future)

A full automated integration test would:
1. Start a mock LLM server that returns scripted responses
2. Launch ucode in headless mode pointing at the mock server
3. Verify the JSON output contains expected events
4. Verify the session file was created and contains the transcript

This is tracked for future implementation. The demo mode (`cargo run -p ucode-tui --example demo`) serves as a scripted integration test for the TUI display layer.
