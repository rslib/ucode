# Agent Hooks

Agent spawning and lifecycle events.

## `agent_spawned`

- **Safety tier:** Safe
- **Hook category:** agent
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-agent/on-spawned`

**Payload schema:**

```json
{
  "agent_id": "string",
  "task": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `agent_message`

- **Safety tier:** Safe
- **Hook category:** agent
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-agent/on-message`

**Payload schema:**

```json
{
  "agent_id": "string",
  "message": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `agent_completed`

- **Safety tier:** Safe
- **Hook category:** agent
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-agent/on-completed`

**Payload schema:**

```json
{
  "agent_id": "string",
  "duration_ms": "u64"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `agent_failed`

- **Safety tier:** Safe
- **Hook category:** agent
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-agent/on-failed`

**Payload schema:**

```json
{
  "agent_id": "string",
  "error": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `agent_cancelled`

- **Safety tier:** Safe
- **Hook category:** agent
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-agent/on-cancelled`

**Payload schema:**

```json
{
  "agent_id": "string",
  "reason": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
