# Tool Hooks

Tool invocation events covering execution, errors, and timeouts.

## `before_tool_call`

- **Safety tier:** Guarded
- **Hook category:** tool
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool/on-before-call`

**Payload schema:**

```json
{
  "tool_name": "string",
  "args": "object"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `after_tool_call`

- **Safety tier:** Safe
- **Hook category:** tool
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool/on-after-call`

**Payload schema:**

```json
{
  "tool_name": "string",
  "result": "object",
  "duration_ms": "u64"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `tool_error`

- **Safety tier:** Safe
- **Hook category:** tool
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool/on-error`

**Payload schema:**

```json
{
  "tool_name": "string",
  "error": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `tool_timeout`

- **Safety tier:** Safe
- **Hook category:** tool
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool/on-timeout`

**Payload schema:**

```json
{
  "tool_name": "string",
  "timeout_ms": "u64"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
