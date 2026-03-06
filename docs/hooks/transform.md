# Transform Hooks

Data transformation and message/prompt modification events.

## `transform_messages`

- **Safety tier:** Guarded
- **Hook category:** transform
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-transform/on-transform-messages`

**Payload schema:**

```json
{
  "messages_json": "string"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `transform_system_prompt`

- **Safety tier:** Guarded
- **Hook category:** transform
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-transform/on-transform-system-prompt`

**Payload schema:**

```json
{
  "prompt": "string"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial
