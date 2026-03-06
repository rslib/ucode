# Diagnostic Hooks

Error and diagnostic events.

## `unhandled_error`

- **Safety tier:** Safe
- **Hook category:** diagnostic
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-diagnostic/on-unhandled-error`

**Payload schema:**

```json
{
  "error": "string",
  "context": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
