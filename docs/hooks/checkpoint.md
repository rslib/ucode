# Checkpoint Hooks

Session checkpoint creation and restoration events.

## `checkpoint_created`

- **Safety tier:** Guarded
- **Hook category:** checkpoint
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-checkpoint/on-created`

**Payload schema:**

```json
{
  "checkpoint_id": "string"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `checkpoint_restored`

- **Safety tier:** Risky
- **Hook category:** checkpoint
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-checkpoint/on-restored`

**Payload schema:**

```json
{
  "checkpoint_id": "string"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)
- `Veto { reason }` -- block the action (Risky only)

**Version history:**

- 1.0.0 -- initial
