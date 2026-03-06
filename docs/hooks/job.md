# Job Hooks

Background job lifecycle events.

## `background_job_state_changed`

- **Safety tier:** Safe
- **Hook category:** job
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-job/on-state-changed`

**Payload schema:**

```json
{
  "job_id": "string",
  "state": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
