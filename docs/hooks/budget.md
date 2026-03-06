# Budget Hooks

Cost tracking and budget threshold events.

## `budget_threshold_warning`

- **Safety tier:** Safe
- **Hook category:** budget
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-budget/on-warning`

**Payload schema:**

```json
{
  "current_cost": "f64",
  "threshold": "f64"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `budget_threshold_reached`

- **Safety tier:** Guarded
- **Hook category:** budget
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-budget/on-reached`

**Payload schema:**

```json
{
  "current_cost": "f64",
  "limit": "f64"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `cost_incurred`

- **Safety tier:** Safe
- **Hook category:** budget
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-budget/on-cost-incurred`

**Payload schema:**

```json
{
  "model": "string",
  "cost_usd": "f64",
  "tokens": "usize"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
