# Model Hooks

Model selection, invocation, and fallback events covering the model lifecycle.

## `before_model_call`

- **Safety tier:** Guarded
- **Hook category:** model
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-model/on-before-call`

**Payload schema:**

```json
{
  "model": "string",
  "message_count": "usize"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `after_model_call`

- **Safety tier:** Safe
- **Hook category:** model
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-model/on-after-call`

**Payload schema:**

```json
{
  "model": "string",
  "tokens_used": "usize",
  "duration_ms": "u64"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `before_model_select`

- **Safety tier:** Guarded
- **Hook category:** model
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-model/on-before-select`

**Payload schema:**

```json
{
  "candidates": ["string"]
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `model_fallback`

- **Safety tier:** Risky
- **Hook category:** model
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-model/on-fallback`

**Payload schema:**

```json
{
  "from_model": "string",
  "to_model": "string",
  "reason": "string"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)
- `Veto { reason }` -- block the action (Risky only)

**Version history:**

- 1.0.0 -- initial

## `router_decision`

- **Safety tier:** Safe
- **Hook category:** model
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-model/on-router-decision`

**Payload schema:**

```json
{
  "model": "string",
  "reason": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `model_rate_limited`

- **Safety tier:** Safe
- **Hook category:** model
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-model/on-rate-limited`

**Payload schema:**

```json
{
  "model": "string",
  "retry_after_ms": "u64 | null"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `model_quota_exhausted`

- **Safety tier:** Safe
- **Hook category:** model
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-model/on-quota-exhausted`

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
