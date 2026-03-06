# Auth Hooks

Authentication and provider events.

## `auth_changed`

- **Safety tier:** Safe
- **Hook category:** auth
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-auth/on-changed`

**Payload schema:**

```json
{
  "provider": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `auth_failed`

- **Safety tier:** Safe
- **Hook category:** auth
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-auth/on-failed`

**Payload schema:**

```json
{
  "provider": "string",
  "error": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `provider_switched`

- **Safety tier:** Safe
- **Hook category:** auth
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-auth/on-provider-switched`

**Payload schema:**

```json
{
  "from": "string",
  "to": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
