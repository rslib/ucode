# Approval Hooks

Access control and approval decision events.

## `approval_required`

- **Safety tier:** Guarded
- **Hook category:** approval
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-approval/on-required`

**Payload schema:**

```json
{
  "tool_name": "string",
  "risk_level": "string"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `approval_granted`

- **Safety tier:** Safe
- **Hook category:** approval
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-approval/on-granted`

**Payload schema:**

```json
{
  "tool_name": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `approval_denied`

- **Safety tier:** Safe
- **Hook category:** approval
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-approval/on-denied`

**Payload schema:**

```json
{
  "tool_name": "string",
  "reason": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `sandbox_decision`

- **Safety tier:** Safe
- **Hook category:** approval
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-approval/on-sandbox-decision`

**Payload schema:**

```json
{
  "tool_name": "string",
  "allowed": "bool",
  "reason": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `permission_decision`

- **Safety tier:** Safe
- **Hook category:** approval
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-approval/on-permission-decision`

**Payload schema:**

```json
{
  "action": "string",
  "allowed": "bool",
  "reason": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
