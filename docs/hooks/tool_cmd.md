# Tool CMD Hooks

Command execution events covering startup and completion.

## `before_run_cmd`

- **Safety tier:** Guarded
- **Hook category:** tool_cmd
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool-cmd/on-before-run`

**Payload schema:**

```json
{
  "command": "string"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `after_run_cmd`

- **Safety tier:** Safe
- **Hook category:** tool_cmd
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool-cmd/on-after-run`

**Payload schema:**

```json
{
  "command": "string",
  "exit_code": "i32",
  "duration_ms": "u64"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
