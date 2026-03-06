# Tool Patch Hooks

Patch application events for file modifications.

## `before_apply_patch`

- **Safety tier:** Guarded
- **Hook category:** tool_patch
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool-patch/on-before-apply`

**Payload schema:**

```json
{
  "file_path": "string",
  "patch_summary": "string"
}
```

**Response options:**

- `Ok` -- observed, no action
- `Modify { changes }` -- propose modifications (Guarded+ only)

**Version history:**

- 1.0.0 -- initial

## `after_apply_patch`

- **Safety tier:** Safe
- **Hook category:** tool_patch
- **Payload version:** 1.0.0
- **WIT interface:** `ucode:hooks-tool-patch/on-after-apply`

**Payload schema:**

```json
{
  "file_path": "string",
  "lines_changed": "usize"
}
```

**Response options:**

- `Ok` -- observed, no action

**Version history:**

- 1.0.0 -- initial
