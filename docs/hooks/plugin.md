# Plugin Hooks

Plugin lifecycle events.

## `plugin_loaded`

- **Safety tier:** Safe
- **Hook category:** plugin
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-plugin/on-loaded`

**Payload schema:**

```json
{
  "plugin_name": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `plugin_unloaded`

- **Safety tier:** Safe
- **Hook category:** plugin
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-plugin/on-unloaded`

**Payload schema:**

```json
{
  "plugin_name": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `plugin_error`

- **Safety tier:** Safe
- **Hook category:** plugin
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-plugin/on-error`

**Payload schema:**

```json
{
  "plugin_name": "string",
  "error": "string"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
