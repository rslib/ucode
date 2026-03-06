# Message Hooks

Message flow events covering user input, assistant responses, and retries.

## `user_message_received`

- **Safety tier:** Safe
- **Hook category:** message
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-message/on-user-message`

**Payload schema:**

```json
{
  "message_len": "usize"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `assistant_response_started`

- **Safety tier:** Safe
- **Hook category:** message
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-message/on-response-started`

**Payload schema:**

```json
{
  "model": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `assistant_response_completed`

- **Safety tier:** Safe
- **Hook category:** message
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-message/on-response-completed`

**Payload schema:**

```json
{
  "model": "string",
  "tokens": "usize",
  "duration_ms": "u64"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `message_retry`

- **Safety tier:** Guarded
- **Hook category:** message
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-message/on-retry`

**Payload schema:**

```json
{
  "reason": "string",
  "attempt": "u32"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial
