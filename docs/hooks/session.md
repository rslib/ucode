# Session Hooks

Session lifecycle events covering start, end, and session metadata updates.

## `session_start`

- **Safety tier:** Safe
- **Hook category:** session
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-session/on-start`

**Payload schema:**

```json
{
  "session_id": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `session_end`

- **Safety tier:** Safe
- **Hook category:** session
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-session/on-end`

**Payload schema:**

```json
{
  "session_id": "string",
  "duration_secs": "f64"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `session_title_generated`

- **Safety tier:** Safe
- **Hook category:** session
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-session/on-title-generated`

**Payload schema:**

```json
{
  "session_id": "string",
  "title": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `session_title_updated`

- **Safety tier:** Safe
- **Hook category:** session
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-session/on-title-updated`

**Payload schema:**

```json
{
  "session_id": "string",
  "title": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `config_reloaded`

- **Safety tier:** Safe
- **Hook category:** session
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-session/on-config-reloaded`

**Payload schema:**

```json
{}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
