# Skill Hooks

Skill activation and deactivation events.

## `skill_activated`

- **Safety tier:** Safe
- **Hook category:** skill
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-skill/on-activated`

**Payload schema:**

```json
{
  "skill_name": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `skill_deactivated`

- **Safety tier:** Safe
- **Hook category:** skill
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-skill/on-deactivated`

**Payload schema:**

```json
{
  "skill_name": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
