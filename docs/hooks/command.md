# Command Hooks

Command invocation and execution events.

## `command_invoked`

- **Safety tier:** Safe
- **Hook category:** command
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-command/on-invoked`

**Payload schema:**

```json
{
  "command": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `palette_command_executed`

- **Safety tier:** Safe
- **Hook category:** command
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-command/on-palette-executed`

**Payload schema:**

```json
{
  "command": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
