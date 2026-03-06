# Tool FS Hooks

File system operations covering read and write events.

## `before_file_read`

- **Safety tier:** Guarded
- **Hook category:** tool_fs
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool-fs/on-before-read`

**Payload schema:**

```json
{
  "path": "string"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `after_file_read`

- **Safety tier:** Safe
- **Hook category:** tool_fs
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool-fs/on-after-read`

**Payload schema:**

```json
{
  "path": "string",
  "size_bytes": "u64"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial

## `before_file_write`

- **Safety tier:** Guarded
- **Hook category:** tool_fs
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool-fs/on-before-write`

**Payload schema:**

```json
{
  "path": "string"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `after_file_write`

- **Safety tier:** Safe
- **Hook category:** tool_fs
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool-fs/on-after-write`

**Payload schema:**

```json
{
  "path": "string",
  "size_bytes": "u64"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
