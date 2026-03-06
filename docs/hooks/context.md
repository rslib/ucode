# Context Hooks

Token context and memory management events.

## `context_overflow`

- **Safety tier:** Guarded
- **Hook category:** context
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-context/on-overflow`

**Payload schema:**

```json
{
  "current_tokens": "usize",
  "max_tokens": "usize"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `context_compaction`

- **Safety tier:** Guarded
- **Hook category:** context
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-context/on-compaction`

**Payload schema:**

```json
{
  "before_tokens": "usize",
  "after_tokens": "usize"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `context_distilled`

- **Safety tier:** Safe
- **Hook category:** context
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-context/on-distilled`

**Payload schema:**

```json
{
  "before_tokens": "usize",
  "after_tokens": "usize"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `token_usage_updated`

- **Safety tier:** Safe
- **Hook category:** context
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-context/on-usage-updated`

**Payload schema:**

```json
{
  "total_tokens": "usize",
  "max_tokens": "usize"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
